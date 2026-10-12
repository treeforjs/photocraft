//! Smart Filter stack controls through the same control and native action paths as the UI.
//! All documents and filter stacks are synthetic.

use std::sync::Arc;

use photocraft_doc::{BlendMode, Document, LayerContent, LayerId, SmartObject, SmartSource};
use serde_json::{Value, json};

use crate::{PhotocraftApp, smart_filter_rows};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 96, "height": 64, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#405080"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("image.adjustments.levels", json!({"gamma": 2.0})).unwrap();
    app.run("image.adjustments.curves", json!({"points": [[0, 0], [128, 170], [255, 255]]})).unwrap();
    app.run("image.adjustments.hueSaturation", json!({"hue": 35, "saturation": 20})).unwrap();
    app
}

fn smart(doc: &Document, layer: LayerId) -> &SmartObject {
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
        _ => panic!("expected immediate control response"),
    }
}

fn open_blend(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> u64 {
    let result = call(app, "ui.dialog.open", json!({"kind": "smartFilterBlend", "layer": layer.0, "index": index}));
    assert_eq!(result["ok"], true, "{result}");
    result["result"]["dialog"].as_u64().unwrap()
}

fn set_blend(app: &mut PhotocraftApp, dialog: u64, opacity: f64, blend: &str) {
    for (field, value) in [("opacity", json!(opacity)), ("blend", json!(blend))] {
        let result = call(app, "ui.dialog.set", json!({"dialog": dialog, "field": field, "value": value}));
        assert_eq!(result["ok"], true, "{result}");
    }
}

fn preview_blend(app: &PhotocraftApp, dialog: u64) -> Document {
    let fields = &app.ui.dialogs.iter().find(|candidate| candidate.id == dialog).unwrap().fields;
    let command = fields["__command"].as_str().unwrap().to_string();
    let (command, params) = crate::smart_ui::confirm_command(fields, command, crate::filter_dialog::params_of(fields));
    assert_eq!(command, "layer.smartFilter.blendingOptions", "the editor changes blend controls, not filter parameters");
    assert!(params.get("params").is_none(), "blend controls must not become a nested filter-parameter patch");
    let state = app.session.active().unwrap();
    crate::filter_dialog::preview_document(&state.doc, state.active_layer, &command, &params, 1).unwrap()
}

#[test]
fn collapsing_a_stack_is_view_state_without_history_or_cache_changes() {
    let mut app = app();
    let state = app.session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let before = state.doc.clone();
    let history = state.history.past_len();
    let revision = state.revision;
    for collapsed in [true, false] {
        let result = call(&mut app, "ui.set", json!({"smartFiltersCollapsed": collapsed}));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(smart_filter_rows::collapsed(&app, layer), collapsed);
        let state = app.session.active().unwrap();
        assert!(Arc::ptr_eq(&before, &state.doc), "collapsing never replaces the document or Smart cache");
        assert_eq!(state.history.past_len(), history);
        assert_eq!(state.revision, revision);
        assert_eq!(smart(&state.doc, layer).cache, smart(&before, layer).cache);
    }
}

#[test]
fn invalid_collapse_on_a_raster_layer_rejects_the_whole_ui_patch() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let tool = app.ui.tool;
    let history = app.session.active().unwrap().history.past_len();
    let result = call(&mut app, "ui.set", json!({"smartFiltersCollapsed": true, "tool": "pencil"}));
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(app.ui.tool, tool, "validate the stack before applying unrelated UI fields");
    assert!(app.ui.smart_filter_stacks.is_empty());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn blending_preview_cancel_and_confirm_preserve_source_and_form_one_undo_step() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let revision = app.session.active().unwrap().revision;
    let dialog = open_blend(&mut app, layer, 0);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["opacity"], 1.0);
    set_blend(&mut app, dialog, 0.5, "Normal");
    let preview = preview_blend(&app, dialog);
    assert_ne!(smart(&preview, layer).cache, smart(&before, layer).cache, "opacity must affect the displayed image");
    assert_eq!(smart(&preview, layer).source, smart(&before, layer).source);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);

    let dialog = open_blend(&mut app, layer, 0);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["opacity"], 1.0, "cancel does not change the recorded blend controls");
    set_blend(&mut app, dialog, 0.5, "Normal");
    let preview = preview_blend(&app, dialog);
    let result = call(&mut app, "ui.dialog.confirm", json!({"dialog": dialog}));
    assert_eq!(result["ok"], true, "{result}");
    let after = app.session.active().unwrap().doc.clone();
    assert_eq!(*after, preview);
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    assert_eq!(smart(&after, layer).source, smart(&before, layer).source);
    assert_eq!((smart(&after, layer).smart_filters[0].opacity, smart(&after, layer).smart_filters[0].blend), (0.5, BlendMode::Normal));
    for (actual, expected) in smart(&after, layer).smart_filters.iter().zip(&smart(&before, layer).smart_filters) {
        assert_eq!((&actual.command, &actual.params, actual.visible), (&expected.command, &expected.params, expected.visible));
    }
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc, before);
    app.session.redo();
    assert_eq!(app.session.active().unwrap().doc, after);
    let dialog = open_blend(&mut app, layer, 0);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["opacity"], 0.5, "reopening loads the current filter blend controls");
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
}

