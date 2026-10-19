use serde_json::json;

use super::*;

/// A 40×10 mid-grey document with its left half selected.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 10, "background": "#808080"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    app
}

fn preview(app: &PhotocraftApp, view: &str, opacity: f64) -> photocraft_compose::Buffer {
    let st = app.session.active().unwrap();
    let mut f = Map::new();
    f.insert(VIEW.into(), json!(view));
    f.insert(OPACITY.into(), json!(opacity));
    let params = preview_params(&f, json!({"radius": 0, "output": "newDocument"}));
    let doc = crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &params, 1).unwrap();
    photocraft_compose::flatten(&doc)
}

fn px(b: &photocraft_compose::Buffer, x: usize) -> [f32; 4] {
    b.px[5 * 40 + x]
}

fn near(a: [f32; 4], e: [f32; 4]) -> bool {
    a.iter().zip(e).all(|(a, e)| (a - e).abs() < 0.02)
}

#[test]
fn every_view_draws_the_refined_selection_its_own_way() {
    let app = app();
    let grey = 128.0 / 255.0;
    let grey = [grey, grey, grey, 1.0];
    let cases: [(&str, f64, [f32; 4]); 7] = [
        ("marchingAnts", 50.0, grey),
        ("overlay", 100.0, [1.0, 0.0, 0.0, 1.0]),
        ("onBlack", 100.0, [0.0, 0.0, 0.0, 1.0]),
        ("onWhite", 100.0, [1.0, 1.0, 1.0, 1.0]),
        ("blackWhite", 50.0, [0.0, 0.0, 0.0, 1.0]),
        ("onLayers", 50.0, [0.0; 4]),
        ("onionSkin", 50.0, [grey[0], grey[1], grey[2], 0.5]),
    ];
    for (view, opacity, outside) in cases {
        let b = preview(&app, view, opacity);
        let inside = if view == "blackWhite" { [1.0; 4] } else { grey };
        assert!(near(px(&b, 5), inside), "{view} inside: {:?}", px(&b, 5));
        let out = px(&b, 35);
        // Premultiplied or not, a transparent pixel only needs its alpha checked.
        let ok = if outside[3] < 1.0 { (out[3] - outside[3]).abs() < 0.02 } else { near(out, outside) };
        assert!(ok, "{view} outside: {out:?}, want {outside:?}");
    }
    // The opacity slider fades the overlay.
    let half = px(&preview(&app, "overlay", 50.0), 35);
    assert!(half[0] > 0.7 && half[1] > 0.2 && half[1] < 0.35, "{half:?}");
}

#[test]
fn the_view_only_reaches_the_preview_never_the_command() {
    let mut app = app();
    let id = crate::menus::invoke(&mut app, &egui::Context::default(), "select.selectAndMask", json!({})).unwrap();
    let id = id["dialog"].as_u64().unwrap();
    let d = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    assert_eq!(d.fields["__label"], "Select and Mask");
    assert_eq!(d.fields[VIEW], "onLayers");
    assert_eq!(d.fields["__preview"], true, "Select and Mask previews live");
    d.fields.insert("output".into(), json!("newDocumentWithMask"));
    let params = crate::filter_dialog::params_of(&d.fields);
    assert!(params.get(VIEW).is_none() && params.get(OPACITY).is_none(), "{params}");
    let mut with_view = preview_params(&d.fields, params.clone());
    assert_eq!(take_view(&mut with_view), Some(("onLayers".to_string(), 0.5)));
    assert_eq!(with_view["output"], "selection", "the preview shows the mask");
    // The command itself outputs to a new document, leaving the source alone.
    app.run(COMMAND, params).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.session.documents()[0].doc.selection.is_some());
    // Unknown views are dropped instead of trusted.
    let mut bogus = json!({"__view": "nope", "radius": 1});
    assert_eq!(take_view(&mut bogus), None);
    assert!(bogus.get("__view").is_none());
}
