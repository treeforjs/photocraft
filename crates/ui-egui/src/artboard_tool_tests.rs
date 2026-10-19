use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use super::*;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150]})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Artboard;
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // Fonts set up after the first frame only apply from the next one.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    h.run_steps(4);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform { rect: app.last_canvas_rect, zoom: v.zoom, center: v.center, flip: false, rotation: v.rotation }.to_screen(x, y)
}

/// Drag in document coordinates, in a few steps, with `mods` held.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2], mods: Modifiers) {
    let a = screen(h, from[0], from[1]);
    let b = screen(h, to[0], to[1]);
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(a));
    h.step();
    h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: mods });
    h.step();
    for i in 1..=6 {
        h.event(Event::PointerMoved(a + (b - a) * (i as f32 / 6.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: mods });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn boards(h: &Harness<'static, PhotocraftApp>) -> Vec<(LayerId, [i32; 4])> {
    let doc = &h.state().session.active().unwrap().doc;
    doc.artboards().into_iter().map(|b| (b.0, [b.2.rect.x0, b.2.rect.y0, b.2.rect.width() as i32, b.2.rect.height() as i32])).collect()
}

#[test]
fn the_artboard_tool_draws_moves_copies_resizes_and_adds_boards() {
    let mut h = harness();
    let first = boards(&h)[0].0;
    // A drag on the pasteboard draws a new board, which becomes the selection.
    drag(&mut h, [300.0, 200.0], [400.0, 300.0], Modifiers::NONE);
    let b = boards(&h);
    assert_eq!(b.len(), 2, "{b:?}");
    let drawn = b.iter().find(|x| x.0 != first).unwrap();
    assert!(drawn.1.iter().zip([300, 200, 100, 100]).all(|(a, e)| (a - e).abs() <= 1), "{drawn:?}");
    assert_eq!(selected(h.state()).map(|s| s.0), Some(drawn.0));
    // Dragging the first board selects it and moves it with its contents.
    drag(&mut h, [100.0, 75.0], [120.0, 85.0], Modifiers::NONE);
    assert_eq!(selected(h.state()).map(|s| s.0), Some(first));
    let r = boards(&h).into_iter().find(|x| x.0 == first).unwrap().1;
    assert!((r[0] - 20).abs() <= 1 && (r[1] - 10).abs() <= 1 && r[2] == 200 && r[3] == 150, "{r:?}");
    // The bottom-right handle resizes it.
    let [x, y, w, hgt] = r;
    drag(&mut h, [(x + w) as f32, (y + hgt) as f32], [(x + w + 30) as f32, (y + hgt + 20) as f32], Modifiers::NONE);
    let r2 = boards(&h).into_iter().find(|x| x.0 == first).unwrap().1;
    assert!(r2[0] == x && r2[1] == y && (r2[2] - (w + 30)).abs() <= 1 && (r2[3] - (hgt + 20)).abs() <= 1, "{r:?} -> {r2:?}");
    // ⌥-drag drags out a copy and leaves the original.
    let n = boards(&h).len();
    drag(&mut h, [100.0, 60.0], [100.0, 360.0], Modifiers::ALT);
    let b = boards(&h);
    assert_eq!(b.len(), n + 1, "{b:?}");
    assert_eq!(b.iter().find(|x| x.0 == first).unwrap().1, r2, "the original stays");
    // The + right of the selected board adds one of its size beside it.
    let (sel, sr) = selected(h.state()).unwrap();
    let sides = plus_sides(&h.state().session.active().unwrap().doc, sr);
    assert!(sides.contains(&"right"), "{sides:?}");
    let plus = screen(&h, sr.x1 as f32, (sr.y0 + sr.y1) as f32 / 2.0) + vec2(PLUS_OFFSET, 0.0);
    let n = boards(&h).len();
    click(&mut h, plus);
    let b = boards(&h);
    assert_eq!(b.len(), n + 1, "{b:?}");
    let added = b.iter().find(|x| !x.0.0.eq(&sel.0) && x.1[2] == sr.width() as i32 && x.1[3] == sr.height() as i32 && x.1[0] > sr.x1).unwrap();
    assert_eq!(added.1[1], sr.y0, "{added:?}");
    // Each gesture is one undo step, and the tool never paints.
    assert!(h.state().session.active().unwrap().history.past_len() >= 5);
}

#[test]
fn resized_never_flips_or_overflows() {
    let r = photocraft_geom::Rect::new(10, 10, 50, 40);
    assert_eq!(resized(r, (1, 1), -100, -100), photocraft_geom::Rect::new(10, 10, 11, 11));
    assert_eq!(resized(r, (-1, -1), 100, 100), photocraft_geom::Rect::new(49, 39, 50, 40));
    assert_eq!(resized(r, (0, 1), 7, 3), photocraft_geom::Rect::new(10, 10, 50, 43));
    assert_eq!(resized(r, (-1, 0), -100, 0).x0, 0, "never left of the origin");
    let far = resized(photocraft_geom::Rect::new(0, 0, i32::MAX - 1, 5), (1, 0), i32::MAX, 0);
    assert_eq!(far.x1, i32::MAX);
}