#[test]
fn blending_dialog_refuses_stale_stack_indices_before_setting_or_confirming() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let dialog = open_blend(&mut app, layer, 0);
    set_blend(&mut app, dialog, 0.5, "Normal");
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    app.session.execute("layer.smartFilter.move", json!({"layer": layer.0, "index": 0, "to": 2})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let result = call(&mut app, "ui.dialog.set", json!({"dialog": dialog, "field": "opacity", "value": 0.25}));
    assert_eq!(result["ok"], false, "{result}");
    assert!(result["error"].as_str().unwrap().contains("changed"), "{result}");
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields, fields);
    let result = call(&mut app, "ui.dialog.confirm", json!({"dialog": dialog}));
    assert_eq!(result["ok"], false, "{result}");
    assert!(result["error"].as_str().unwrap().contains("changed"), "{result}");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn reduced_blending_preview_keeps_translated_placement_and_the_existing_filter_mask() {
    let mut app = app();
    app.run("edit.transform", json!({"matrix": [1.5, 0, 0, 1.5, 12, 6]})).unwrap();
    app.run("select.rect", json!({"x": 12, "y": 6, "width": 30, "height": 24})).unwrap();
    app.run("image.adjustments.levels", json!({"gamma": 0.85})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let state = app.session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let params = json!({"layer": layer.0, "index": 1, "opacity": 0.5, "blend": "Normal"});
    let command = smart_filter_rows::BLENDING;
    assert_eq!(crate::smart_adjust_preview::factor(&state.doc, state.active_layer, command, &params, 2), 2);
    let full = crate::filter_dialog::preview_document(&state.doc, state.active_layer, command, &params, 1).unwrap();
    let reduced = crate::filter_dialog::preview_document(&state.doc, state.active_layer, command, &params, 2).unwrap();
    assert_eq!((reduced.size.width, reduced.size.height), (48, 32));
    assert_eq!(smart(&reduced, layer).source, smart(&state.doc, layer).source);

    let mut proxy = crate::proxy::proxy_document(&state.doc, 2);
    crate::smart_adjust_preview::prepare(&mut proxy, state.active_layer, command, &params, 2);
    let mut expected = photocraft_engine::Session::new();
    expected.add_document(proxy, None);
    expected.select_layer(layer).unwrap();
    expected.execute(command, params).unwrap();
    assert_eq!(smart(&reduced, layer), smart(&expected.active().unwrap().doc, layer));

    // Constant-color regions away from mask and placement edges also have a direct
    // full-resolution pixel oracle, independent of the proxy preparation helpers.
    let full = photocraft_compose::flatten(&full);
    let reduced = photocraft_compose::flatten(&reduced);
    for (x, y) in [(0, 0), (18, 12), (30, 18), (54, 12), (84, 48)] {
        assert_eq!(reduced.px[(y / 2 * 48 + x / 2) as usize], full.px[(y * 96 + x) as usize], "blending proxy at {x},{y}");
    }
}

fn deny_source(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    if params.get("path").is_some() {
        assert_eq!(command, "layer.smartFilter.setParams");
        return Err(photocraft_engine::EngineError::Other("Smart Filter blending source denied".into()));
    }
    Ok(())
}

#[test]
fn blending_open_checks_source_authority_for_control_and_synthetic_native_actions() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("Synthetic denied linked source", |doc, _| {
            let LayerContent::Smart(sm) = &mut doc.layer_mut(layer).unwrap().content else { panic!("smart fixture") };
            sm.source = SmartSource::Linked { path: "/__photocraft_blending_denied__/source.pcraft".into() };
            Ok(())
        })
        .unwrap();
    app.services.automation_authorize = Some(deny_source);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let result = call(&mut app, "ui.dialog.open", json!({"kind": "smartFilterBlend", "layer": layer.0, "index": 0}));
    assert_eq!(result["ok"], false, "{result}");
    assert!(result["error"].as_str().unwrap().contains("Smart Filter blending source denied"), "authorize before resolving the missing path: {result}");
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.authorize.is_none());

    app.automation_input = true;
    let result = smart_filter_rows::action(&mut app, &egui::Context::default(), "ui.smartFilterBlend", &json!({"layer": layer.0, "index": 0})).unwrap();
    app.automation_input = false;
    assert!(result.unwrap_err().contains("Smart Filter blending source denied"));
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.authorize.is_none(), "the temporary authority is restored");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// Independent visual oracle: remove the named source row and insert it before/after the
/// named target row in top-to-bottom order. No engine-index insertion arithmetic is shared.
fn visual_order_after_drop(commands: &[String], source: usize, target: usize, above: bool) -> Vec<String> {
    let mut visual: Vec<String> = commands.iter().rev().cloned().collect();
    if source != target {
        let source_name = &commands[source];
        let target_name = &commands[target];
        let removed = visual.remove(visual.iter().position(|name| name == source_name).unwrap());
        let anchor = visual.iter().position(|name| name == target_name).unwrap();
        visual.insert(if above { anchor } else { anchor + 1 }, removed);
    }
    visual.into_iter().rev().collect()
}

