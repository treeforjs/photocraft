//! Clicking the Smart Filters mask swatch targets the filter mask; the brush then paints it.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn filter_mask(h: &Harness<'_, PhotocraftApp>, layer: u64) -> Option<photocraft_doc::LayerMask> {
    let st = h.state().session.active().unwrap();
    match &st.doc.layer(photocraft_doc::LayerId(layer)).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.filter_mask.clone(),
        _ => None,
    }
}

#[test]
fn clicking_the_filter_mask_swatch_adds_and_targets_the_mask() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Photo"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    let layer = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    assert!(filter_mask(&h, layer).is_none());
    assert_eq!(crate::canvas::paint_target(h.state()), json!("pixels"));
    let at = crate::smart_ui::filter_mask_swatch(&h.ctx, layer).expect("the Smart Filters row shows a mask swatch").center();
    click(&mut h, at);
    assert!(filter_mask(&h, layer).is_some(), "a white filter mask is added");
    assert!(h.state().ui.filter_mask_target);
    assert_eq!(crate::canvas::paint_target(h.state()), json!("filterMask"));
    assert_eq!(h.state().with_mask_target("image.adjustments.invert", json!({}))["target"], json!("filterMask"), "⌘I inverts the filter mask");
    // The brush paints the mask, not the smart object.
    let target = crate::canvas::paint_target(h.state());
    h.state_mut().run("paint.stroke", json!({"points": [[50, 50]], "size": 20, "hardness": 1.0, "color": "#000000", "target": target})).unwrap();
    assert!(filter_mask(&h, layer).unwrap().value(50, 50) < 0.01);
    // Targeting the layer mask (or the layer) leaves the filter mask.
    h.state_mut().ui.mask_target = true;
    h.run_steps(2);
    assert!(!h.state().ui.filter_mask_target);
}
