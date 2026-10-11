//! Reduced previews of editable adjustment Smart Filters. Smart source coordinates remain
//! unchanged; their placement and mask are reduced before the engine renders the proxy.

use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::{Value, json};

pub(crate) fn supported(command: &str) -> bool {
    matches!(command, "image.adjustments.levels" | "image.adjustments.curves" | "image.adjustments.shadowsHighlights")
}

fn target(active: Option<LayerId>, command: &str, params: &Value) -> Option<LayerId> {
    if command == "layer.smartFilter.setParams" {
        params.get("layer").and_then(Value::as_u64).map(LayerId).or(active)
    } else if supported(command) && (params.get("target").is_none() || params.get("target").and_then(Value::as_str) == Some("pixels")) {
        active
    } else {
        None
    }
}

/// Unknown and spatial filters retain a full-resolution preview: blindly reducing their
/// parameters could change semantics. Levels/Curves and Shadows/Highlights are qualified here.
pub(crate) fn factor(doc: &Document, active: Option<LayerId>, command: &str, params: &Value, requested: u32) -> u32 {
    let Some(layer) = target(active, command, params).and_then(|id| doc.layer(id)) else { return requested.max(1) };
    let LayerContent::Smart(sm) = &layer.content else { return requested.max(1) };
    if !photocraft_compose::proxy::proxy_faithful(doc) || sm.smart_filters.iter().any(|filter| !supported(&filter.command)) { 1 } else { requested.max(1) }
}

pub(crate) fn scale_params(command: &str, params: &mut Value, k: u32) {
    if k <= 1 || command != "image.adjustments.shadowsHighlights" {
        return;
    }
    if let Some(fields) = params.as_object_mut() {
        // These radii default to 30 px even when the serialized defaults are an empty object.
        for key in ["shadowRadius", "highlightRadius"] {
            if let Some(radius) = scaled_radius(fields.get(key), k) {
                fields.insert(key.into(), json!(radius));
            }
        }
    }
}

/// A re-edit merges into the already scaled stack: absent radii must stay absent, rather
/// than reset an existing nondefault radius to the command's default.
pub(crate) fn scale_edit_params(command: &str, params: &mut Value, k: u32) {
    if k <= 1 || command != "image.adjustments.shadowsHighlights" {
        return;
    }
    if let Some(fields) = params.as_object_mut() {
        for key in ["shadowRadius", "highlightRadius"] {
            if fields.contains_key(key)
                && let Some(radius) = scaled_radius(fields.get(key), k)
            {
                fields.insert(key.into(), json!(radius));
            }
        }
    }
}

fn scaled_radius(value: Option<&Value>, k: u32) -> Option<f64> {
    let radius = match value {
        None | Some(Value::Null) => 30.0,
        Some(value) => value.as_f64()?.clamp(0.0, 2500.0),
    };
    Some(radius / f64::from(k))
}

