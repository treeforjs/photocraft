use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app
}

#[test]
fn add_buttons_save_to_the_library_and_render() {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(320.0, 420.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            panel(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    h.run_steps(2);
    h.get_by_label("+ Color").click();
    h.run_steps(2);
    // The new colour is named in place; Enter keeps the name.
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.get_by_label("+ Graphic").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let libs = &h.state().session.libraries.list;
    assert_eq!(libs.len(), 1);
    let kinds: Vec<&str> = libs[0].items.iter().map(|i| i.kind.key()).collect();
    assert_eq!(kinds, ["color", "graphic"]);
    // A graphic is selected, so the Place buttons show; Place adds a smart object layer.
    let layers = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Place").click();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn clicking_a_color_sets_the_foreground_and_dropping_a_graphic_places_it() {
    let mut app = app();
    app.run("tools.setColors", json!({"foreground": "#123456"})).unwrap();
    add(&mut app, "color");
    let color = app.session.libraries.list[0].items[0].id.to_string();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    handle(&mut app, Ev::Select(color.clone()), false);
    assert_eq!(c32([app.session.tools.foreground[0], app.session.tools.foreground[1], app.session.tools.foreground[2]]), Color32::from_rgb(0x12, 0x34, 0x56));
    handle(&mut app, Ev::Select(color.clone()), true);
    assert_eq!(app.session.tools.background[2], app.session.tools.foreground[2], "Alt-click sets the background");

    add(&mut app, "graphic");
    let graphic = app.session.libraries.list[0].items[1].id.to_string();
    let before = app.session.active().unwrap().doc.layers.len();
    handle(&mut app, Ev::Activate(graphic.clone()), false);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), before + 1, "double-click places");
    // Dropped outside the canvas (no view yet): placed at the centre.
    handle(&mut app, Ev::Drop(graphic.clone(), egui::pos2(-50.0, -50.0)), false);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), before + 2);

    handle(&mut app, Ev::Rename(graphic.clone(), "Logo".into()), false);
    assert_eq!(app.session.libraries.list[0].items[1].name, "Logo");
    handle(&mut app, Ev::NewGroup, false);
    let second = app.session.libraries.list[1].name.clone();
    handle(&mut app, Ev::Move(graphic.clone(), second), false);
    assert_eq!(app.session.libraries.list[1].items.len(), 1);
    handle(&mut app, Ev::Delete(graphic), false);
    assert!(app.session.libraries.list[1].items.is_empty());
    // Keys that aren't item ids are ignored.
    handle(&mut app, Ev::Select("not an id".into()), false);
    handle(&mut app, Ev::Delete("x".into()), false);
}

#[test]
fn window_menu_opens_the_libraries_tab() {
    let mut app = app();
    crate::preset_panels::menu(&mut app, "window.panel.libraries", &json!({"show": true})).unwrap().unwrap();
    assert!(app.ui.panels.color);
    assert_eq!(app.ui.dock_tabs.color, TAB);
    assert_eq!(crate::preset_panels::checked(&app, "window.panel.libraries"), Some(true));
}
