use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;

use super::*;

fn harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    h.run_steps(3);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    // Out of the way of the box: drag the dialog to the bottom-right corner.
    let place = egui::Id::new(("dialog", id));
    h.ctx.data_mut(|m| m.insert_temp(place, vec2(780.0, 600.0)));
    h.run_steps(3);
    (h, id)
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform { rect: app.last_canvas_rect, zoom: v.zoom, center: v.center, flip: false, rotation: v.rotation }.to_screen(x, y)
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2], mods: Modifiers) {
    let (a, b) = (screen(h, from[0], from[1]), screen(h, to[0], to[1]));
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
    h.run_steps(2);
}

fn current(h: &Harness<'static, PhotocraftApp>, id: u64) -> DocRect {
    box_of(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields).unwrap()
}

fn near(r: DocRect, e: [i32; 4]) -> bool {
    [r.x0, r.y0, r.width() as i32, r.height() as i32].iter().zip(e).all(|(a, e)| (a - e).abs() <= 1)
}

#[test]
fn the_box_resizes_moves_and_keeps_proportions_and_drives_the_command() {
    let (mut h, id) = harness();
    assert_eq!(current(&h, id), DocRect::new(100, 100, 300, 200), "the box starts on the selection");
    // The right-middle handle stretches only the width.
    drag(&mut h, [300.0, 150.0], [360.0, 150.0], Modifiers::NONE);
    let r = current(&h, id);
    assert!(near(r, [100, 100, 260, 100]), "{r:?}");
    // Inside the box moves it.
    drag(&mut h, [200.0, 150.0], [220.0, 180.0], Modifiers::NONE);
    let r = current(&h, id);
    assert!(near(r, [120, 130, 260, 100]), "{r:?}");
    // ⇧ on a corner keeps the proportions.
    let (x1, y1) = (r.x1 as f32, r.y1 as f32);
    drag(&mut h, [x1, y1], [x1 - 130.0, y1 + 40.0], Modifiers::SHIFT);
    let s = current(&h, id);
    assert!((s.width() as i32 - 130).abs() <= 1 && (s.height() as i32 - 50).abs() <= 1, "{s:?}");
    // OK runs the command with the box: the scaled pixels land in it.
    let params = crate::filter_dialog::params_of(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    let out = h.state_mut().run(COMMAND, params).unwrap();
    assert_eq!(out["bounds"], json!([s.x0, s.y0, s.width(), s.height()]));
}

#[test]
fn dragging_never_overflows_and_proxy_params_scale() {
    let d = Drag { handle: Some((1, 1)), start: [0.0, 0.0], rect: DocRect::new(0, 0, 10, 10) };
    for cur in [[f64::NAN, 1.0], [f64::INFINITY, -f64::INFINITY], [1e300, 1e300], [-1e300, 5.0]] {
        for shift in [false, true] {
            let r = dragged(&d, cur, shift);
            assert!(r.width() >= 1 && r.height() >= 1, "{cur:?} {r:?}");
            let m = dragged(&Drag { handle: None, ..d }, cur, shift);
            assert!(m.x0.abs() <= 60_000 && m.y0.abs() <= 60_000, "{m:?}");
        }
    }
    let mut p = json!({"x": 100, "y": 50, "width": 300, "height": 40});
    scale_params(&mut p, 4);
    assert_eq!(p, json!({"x": 25.0, "y": 12.5, "width": 75.0, "height": 40}));
}
