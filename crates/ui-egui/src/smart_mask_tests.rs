//! Smart Filter mask targeting through native controls and automation, including the full
//! live/cancel/commit path. Sources are synthetic embedded documents.

use egui::Modifiers;
use photocraft_doc::{LayerContent, LayerId, SmartObject, SmartSource};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 96, "height": 64, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#405080"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("image.adjustments.levels", json!({"gamma": 2.0})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
    app
}

fn smart(doc: &photocraft_doc::Document, layer: LayerId) -> &SmartObject {
    match &doc.layer(layer).unwrap().content {
        LayerContent::Smart(smart) => smart,
        _ => panic!("expected Smart Object"),
    }
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (request, _) = crate::control::ControlRequest::new(method, params);
    match crate::control::handle(app, &ctx, &request) {
        crate::control::Outcome::Done(result) => result,
        _ => panic!("expected immediate response"),
    }
}

fn target(app: &mut PhotocraftApp) {
    let result = call(app, "ui.set", json!({"smartFilterMaskTarget": true}));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(crate::canvas::paint_target(app), "smartFilterMask");
}

fn begin(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 12.0, y: 24.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 52.0, y: 24.0, pressure: 1.0 }, Modifiers::NONE);
}

#[test]
fn smart_mask_control_target_live_cancel_commit_and_one_undo() {
    let mut app = app();
    let before = app.session.active().unwrap().doc.clone();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    target(&mut app);
    assert!(!app.ui.mask_target && !app.ui.vector_mask_target);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "targeting is view state");
    begin(&mut app);
    assert!(app.live_stroke.is_some());
    let (preview, _) = crate::canvas::display_doc(&mut app, 0);
    assert!(smart(&preview, layer).filter_mask.is_some());
    assert_ne!(smart(&preview, layer).cache, smart(&before, layer).cache);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(crate::smart_mask_ui::cancel_stroke(&mut app));
    let (shown, _) = crate::canvas::display_doc(&mut app, 0);
    assert_eq!(shown, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    begin(&mut app);
    let (preview, _) = crate::canvas::display_doc(&mut app, 0);
    tool_event(&mut app, ToolEvent::Up { x: 52.0, y: 24.0 }, Modifiers::NONE);
    let committed = app.session.active().unwrap().doc.clone();
    assert_eq!(smart(&committed, layer).filter_mask, smart(&preview, layer).filter_mask);
    assert_eq!(smart(&committed, layer).cache, smart(&preview, layer).cache);
    assert_eq!(smart(&committed, layer).source, smart(&before, layer).source);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc, before);
    app.session.redo();
    assert_eq!(app.session.active().unwrap().doc, committed);
}

#[test]
fn smart_mask_target_switch_during_a_stroke_cancels_without_retargeting() {
    let mut app = app();
    target(&mut app);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    begin(&mut app);
    assert_eq!(call(&mut app, "ui.set", json!({"smartFilterMaskTarget": false}))["ok"], true);
    tool_event(&mut app, ToolEvent::Up { x: 52.0, y: 24.0 }, Modifiers::NONE);
    assert!(app.live_stroke.is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

fn deny_source(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    if command == "layer.smartFilter.setParams" && params.get("path").is_some() {
        return Err(photocraft_engine::EngineError::Other("smart mask source denied".into()));
    }
    Ok(())
}

#[test]
fn synthetic_pointer_mask_preview_cannot_read_an_unauthorized_source() {
    let mut app = app();
    app.session
        .edit("Linked fixture", |doc, active| {
            let LayerContent::Smart(smart) = &mut doc.layer_mut(active.unwrap()).unwrap().content else { panic!("smart") };
            smart.source = SmartSource::Linked { path: "/__photocraft_mask_denied__/source.pcraft".into() };
            Ok(())
        })
        .unwrap();
    app.services.automation_authorize = Some(deny_source);
    target(&mut app);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    app.automation_input = true;
    begin(&mut app);
    assert!(app.live_stroke.is_none());
    // The delayed release retains the captured gate after the control request restores it.
    app.automation_input = false;
    tool_event(&mut app, ToolEvent::Up { x: 52.0, y: 24.0 }, Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(app.ui.status.contains("smart mask source denied"), "{}", app.ui.status);
    assert!(app.session.authorize.is_none());
}

#[test]
fn selecting_smart_mask_validates_before_other_ui_fields_change() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let tool = app.ui.tool;
    let result = call(&mut app, "ui.set", json!({"smartFilterMaskTarget": true, "tool": "pencil"}));
    assert_eq!(result["ok"], false);
    assert_eq!(app.ui.tool, tool);
    assert!(app.ui.smart_filter_mask_target.is_none());
}

#[test]
fn native_mask_thumbnail_targets_paints_and_escape_cancels() {
    use egui_kittest::kittest::Queryable;
    let app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |creation| {
        PhotocraftApp::setup_context(&creation.egui_ctx, Default::default());
        app
    });
    harness.run_steps(4);
    harness.get_by_label("Smart Filter mask").click();
    harness.run_steps(2);
    assert_eq!(harness.state().ui.smart_filter_mask_target.unwrap().layer, layer);
    assert!(harness.state().thumbs.contains_key(&(layer, crate::smart_mask_ui::THUMB)));
    let before = harness.state().session.active().unwrap().doc.clone();
    begin(harness.state_mut());
    assert!(harness.state().live_stroke.is_some());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().live_stroke.is_none());
    assert!(harness.state().drag.is_none());
    assert_eq!(harness.state().session.active().unwrap().doc, before);
}