pub(crate) fn prepare(proxy: &mut Document, active: Option<LayerId>, command: &str, params: &Value, k: u32) {
    if k <= 1 {
        return;
    }
    let Some(layer) = target(active, command, params).and_then(|id| proxy.layer_mut(id)) else { return };
    let LayerContent::Smart(sm) = &mut layer.content else { return };
    let divisor = f64::from(k);
    for value in &mut sm.transform.m {
        *value /= divisor;
    }
    if let Some(matrix) = &mut sm.perspective {
        for value in matrix.iter_mut().take(6) {
            *value /= divisor;
        }
    }
    if let Some(mask) = &mut sm.filter_mask {
        mask.surface = crate::proxy::downsample(&mask.surface, k);
        mask.feather /= k as f32;
    }
    for filter in &mut sm.smart_filters {
        scale_params(&filter.command, &mut filter.params, k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhotocraftApp;
    use photocraft_engine::Session;
    use photocraft_geom::Rect;

    fn app() -> PhotocraftApp {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 96, "height": 64})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("Synthetic blocks", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 24, 16), &[0.25, 0.5, 0.75, 1.0]);
            surf.fill_rect(Rect::new(24, 0, 48, 16), &[0.75, 0.25, 0.5, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        PhotocraftApp::new(s, Default::default())
    }

    fn smart(app: &PhotocraftApp) -> &photocraft_doc::SmartObject {
        let st = app.session.active().unwrap();
        match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            LayerContent::Smart(sm) => sm,
            _ => panic!("smart object"),
        }
    }

    fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
        let (request, _) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &egui::Context::default(), &request) {
            crate::control::Outcome::Done(value) => value,
            _ => panic!("synchronous response"),
        }
    }

    fn open(app: &mut PhotocraftApp) -> u64 {
        let result = control(app, "ui.dialog.open", json!({"kind": "smartFilter", "index": 0}));
        assert_eq!(result["ok"], true, "{result}");
        result["result"]["dialog"].as_u64().unwrap()
    }

    #[test]
    fn smart_adjustment_dialog_previews_replacement_cancel_and_single_step_commit() {
        for (kind, initial, field, value) in [
            ("levels", json!({"gamma": 1.4, "red": {"gamma": 0.7}}), "gamma", json!(0.65)),
            ("curves", json!({"points": [[0, 0], [128, 180], [255, 255]], "blue": [[0, 20], [255, 235]]}), "points", json!([[0, 0], [128, 90], [255, 255]])),
            ("shadowsHighlights", json!({"shadowAmount": 65, "color": 0}), "shadowAmount", json!(10)),
        ] {
            let mut app = app();
            app.session.execute(&format!("image.adjustments.{kind}"), initial).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let steps = app.session.active().unwrap().history.past_len();
            let settings = smart(&app).smart_filters[0].params.clone();
            let mut harness =
                egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&harness.ctx, Default::default());
            let dialog = open(harness.state_mut());
            harness.run_steps(3);
            let app = harness.state_mut();
            for (key, value) in settings.as_object().unwrap() {
                assert_eq!(app.ui.dialogs[0].fields.get(key), Some(value), "{kind}/{key}");
            }
            assert_eq!(control(app, "ui.dialog.set", json!({"dialog":dialog,"field":field,"value":value}))["ok"], true);
            let fields = app.ui.dialogs[0].fields.clone();
            assert_eq!(fields["__preview"], true);
            let (command, params) = crate::smart_ui::confirm_command(&fields, format!("image.adjustments.{kind}"), crate::filter_dialog::params_of(&fields));
            let preview = crate::filter_dialog::preview_document(&before, app.session.active().unwrap().active_layer, &command, &params, 1).unwrap();
            assert_eq!(smart(app).smart_filters.len(), 1);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            assert_eq!(control(app, "ui.dialog.cancel", json!({"dialog":dialog}))["ok"], true);
            assert_eq!(smart(app).smart_filters[0].params, settings);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            let dialog = open(app);
            control(app, "ui.dialog.set", json!({"dialog":dialog,"field":field,"value":value}));
            assert_eq!(control(app, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
            assert_eq!(smart(app).smart_filters.len(), 1);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
            assert_eq!(photocraft_compose::flatten(&preview).px, photocraft_compose::flatten(&app.session.active().unwrap().doc).px, "{kind}");
        }
    }

    #[test]
    fn smart_adjustment_reopen_uses_stored_defaults_instead_of_other_dialog_preferences() {
        for (command, initial, field, expected, remembered) in [
            ("image.adjustments.shadowsHighlights", json!({}), "shadowAmount", json!(35.0), json!({"shadowAmount":92,"blackClip":1,"color":-40})),
            ("filter.blur.gaussianBlur", json!({"radius":2.5}), "radius", json!(2.5), json!({"radius":11})),
        ] {
            let mut app = app();
            app.session.execute(command, initial).unwrap();
            let stored = smart(&app).smart_filters[0].params.clone();
            let before = photocraft_compose::flatten(&app.session.active().unwrap().doc).px;
            app.session.prefs.edit(|prefs| {
                prefs.dialogs.insert(command.into(), remembered.clone());
            });

            let dialog = open(&mut app);
            let fields = &app.ui.dialogs[0].fields;
            assert_eq!(fields[field], expected, "{command}");
            if command == "image.adjustments.shadowsHighlights" {
                assert_eq!(stored, json!({}), "defaults are intentionally omitted from storage");
                assert_eq!(fields["color"], json!(20.0));
                assert!((fields["blackClip"].as_f64().unwrap() - 0.01).abs() < 1e-8);
            }
            assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
            assert_eq!(smart(&app).smart_filters[0].params, stored);
            assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, before);

            let creation = crate::filter_dialog::open(&mut app, command).unwrap();
            assert_eq!(app.ui.dialog_mut(creation).unwrap().fields[field], remembered[field]);
        }
    }

    #[test]
    fn smart_adjustment_preview_scales_translated_placement_and_existing_mask() {
        for k in [2, 3] {
            let mut app = app();
            app.session.execute("edit.transform", json!({"matrix": [1.5,0,0,1.5,12,6]})).unwrap();
            app.session.execute("select.rect", json!({"x": 12,"y":6,"width":30,"height":24})).unwrap();
            app.session.execute("image.adjustments.levels", json!({"gamma":1.4})).unwrap();
            app.session.execute("select.deselect", json!({})).unwrap();
            let st = app.session.active().unwrap();
            let params = json!({"layer":st.active_layer.unwrap().0,"index":0,"params":{"gamma":0.7}});
            let full = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "layer.smartFilter.setParams", &params, 1).unwrap();
            let reduced = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "layer.smartFilter.setParams", &params, k).unwrap();
            assert_eq!((reduced.size.width, reduced.size.height), (96u32.div_ceil(k), 64u32.div_ceil(k)));
            let full = photocraft_compose::flatten(&full);
            let reduced = photocraft_compose::flatten(&reduced);
            for (x, y) in [(0, 0), (18, 12), (30, 18), (54, 12), (84, 48)] {
                let expected = full.px[(y * 96 + x) as usize];
                let got = reduced.px[(y / k * 96u32.div_ceil(k) + x / k) as usize];
                assert_eq!(got, expected, "k={k} at {x},{y}");
            }
        }
    }

    #[test]
    fn smart_adjustment_preview_scales_default_radii_and_keeps_unknown_stacks_full_size() {
        let mut app = app();
        app.session.execute("image.adjustments.shadowsHighlights", json!({})).unwrap();
        let st = app.session.active().unwrap();
        let p = json!({"layer":st.active_layer.unwrap().0,"index":0,"params":{"shadowAmount":60}});
        let preview = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "layer.smartFilter.setParams", &p, 3).unwrap();
        let LayerContent::Smart(sm) = &preview.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
        assert_eq!(sm.smart_filters[0].params["shadowRadius"], json!(10.0));
        assert_eq!(sm.smart_filters[0].params["highlightRadius"], json!(10.0));
        app.session.execute("filter.blur.gaussianBlur", json!({"radius":2})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(factor(&st.doc, st.active_layer, "layer.smartFilter.setParams", &p, 3), 1);
    }

    #[test]
    fn smart_adjustment_preview_edits_selected_alpha_without_reading_smart_source() {
        let mut app = app();
        app.session
            .edit("Synthetic alpha and unavailable source", |doc, active| {
                let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
                surface.fill_rect(doc.bounds(), &[0.25]);
                doc.channels.push(photocraft_doc::AlphaChannel::new("Synthetic alpha", surface));
                let LayerContent::Smart(sm) = &mut doc.layer_mut(active.unwrap()).unwrap().content else { panic!() };
                sm.source = photocraft_doc::SmartSource::Linked { path: "/unavailable/smart-source.pcraft".into() };
                Ok(())
            })
            .unwrap();
        app.session.execute("channel.target", json!({"channel":0})).unwrap();
        let command = "image.adjustments.levels";
        let params = photocraft_engine::channel_cmds::inject_target(&app.session, command, app.with_mask_target(command, json!({"gamma":1.8})));
        assert_eq!(params["target"], json!({"channel":0}));
        let st = app.session.active().unwrap();
        let preview = crate::filter_dialog::preview_document(&st.doc, st.active_layer, command, &params, 1).unwrap();
        assert_eq!(&preview.layer(st.active_layer.unwrap()).unwrap().content, &st.doc.layer(st.active_layer.unwrap()).unwrap().content);
        assert!((preview.channels[0].surface.sample_channel(2, 2, 0) - 0.25).abs() > 0.01);
        app.session.execute(command, json!({"gamma":1.8})).unwrap();
        assert_eq!(preview.channels, app.session.active().unwrap().doc.channels);
    }

    #[test]
    fn smart_adjustment_radius_proxy_matches_clamping_nulls_and_bad_input() {
        let command = "image.adjustments.shadowsHighlights";
        let mut params = json!({"shadowRadius":1e300,"highlightRadius":null});
        scale_edit_params(command, &mut params, 4);
        assert_eq!(params, json!({"shadowRadius":625.0,"highlightRadius":7.5}));
        let mut bad = json!({"shadowRadius":"bad"});
        scale_params(command, &mut bad, 4);
        assert_eq!(bad["shadowRadius"], "bad", "invalid values remain invalid for command validation");
        let mut partial = json!({"shadowAmount":55});
        scale_edit_params(command, &mut partial, 4);
        assert_eq!(partial, json!({"shadowAmount":55}), "a partial edit must retain the stored radius");
    }

    #[test]
    fn smart_adjustment_dialog_refuses_stale_document_and_unauthorized_linked_source() {
        let mut app = app();
        app.session.execute("image.adjustments.levels", json!({"gamma":1.4})).unwrap();
        let dialog = open(&mut app);
        app.session.execute("layer.smartFilter.delete", json!({"index":0})).unwrap();
        assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], false);
        assert_eq!(smart(&app).smart_filters.len(), 0);
        app.session.execute("image.adjustments.levels", json!({})).unwrap();
        app.session
            .edit("Missing link", |doc, active| {
                let LayerContent::Smart(sm) = &mut doc.layer_mut(active.unwrap()).unwrap().content else { panic!() };
                sm.source = photocraft_doc::SmartSource::Linked { path: "/unauthorized/smart-source.pcraft".into() };
                Ok(())
            })
            .unwrap();
        app.services.automation_authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("test source access denied".into())));
        let result = control(&mut app, "ui.dialog.open", json!({"kind":"smartFilter","index":0}));
        assert_eq!(result["ok"], false);
        assert!(app.ui.dialogs.is_empty());
        assert!(app.ui.status.contains("test source access denied"));
    }

    #[test]
    #[ignore = "24 MP preview benchmark; run explicitly on an otherwise idle machine"]
    fn smart_adjustment_large_preview_measurement() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":6000,"height":4000})).unwrap();
        s.edit("Synthetic 24 MP tones", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 3000, 4000), &[0.25, 0.5, 0.75, 1.0]);
            surf.fill_rect(Rect::new(3000, 0, 6000, 4000), &[0.75, 0.25, 0.5, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("image.adjustments.levels", json!({"gamma":1.1})).unwrap();
        let st = s.active().unwrap();
        let k = crate::proxy::factor(&st.doc);
        assert!(st.doc.size.area() / u64::from(k * k) <= crate::proxy::PREVIEW_PIXELS);
        let params = json!({"layer":st.active_layer.unwrap().0,"index":0,"params":{"gamma":0.8}});
        let mut timings = Vec::new();
        for factor in [1, k] {
            let mut elapsed = Vec::new();
            for _ in 0..3 {
                let start = std::time::Instant::now();
                let doc = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "layer.smartFilter.setParams", &params, factor).unwrap();
                elapsed.push(start.elapsed().as_secs_f64());
                let LayerContent::Smart(sm) = &doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
                let actual = sm.cache.as_ref().unwrap().content_bounds();
                assert!(actual.width() <= 6000 / factor && actual.height() <= 4000 / factor);
            }
            elapsed.sort_by(f64::total_cmp);
            timings.push(elapsed[1]);
        }
        eprintln!(
            "SMART_ADJUST_PREVIEW_24MP full_median_s={:.6} reduced_median_s={:.6} factor={} speedup={:.2}",
            timings[0],
            timings[1],
            k,
            timings[0] / timings[1]
        );
    }
}
