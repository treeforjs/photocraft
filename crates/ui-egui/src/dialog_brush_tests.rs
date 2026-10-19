use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;

use super::*;

#[test]
fn the_sampling_brush_paints_and_erases_the_fills_source_area() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "#33aa55"})).unwrap();
    app.run("select.rect", json!({"x": 400, "y": 150, "width": 40, "height": 40})).unwrap();
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
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.contentAwareFill", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!(fields[PARAM], "sampleBrush");
    assert!(armed(h.state()).is_none(), "the brush starts off");
    h.ctx.data_mut(|m| m.insert_temp(egui::Id::new(("dialog", id)), vec2(780.0, 600.0)));
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert(ARMED.into(), json!(true));
    d.fields.insert(SIZE.into(), json!(30.0));
    h.run_steps(3);
    let screen = |h: &Harness<'static, PhotocraftApp>, x: f32, y: f32| {
        let app = h.state();
        let v = &app.ui.views[0];
        ViewXform { rect: app.last_canvas_rect, zoom: v.zoom, center: v.center, flip: false, rotation: v.rotation }.to_screen(x, y)
    };
    let stroke = |h: &mut Harness<'static, PhotocraftApp>, mods: Modifiers| {
        let (a, b) = (screen(h, 40.0, 60.0), screen(h, 120.0, 60.0));
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(a));
        h.step();
        h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.step();
        for i in 1..=8 {
            h.event(Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
            h.step();
        }
        h.event(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: mods });
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
    };
    stroke(&mut h, Modifiers::NONE);
    stroke(&mut h, Modifiers::ALT);
    let dabs = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["sampleBrush"].as_array().unwrap().clone();
    assert!(dabs.iter().any(|d| d[3] == true) && dabs.iter().any(|d| d[3] == false), "{dabs:?}");
    assert!(dabs.iter().all(|d| (d[1].as_f64().unwrap() - 60.0).abs() < 1.5 && (d[2].as_f64().unwrap() - 15.0).abs() < 1e-6), "{dabs:?}");
    // OK hands the strokes to the command.
    let params = crate::filter_dialog::params_of(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    assert!(params.get(ARMED).is_none() && params["sampleBrush"].is_array());
    h.state_mut().run("edit.contentAwareFill", params).unwrap();
}

#[test]
fn scaling_and_painting_ignore_junk() {
    let mut p = json!({"edgeBrush": [[10, 20, 4, true], "x", [1]], "sampleBrush": [[8, 8, 8]], "radius": 3});
    scale(&mut p, 4);
    assert_eq!(p, json!({"edgeBrush": [[2.5, 5.0, 1.0, true], "x", [0.25]], "sampleBrush": [[2.0, 2.0, 2.0]], "radius": 3}));
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // No dialog, or a dialog without a brush: painting does nothing.
    paint(&mut app, [1.0, 1.0], false);
    let mut f = Map::new();
    f.insert(PARAM.into(), json!("notABrush"));
    f.insert(ARMED.into(), json!(true));
    app.ui.open_dialog(crate::state::DialogKind::Command, f);
    assert!(armed(&app).is_none(), "only known parameters take dabs");
    paint(&mut app, [1.0, 1.0], false);
    assert!(app.ui.dialogs[0].fields.get("notABrush").is_none());
}