#[test]
fn every_drag_source_target_and_side_matches_visual_order_with_noop_history_preserved() {
    for source in 0..3 {
        for target in 0..3 {
            for above in [false, true] {
                let mut app = app();
                let layer = app.session.active().unwrap().active_layer.unwrap();
                let before = app.session.active().unwrap().doc.clone();
                let history = app.session.active().unwrap().history.past_len();
                let commands: Vec<_> = smart(&before, layer).smart_filters.iter().map(|filter| filter.command.clone()).collect();
                let expected = visual_order_after_drop(&commands, source, target, above);
                smart_filter_rows::begin_drag(&mut app, layer, source).unwrap();
                let drag = app.ui.smart_filter_drag.clone().unwrap();
                let drop = smart_filter_rows::drop_command(&app, &drag, layer, target, above).unwrap();
                assert_eq!(drop.is_none(), expected == commands, "source={source}, target={target}, above={above}");
                smart_filter_rows::action(
                    &mut app,
                    &egui::Context::default(),
                    "ui.smartFilterMove",
                    &json!({"layer": layer.0, "index": target, "above": above, "drag": drag}),
                )
                .unwrap()
                .unwrap();
                assert!(app.ui.smart_filter_drag.is_none());
                let after = app.session.active().unwrap().doc.clone();
                let actual: Vec<_> = smart(&after, layer).smart_filters.iter().map(|filter| filter.command.clone()).collect();
                assert_eq!(actual, expected, "source={source}, target={target}, above={above}");
                assert_eq!(smart(&after, layer).source, smart(&before, layer).source);
                if expected == commands {
                    assert!(Arc::ptr_eq(&before, &after), "a no-op drop cannot rebuild the Smart cache");
                    assert_eq!(app.session.active().unwrap().history.past_len(), history);
                } else {
                    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
                    app.session.undo();
                    assert_eq!(app.session.active().unwrap().doc, before);
                    app.session.redo();
                    assert_eq!(app.session.active().unwrap().doc, after);
                }
            }
        }
    }
}

#[test]
fn drag_rejects_cross_layer_and_stale_queued_bindings_without_history() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let sibling = app
        .session
        .edit("Synthetic Smart sibling", |doc, _| {
            let sibling = doc.layer(layer).unwrap().duplicate();
            let id = sibling.id;
            doc.layers.push(sibling);
            Ok(id)
        })
        .unwrap();
    smart_filter_rows::begin_drag(&mut app, layer, 0).unwrap();
    let drag = app.ui.smart_filter_drag.clone().unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    assert!(smart_filter_rows::drop_command(&app, &drag, sibling, 1, true).is_err());
    let result = smart_filter_rows::action(
        &mut app,
        &egui::Context::default(),
        "ui.smartFilterMove",
        &json!({"layer": sibling.0, "index": 1, "above": true, "drag": drag}),
    )
    .unwrap();
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);

    smart_filter_rows::begin_drag(&mut app, layer, 0).unwrap();
    let drag = app.ui.smart_filter_drag.clone().unwrap();
    assert!(smart_filter_rows::drop_command(&app, &drag, layer, 2, true).unwrap().is_some());
    app.session.execute("layer.smartFilter.move", json!({"layer": layer.0, "index": 0, "to": 1})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    assert!(smart_filter_rows::drop_command(&app, &drag, layer, 2, true).is_err());
    let result = smart_filter_rows::action(
        &mut app,
        &egui::Context::default(),
        "ui.smartFilterMove",
        &json!({"layer": layer.0, "index": 2, "above": true, "drag": drag}),
    )
    .unwrap();
    assert!(result.unwrap_err().contains("changed"), "the queued action must revalidate even after a valid drop was computed");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn native_stack_buttons_collapse_expand_and_open_the_named_blend_editor() {
    use egui_kittest::kittest::Queryable;

    let app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |creation| {
        PhotocraftApp::setup_context(&creation.egui_ctx, Default::default());
        app
    });
    harness.run_steps(4);
    harness.get_by_label("Collapse Smart Filters").click();
    harness.run_steps(2);
    assert!(smart_filter_rows::collapsed(harness.state(), layer));
    assert!(harness.query_by_label("Blending Options: Levels").is_none());
    harness.get_by_label("Expand Smart Filters").click();
    harness.run_steps(2);
    assert!(!smart_filter_rows::collapsed(harness.state(), layer));
    harness.get_by_label("Blending Options: Levels").click();
    harness.run_steps(3);
    let dialog = harness.state().ui.dialogs.last().unwrap();
    assert_eq!(dialog.fields["__command"], "layer.smartFilter.blendingOptions");
    assert_eq!(dialog.fields["__smartFilter"]["layer"], layer.0);
    assert_eq!(dialog.fields["__smartFilter"]["index"], 0);
    assert!(smart_filter_rows::blending_owns(&dialog.fields));
    assert_eq!(dialog.fields["opacity"], 1.0, "idle native frames must preserve the stored opacity");
    assert_eq!(dialog.fields["blend"], "Normal");
    assert!(harness.query_by_label("Preview").is_some());
    assert_eq!(harness.state().session.active().unwrap().doc, before);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), history);
    let dialog = dialog.id;
    crate::dialogs::cancel(harness.state_mut(), dialog).unwrap();
    harness.run_steps(2);
    assert_eq!(harness.state().session.active().unwrap().doc, before);
}

#[test]
fn inspection_reports_smart_mask_selection_stack_disclosure_and_drag_binding() {
    let mut app = app();
    let state = app.session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let document = state.doc.id;
    let revision = state.revision;
    let before = state.doc.clone();
    let history = state.history.past_len();
    let empty = call(&mut app, "ui.inspect", json!({}));
    assert_eq!(empty["result"]["smart_filter_mask_target"], Value::Null);
    assert_eq!(empty["result"]["smart_filter_stacks"], json!([]));
    assert_eq!(empty["result"]["smart_filter_drag"], Value::Null);
    assert_eq!(call(&mut app, "ui.set", json!({"smartFilterMaskTarget": true, "smartFiltersCollapsed": true}))["ok"], true);
    smart_filter_rows::begin_drag(&mut app, layer, 1).unwrap();
    let seen = call(&mut app, "ui.inspect", json!({}));
    assert_eq!(seen["result"]["smart_filter_mask_target"], json!({"document": document.0, "layer": layer.0}));
    assert_eq!(seen["result"]["smart_filter_stacks"], json!([{"document": document.0, "layer": layer.0, "collapsed": true}]));
    assert_eq!(seen["result"]["smart_filter_drag"], json!({"document": document.0, "revision": revision, "layer": layer.0, "index": 1}));
    assert_eq!(call(&mut app, "ui.set", json!({"smartFilterMaskTarget": false, "smartFiltersCollapsed": false}))["ok"], true);
    smart_filter_rows::action(
        &mut app,
        &egui::Context::default(),
        "ui.smartFilterMove",
        &json!({"layer": layer.0, "index": 1, "above": true, "drag": seen["result"]["smart_filter_drag"]}),
    )
    .unwrap()
    .unwrap();
    let cleared = call(&mut app, "ui.inspect", json!({}));
    assert_eq!(cleared["result"]["smart_filter_mask_target"], Value::Null);
    assert_eq!(cleared["result"]["smart_filter_stacks"], json!([]));
    assert_eq!(cleared["result"]["smart_filter_drag"], Value::Null);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn dissolve_dialog_discloses_its_current_approximation_only_for_dissolve() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run(smart_filter_rows::BLENDING, json!({"layer": layer.0, "index": 0, "blend": "Dissolve", "opacity": 0.5})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&harness.ctx, Default::default());
    let dialog = open_blend(harness.state_mut(), layer, 0);
    harness.run_steps(3);
    assert!(harness.query_by_label("Dissolve currently blends smoothly; a speckled Dissolve pattern is not supported yet.").is_some());
    assert_eq!(call(harness.state_mut(), "ui.dialog.set", json!({"dialog": dialog, "field": "blend", "value": "Normal"}))["ok"], true);
    harness.run_steps(2);
    assert!(harness.query_by_label("Dissolve currently blends smoothly; a speckled Dissolve pattern is not supported yet.").is_none());
    assert_eq!(harness.state().session.active().unwrap().doc, before);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), history);
}
