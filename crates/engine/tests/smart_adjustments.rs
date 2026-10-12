//! Image adjustments on Smart Objects stay editable, undoable and serializable. All fixtures
//! are generated here; the destructive command is the pixel oracle for the smart workflow.

use photocraft_color::PixelFormat;
use photocraft_doc::{AlphaChannel, BlendMode, Document, Layer, LayerContent, LayerId, LayerMask, SmartFilter, SmartObject, SmartSource};
use photocraft_engine::{EngineError, Session};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const WIDTH: i32 = 24;
const HEIGHT: i32 = 16;

fn session(depth: u64) -> Session {
    session_in_mode(depth, "rgb")
}

fn session_in_mode(depth: u64, mode: &str) -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": WIDTH, "height": HEIGHT, "depth": depth, "mode": mode})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Synthetic tones"})).unwrap();
    session
        .edit("Synthetic fixture", |doc, active| {
            let bounds = doc.bounds();
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let format = surface.format();
            let mut pixels = Vec::new();
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let tone = (x + y * 3) as f32 / 70.0;
                    pixels.extend(photocraft_raster::from_rgba(&format, [0.05 + tone * 0.8, 0.8 - tone * 0.6, 0.15 + tone * 0.5, 1.0]));
                }
            }
            surface.write_region(bounds, &pixels);
            Ok(())
        })
        .unwrap();
    session
}

fn convert(session: &mut Session) -> LayerId {
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.active().unwrap().active_layer.unwrap()
}

fn smart(doc: &Document, layer: LayerId) -> &SmartObject {
    match &doc.layer(layer).unwrap().content {
        LayerContent::Smart(smart) => smart,
        other => panic!("expected a smart object, got {}", other.kind_name()),
    }
}

fn only_smart(doc: &Document) -> (LayerId, &SmartObject) {
    doc.walk()
        .into_iter()
        .find_map(|(_, _, layer)| match &layer.content {
            LayerContent::Smart(smart) => Some((layer.id, smart)),
            _ => None,
        })
        .unwrap()
}

fn pixels(doc: &Document) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(doc).px
}

fn difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0, f32::max)
}

fn depth_tolerance(depth: u64) -> f32 {
    match depth {
        8 => 1.0 / 255.0 + 1e-6,
        16 => 2.0 / 65535.0 + 1e-6,
        _ => 1e-6,
    }
}

fn cases() -> [(&'static str, Value, Value); 5] {
    [
        ("levels", json!({"inBlack": 15, "inWhite": 240, "gamma": 1.3, "red": {"gamma": 0.8}}), json!({"gamma": 0.72})),
        ("curves", json!({"points": [[0, 0], [128, 180], [255, 255]], "blue": [[0, 14], [255, 239]]}), json!({"points": [[0, 10], [160, 132], [255, 255]]})),
        (
            "shadowsHighlights",
            json!({"shadowAmount": 65, "shadowRadius": 3, "highlightAmount": 35, "highlightRadius": 2, "blackClip": 0, "whiteClip": 0, "color": 0}),
            json!({"shadowAmount": 20}),
        ),
        (
            "hueSaturation",
            json!({"hue":25,"saturation":35,"lightness":-8,"reds":{"hue":40,"saturation":-20,"range":[320,340,380,410]},"blues":{"lightness":12}}),
            json!({"hue":-30,"saturation":15}),
        ),
        (
            "hueSaturation",
            json!({"colorize":true,"hue":210,"saturation":10,"lightness":8,"reds":{"hue":35,"range":[320,340,380,410]}}),
            json!({"colorize":false,"hue":-25,"saturation":20}),
        ),
    ]
}

fn destructive(doc: &Document, layer: LayerId, command: &str, params: Value) -> Vec<[f32; 4]> {
    let mut session = Session::new();
    session.add_document(doc.clone(), None);
    session.select_layer(layer).unwrap();
    session.execute(command, params).unwrap();
    assert!(matches!(session.active().unwrap().doc.layer(layer).unwrap().content, LayerContent::Raster(_)));
    pixels(&session.active().unwrap().doc)
}

#[test]
fn create_and_reedit_match_destructive_at_every_depth_and_roundtrip_natively() {
    for depth in [8, 16, 32] {
        for (kind, params, patch) in cases() {
            let command = format!("image.adjustments.{kind}");
            let mut session = session(depth);
            let raster = session.active().unwrap().doc.clone();
            let raster_layer = session.active().unwrap().active_layer.unwrap();
            let expected = destructive(&raster, raster_layer, &command, params.clone());
            let layer = convert(&mut session);
            let before = session.active().unwrap().doc.clone();
            let original = smart(&before, layer).clone();
            let steps = session.active().unwrap().history.past_len();

            assert!(session.is_enabled(&command), "{command} must be enabled on a Smart Object");
            session.execute(&command, params.clone()).unwrap();
            let created = session.active().unwrap().doc.clone();
            let filter = &smart(&created, layer).smart_filters;
            assert_eq!(filter.len(), 1);
            assert_eq!(filter[0].command, command);
            assert_eq!(smart(&created, layer).source, original.source);
            assert_eq!(smart(&created, layer).transform, original.transform);
            assert_eq!(session.active().unwrap().history.past_len(), steps + 1);
            assert!(difference(&pixels(&created), &expected) <= depth_tolerance(depth), "{kind}, {depth}-bit creation");
            assert!(difference(&pixels(&created), &pixels(&before)) > 0.01, "the fixture must exercise the adjustment");

            let mut merged = params;
            merged.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            let expected = destructive(&raster, raster_layer, &command, merged);
            session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": patch})).unwrap();
            let edited = session.active().unwrap().doc.clone();
            assert_eq!(smart(&edited, layer).smart_filters.len(), 1, "editing replaces, never appends");
            assert_eq!(smart(&edited, layer).source, original.source);
            assert_eq!(smart(&edited, layer).transform, original.transform);
            assert_eq!(session.active().unwrap().history.past_len(), steps + 2);
            assert!(difference(&pixels(&edited), &expected) <= depth_tolerance(depth), "{kind}, {depth}-bit re-edit");
            session.undo();
            assert_eq!(*session.active().unwrap().doc, *created);
            session.undo();
            assert_eq!(*session.active().unwrap().doc, *before);
            session.redo();
            session.redo();
            assert_eq!(*session.active().unwrap().doc, *edited);

            let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(smart(&reopened, layer), smart(&edited, layer), "{kind}, {depth}-bit native roundtrip");
            assert_eq!(pixels(&reopened), pixels(&edited));
        }
    }
}

#[test]
fn neutral_hue_filter_opacity_preserves_transparency_and_source() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "grayscale", "cmyk", "lab"] {
            let mut session = session_in_mode(depth, mode);
            session
                .edit("Partial alpha fixture", |doc, active| {
                    let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                    let format = surface.format();
                    for x in 0..WIDTH {
                        let alpha = (x + 1) as f32 / (WIDTH + 1) as f32;
                        surface.write_pixel(x, 0, &photocraft_raster::from_rgba(&format, [0.8, 0.2, 0.4, alpha]));
                    }
                    Ok(())
                })
                .unwrap();
            let layer = convert(&mut session);
            let original = smart(&session.active().unwrap().doc, layer).clone();
            session.execute("image.adjustments.hueSaturation", json!({})).unwrap();
            let neutral = smart(&session.active().unwrap().doc, layer).cache.as_ref().unwrap().read_region(session.active().unwrap().doc.bounds());
            for opacity in [0.0, 0.1, 0.5, 1.0] {
                let before = session.active().unwrap().doc.clone();
                session.execute("layer.smartFilter.blendingOptions", json!({"index":0,"blend":"normal","opacity":opacity})).unwrap();
                let doc = &session.active().unwrap().doc;
                let actual = smart(doc, layer);
                let cache = actual.cache.as_ref().unwrap().read_region(doc.bounds());
                assert!(cache.iter().zip(&neutral).all(|(a, b)| (a - b).abs() <= depth_tolerance(depth) * 2.0), "{mode}/{depth}/{opacity}");
                assert_eq!(actual.source, original.source);
                let after = doc.clone();
                session.undo();
                assert_eq!(session.active().unwrap().doc, before);
                session.redo();
                assert_eq!(session.active().unwrap().doc, after);
            }
        }
    }
}

fn assert_psd_structure(bytes: &[u8]) {
    let parsed = photocraft_psd::PsdFile::from_bytes(bytes).unwrap();
    for block in &parsed.global_blocks {
        block.check_structure().unwrap();
    }
    for layer in parsed.layers() {
        for block in &layer.blocks {
            block.check_structure().unwrap();
        }
    }
}

#[test]
fn rgb_levels_and_curves_keep_parameters_and_rendering_through_psd() {
    for depth in [8, 16, 32] {
        for (kind, params, _) in cases().into_iter().take(2) {
            let mut session = session(depth);
            let layer = convert(&mut session);
            session.execute(&format!("image.adjustments.{kind}"), params).unwrap();
            let doc = &session.active().unwrap().doc;
            let exported = photocraft_io::export(doc, "psd", &Default::default()).unwrap();
            assert!(exported.warnings.is_empty(), "{kind}, {depth}-bit: {:?}", exported.warnings);
            assert_psd_structure(&exported.bytes);
            let reopened = photocraft_io::import("synthetic-adjustment.psd", &exported.bytes).unwrap().document;
            let (imported_layer, imported) = only_smart(&reopened);
            assert_eq!(imported.smart_filters.len(), 1);
            let original = &smart(doc, layer).smart_filters[0];
            let actual = &imported.smart_filters[0];
            assert_eq!(
                (&actual.command, actual.blend, actual.opacity, actual.visible),
                (&original.command, original.blend, original.opacity, original.visible)
            );
            let adjustment = |params: &Value| photocraft_engine::adjust_params::from_params(kind, params, None, doc.mode).unwrap();
            assert_eq!(adjustment(&actual.params), adjustment(&original.params), "{kind}: PSD parameter fidelity");
            let mut rerendered = reopened.clone();
            assert!(photocraft_engine::smart_cmds::refresh(&mut rerendered, imported_layer).unwrap());
            assert!(difference(&pixels(&rerendered), &pixels(doc)) <= 2.0 / 255.0 + 1e-6, "{kind}, {depth}-bit: source-based PSD render");
        }
    }
}

#[test]
fn shadows_highlights_default_psd_mapping_and_custom_export_warning_are_explicit() {
    for params in [
        json!({}),
        json!({"shadowAmount": 35, "shadowTone": 50, "shadowRadius": 30, "highlightAmount": 0, "highlightTone": 50, "highlightRadius": 30,
               "color": 20, "midtone": 0, "blackClip": 0.01, "whiteClip": 0.01}),
    ] {
        let mut session = session(8);
        let layer = convert(&mut session);
        session.execute("image.adjustments.shadowsHighlights", params).unwrap();
        assert_eq!(smart(&session.active().unwrap().doc, layer).smart_filters[0].params, json!({}), "default-valued keys stay PSD-writable");
        let exported = photocraft_io::export(&session.active().unwrap().doc, "psd", &Default::default()).unwrap();
        assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
        assert_psd_structure(&exported.bytes);
        let reopened = photocraft_io::import("default-shadows.psd", &exported.bytes).unwrap().document;
        assert_eq!(only_smart(&reopened).1.smart_filters, smart(&session.active().unwrap().doc, layer).smart_filters);

        session.execute("layer.smartFilter.setParams", json!({"index": 0, "params": {"shadowAmount": 70}})).unwrap();
        let exported = photocraft_io::export(&session.active().unwrap().doc, "psd", &Default::default()).unwrap();
        assert!(
            exported.warnings.iter().any(|warning| warning.contains("image.adjustments.shadowsHighlights") && warning.contains("not written to PSD")),
            "a native-only setting must never silently disappear: {:?}",
            exported.warnings
        );
        session.execute("layer.smartFilter.setParams", json!({"index": 0, "params": {"shadowAmount": 35}})).unwrap();
        assert_eq!(smart(&session.active().unwrap().doc, layer).smart_filters[0].params, json!({}));
    }
}

#[test]
fn partial_range_and_channel_edits_keep_other_controls_and_null_resets_only_that_record() {
    for (kind, initial, record, field, value) in [
        (
            "hueSaturation",
            json!({"hue":20,"reds":{"hue":35,"saturation":-20,"lightness":12,"range":[320,340,380,410]},"blues":{"lightness":15}}),
            "reds",
            "hue",
            json!(-40),
        ),
        (
            "levels",
            json!({"gamma":1.2,"red":{"inBlack":20,"inWhite":220,"gamma":1.5,"outBlack":10,"outWhite":240},"blue":{"gamma":0.8}}),
            "red",
            "gamma",
            json!(0.65),
        ),
    ] {
        let mut session = session(16);
        let layer = convert(&mut session);
        session.execute(&format!("image.adjustments.{kind}"), initial).unwrap();
        let before = session.active().unwrap().doc.clone();
        let original = smart(&before, layer).smart_filters[0].params.clone();
        session.execute("layer.smartFilter.setParams", json!({"index":0,"params":{record:{field:value}}})).unwrap();
        let edited = smart(&session.active().unwrap().doc, layer).smart_filters[0].params.clone();
        assert!((edited[record][field].as_f64().unwrap() - value.as_f64().unwrap()).abs() < 1e-6);
        for (key, old) in original.as_object().unwrap() {
            if key == record {
                for (name, old) in old.as_object().unwrap() {
                    if name != field {
                        assert_eq!(&edited[key][name], old, "{kind}: editing {field} must preserve {name}");
                    }
                }
            } else {
                assert_eq!(&edited[key], old, "{kind}: another record changed");
            }
        }
        session.undo();
        assert_eq!(*session.active().unwrap().doc, *before);
        session.redo();
        session.execute("layer.smartFilter.setParams", json!({"index":0,"params":{record:null}})).unwrap();
        let defaults = photocraft_engine::adjust_params::from_params(kind, &json!({}), None, before.mode).unwrap();
        let defaults = photocraft_engine::adjust_params::to_params(&defaults);
        let reset = &smart(&session.active().unwrap().doc, layer).smart_filters[0].params;
        assert_eq!(reset[record], defaults[record]);
        for (key, value) in edited.as_object().unwrap() {
            if key != record {
                assert_eq!(&reset[key], value);
            }
        }
    }
}

#[test]
fn hue_colorize_and_all_six_ranges_roundtrip_natively_in_every_supported_color_model() {
    for mode in ["rgb", "grayscale", "cmyk", "lab"] {
        for depth in [8, 16] {
            for colorize in [false, true] {
                let mut session = session_in_mode(depth, mode);
                let layer = convert(&mut session);
                let source = smart(&session.active().unwrap().doc, layer).source.clone();
                let mut params = json!({"hue":35,"saturation":10,"lightness":8,"colorize":colorize});
                for (index, range) in photocraft_engine::adjust_params::HUE_RANGES.iter().enumerate() {
                    let angle = index as i32 * 60;
                    params[*range] = json!({"hue":index as i32 * 5 + 5,"saturation":-15,"lightness":7,"range":[angle-30,angle-10,angle+15,angle+40]});
                }
                session.execute("image.adjustments.hueSaturation", params).unwrap();
                let original = session.active().unwrap().doc.clone();
                let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&original, &Default::default()).unwrap()).unwrap();
                assert_eq!(smart(&reopened, layer), smart(&original, layer), "{mode}/{depth}/{colorize}");
                assert_eq!(smart(&reopened, layer).source, source);
                assert_eq!(pixels(&reopened), pixels(&original));
                let mut resumed = Session::new();
                resumed.add_document(reopened, None);
                resumed.execute("layer.smartFilter.setParams", json!({"layer":layer.0,"index":0,"params":{"colorize":!colorize}})).unwrap();
                let changed = &smart(&resumed.active().unwrap().doc, layer).smart_filters[0].params;
                for range in photocraft_engine::adjust_params::HUE_RANGES {
                    assert_eq!(changed[range], smart(&original, layer).smart_filters[0].params[range]);
                }
                resumed.undo();
                assert_eq!(*resumed.active().unwrap().doc, *original);
            }
        }
    }
}

#[test]
fn hue_smart_filter_psd_export_warns_and_preserves_cached_rendered_appearance() {
    for depth in [8, 16, 32] {
        for colorize in [false, true] {
            let mut session = session(depth);
            let layer = convert(&mut session);
            session.execute("image.adjustments.hueSaturation", json!({"hue":35,"saturation":10,"lightness":8,"colorize":colorize})).unwrap();
            let original = session.active().unwrap().doc.clone();
            let exported = photocraft_io::export(&original, "psd", &Default::default()).unwrap();
            assert!(
                exported.warnings.iter().any(|warning| warning.contains("image.adjustments.hueSaturation") && warning.contains("not written to PSD")),
                "{:?}",
                exported.warnings
            );
            assert_psd_structure(&exported.bytes);
            let reopened = photocraft_io::import("synthetic-hue.psd", &exported.bytes).unwrap().document;
            assert!(only_smart(&reopened).1.smart_filters.is_empty(), "no invented Hue Smart Filter descriptor");
            assert!(difference(&pixels(&reopened), &pixels(&original)) <= 2.0 / 255.0 + 1e-6, "cached appearance, {depth}/{colorize}");
            assert_eq!(smart(&session.active().unwrap().doc, layer), smart(&original, layer), "export leaves native editing data intact");
        }
    }
}

#[test]
fn selection_becomes_the_filter_mask_and_later_adjustments_preserve_it() {
    let mut session = session(16);
    let layer = convert(&mut session);
    let before = pixels(&session.active().unwrap().doc);
    session.execute("select.rect", json!({"x": 0, "y": 0, "width": WIDTH / 2, "height": HEIGHT})).unwrap();
    session.execute("image.adjustments.levels", json!({"gamma": 2})).unwrap();
    let mask = smart(&session.active().unwrap().doc, layer).filter_mask.clone().unwrap();
    assert_eq!(mask.surface.sample_channel(2, 2, 0), 1.0);
    assert_eq!(mask.surface.sample_channel(WIDTH - 2, 2, 0), 0.0);
    session.execute("select.rect", json!({"x": WIDTH / 2, "y": 0, "width": WIDTH / 2, "height": HEIGHT})).unwrap();
    session.execute("image.adjustments.curves", json!({"points": [[0, 40], [255, 255]]})).unwrap();
    assert_eq!(smart(&session.active().unwrap().doc, layer).filter_mask.as_ref(), Some(&mask));
    let after = pixels(&session.active().unwrap().doc);
    for y in 0..HEIGHT as usize {
        for x in WIDTH as usize / 2..WIDTH as usize {
            let index = y * WIDTH as usize + x;
            assert_eq!(after[index], before[index], "outside the original filter mask");
        }
    }
    assert!(difference(&before, &after) > 0.01);
}

fn assert_rejected_without_edit(session: &mut Session, command: &str, params: Value) {
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let steps = session.active().unwrap().history.past_len();
    assert!(session.execute(command, params).is_err(), "{command} must reject the request");
    assert_eq!(*session.active().unwrap().doc, *before);
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), steps);
}

#[test]
fn malformed_creation_and_merged_edits_leave_pixels_parameters_and_history_intact() {
    let invalid = [
        ("levels", json!({"gamma": "invalid"})),
        ("levels", json!({"red": 3})),
        ("curves", json!({"points": [[0, 0], [0, 255]]})),
        ("curves", json!({"blue": "invalid"})),
        ("shadowsHighlights", json!({"shadowAmount": "invalid"})),
        ("shadowsHighlights", json!({"shadowRadius": {"pixels": 3}})),
        ("hueSaturation", json!({"colorize": 1})),
        ("hueSaturation", json!({"reds": {"range": [1, 2]}})),
        ("hueSaturation", json!({"saturation": "invalid"})),
    ];
    for (kind, invalid) in invalid {
        let mut session = session(8);
        convert(&mut session);
        let command = format!("image.adjustments.{kind}");
        assert_rejected_without_edit(&mut session, &command, invalid.clone());
        session.execute(&command, json!({})).unwrap();
        assert_rejected_without_edit(&mut session, "layer.smartFilter.setParams", json!({"index": 0, "params": invalid}));
        assert_rejected_without_edit(&mut session, "layer.smartFilter.setParams", json!({"index": 0, "params": [1, 2]}));
    }
}

#[test]
fn pixel_and_all_locks_apply_to_creation_and_reediting_including_parent_groups() {
    for inherited in [false, true] {
        for all in [false, true] {
            let mut session = session(8);
            let layer = convert(&mut session);
            session.execute("image.adjustments.levels", json!({"gamma": 1.3})).unwrap();
            session
                .edit("Lock fixture", |doc, _| {
                    let locked = if inherited {
                        let index = doc.layers.iter().position(|candidate| candidate.id == layer).unwrap();
                        let child = doc.layers.remove(index);
                        let group = Layer::group("Locked parent", vec![child]);
                        let group_id = group.id;
                        doc.layers.insert(index, group);
                        group_id
                    } else {
                        layer
                    };
                    let locks = &mut doc.layer_mut(locked).unwrap().locks;
                    locks.all = all;
                    locks.pixels = !all;
                    Ok(())
                })
                .unwrap();
            for (kind, params, _) in cases() {
                assert_rejected_without_edit(&mut session, &format!("image.adjustments.{kind}"), params);
            }
            assert_rejected_without_edit(&mut session, "layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"gamma": 0.8}}));
        }
    }
}

#[test]
fn reediting_an_explicit_nonactive_smart_layer_keeps_the_active_layer() {
    let mut session = session(8);
    let layer = convert(&mut session);
    session.execute("image.adjustments.curves", json!({"points": [[0, 0], [128, 160], [255, 255]]})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Stay selected"})).unwrap();
    let active = session.active().unwrap().active_layer;
    let before = smart(&session.active().unwrap().doc, layer).clone();
    session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"points": [[0, 20], [255, 255]]}})).unwrap();
    assert_eq!(session.active().unwrap().active_layer, active);
    let after = smart(&session.active().unwrap().doc, layer);
    assert_eq!(after.smart_filters.len(), 1);
    assert_ne!(after.cache, before.cache);
    session.undo();
    assert_eq!(smart(&session.active().unwrap().doc, layer), &before);
    assert_eq!(session.active().unwrap().active_layer, active);
}

#[test]
fn mask_alpha_and_quick_mask_targets_are_edited_without_adding_smart_filters() {
    for (kind, params, _) in cases() {
        for target in ["mask", "alpha", "quickMask"] {
            let mut session = session(8);
            let layer = convert(&mut session);
            session
                .edit("Channel fixture", |doc, _| {
                    let mut surface = Surface::new(PixelFormat::GRAY8);
                    surface.fill_rect(doc.bounds(), &[0.25]);
                    match target {
                        "mask" => doc.layer_mut(layer).unwrap().mask = Some(LayerMask { surface, ..LayerMask::reveal_all() }),
                        "alpha" => doc.channels.push(AlphaChannel::new("Synthetic alpha", surface)),
                        _ => doc.quick_mask = Some(AlphaChannel::new("Quick Mask", surface)),
                    }
                    Ok(())
                })
                .unwrap();
            if target == "alpha" {
                session.execute("channel.target", json!({"channel": 0})).unwrap();
            }
            let before = smart(&session.active().unwrap().doc, layer).clone();
            let mut params = params.clone();
            if target == "mask" {
                params["target"] = json!("mask");
            }
            session.execute(&format!("image.adjustments.{kind}"), params).unwrap();
            let doc = &session.active().unwrap().doc;
            assert_eq!(smart(doc, layer), &before, "{kind} on {target} must not edit the smart object's pixels or filters");
            let surface = match target {
                "mask" => &doc.layer(layer).unwrap().mask.as_ref().unwrap().surface,
                "alpha" => &doc.channels[0].surface,
                _ => &doc.quick_mask.as_ref().unwrap().surface,
            };
            assert!((surface.sample_channel(2, 2, 0) - 0.25).abs() > 0.01, "{kind} must edit {target}");
        }
    }
}

#[test]
fn a_single_color_channel_never_creates_an_unrestricted_smart_filter() {
    for (kind, params, _) in cases() {
        let mut session = session(8);
        convert(&mut session);
        session.execute("channel.target", json!({"channel": "red"})).unwrap();
        assert_rejected_without_edit(&mut session, &format!("image.adjustments.{kind}"), params);
    }
}

#[test]
fn native_tone_filters_keep_document_channel_spaces_and_non_rgb_psd_limits_are_reported() {
    for depth in [8, 16] {
        for mode in ["rgb", "grayscale", "cmyk", "lab"] {
            let mut session = session_in_mode(depth, mode);
            let layer = convert(&mut session);
            let (levels, curves, edit) = match mode {
                "cmyk" => (
                    json!({"cyan": {"gamma": 1.4}, "black": {"gamma": 0.85}}),
                    json!({"cyan": [[0, 0], [128, 155], [255, 255]], "black": [[0, 12], [255, 245]]}),
                    json!({"cyan": {"gamma": 0.7}}),
                ),
                "lab" => (
                    json!({"lightness": {"gamma": 1.4}, "a": {"inBlack": 12}}),
                    json!({"lightness": [[0, 0], [128, 155], [255, 255]], "a": [[0, 12], [255, 245]]}),
                    json!({"lightness": {"gamma": 0.7}}),
                ),
                _ => (json!({"gamma": 1.4}), json!({"points": [[0, 0], [128, 155], [255, 255]]}), json!({"gamma": 0.7})),
            };
            session.execute("image.adjustments.levels", levels).unwrap();
            session.execute("image.adjustments.curves", curves).unwrap();
            let before = session.active().unwrap().doc.clone();
            let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!((reopened.mode, reopened.depth), (before.mode, before.depth));
            assert_eq!(smart(&reopened, layer), smart(&before, layer), "{mode}, {depth}-bit native settings and cache");
            assert_eq!(pixels(&reopened), pixels(&before));
            let mut resumed = Session::new();
            resumed.add_document(reopened, None);
            resumed.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": edit})).unwrap();
            let edited = resumed.active().unwrap().doc.clone();
            assert_eq!(smart(&edited, layer).smart_filters.len(), 2);
            assert!(difference(&pixels(&edited), &pixels(&before)) > 0.01, "{mode} edit must reach its named channel");
            resumed.undo();
            assert_eq!(smart(&resumed.active().unwrap().doc, layer), smart(&before, layer));

            if matches!(mode, "cmyk" | "lab") {
                let exported = photocraft_io::export(&before, "psd", &Default::default()).unwrap();
                assert_psd_structure(&exported.bytes);
                for kind in ["levels", "curves"] {
                    assert!(
                        exported
                            .warnings
                            .iter()
                            .any(|warning| { warning.contains(&format!("image.adjustments.{kind}")) && warning.contains("not written to PSD") }),
                        "{mode}, {depth}-bit {kind} must explicitly report unmapped PSD channels: {:?}",
                        exported.warnings
                    );
                }
            }
        }
    }
}

fn add_unsupported_filter(session: &mut Session, layer: LayerId, visible: bool, stack_enabled: bool) {
    session
        .edit("Preserved unsupported fixture", |doc, _| {
            let LayerContent::Smart(smart) = &mut doc.layer_mut(layer).unwrap().content else { panic!("expected Smart Object") };
            smart.filters_enabled = stack_enabled;
            smart.smart_filters.push(SmartFilter {
                command: photocraft_engine::smart_cmds::UNSUPPORTED_FILTER.into(),
                params: json!({"name": "Synthetic unmapped filter", "psd": "00000000"}),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible,
            });
            // Imported unknown filters can contribute to this cache. Do not rerender here:
            // a rejected edit must preserve the trusted imported appearance byte for byte.
            Ok(())
        })
        .unwrap();
}

#[test]
fn visible_unsupported_filters_block_creation_opening_and_reediting_without_losing_the_cache() {
    let mut session = session(16);
    let layer = convert(&mut session);
    session.execute("image.adjustments.levels", json!({"gamma": 1.3})).unwrap();
    add_unsupported_filter(&mut session, layer, true, true);
    let before = session.active().unwrap().doc.clone();
    assert!(photocraft_engine::smart_cmds::check_filter_edit(&session, layer).is_err());
    assert_eq!(*session.active().unwrap().doc, *before);
    for (kind, params, _) in cases() {
        assert_rejected_without_edit(&mut session, &format!("image.adjustments.{kind}"), params);
    }
    assert_rejected_without_edit(&mut session, "layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"gamma": 0.8}}));
}

#[test]
fn hidden_unsupported_filters_and_disabled_stacks_allow_supported_edits() {
    for (visible, stack_enabled) in [(false, true), (true, false)] {
        let mut session = session(8);
        let layer = convert(&mut session);
        session.execute("image.adjustments.levels", json!({"gamma": 1.3})).unwrap();
        add_unsupported_filter(&mut session, layer, visible, stack_enabled);
        let preserved = smart(&session.active().unwrap().doc, layer).smart_filters[1].clone();
        assert!(photocraft_engine::smart_cmds::check_filter_edit(&session, layer).is_ok());
        session.execute("image.adjustments.curves", json!({"points": [[0, 20], [255, 255]]})).unwrap();
        session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"gamma": 0.8}})).unwrap();
        let smart = smart(&session.active().unwrap().doc, layer);
        assert_eq!(smart.smart_filters.len(), 3);
        assert_eq!(smart.smart_filters[1], preserved, "the opaque descriptor and visibility remain intact");
        assert_eq!(smart.filters_enabled, stack_enabled);
        assert!((smart.smart_filters[0].params["gamma"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    }
}

fn replace_with_missing_link(session: &mut Session, layer: LayerId) {
    session
        .edit("Unavailable linked source fixture", |doc, _| {
            let LayerContent::Smart(smart) = &mut doc.layer_mut(layer).unwrap().content else { panic!("expected Smart Object") };
            smart.source = SmartSource::Linked { path: "/__photocraft_synthetic_missing_source__/source.pcraft".into() };
            Ok(())
        })
        .unwrap();
}

#[test]
fn missing_linked_sources_preserve_the_cached_image_and_refuse_filter_edits() {
    let mut session = session(8);
    let layer = convert(&mut session);
    session.execute("image.adjustments.levels", json!({"gamma": 1.3})).unwrap();
    replace_with_missing_link(&mut session, layer);
    assert!(photocraft_engine::smart_cmds::check_filter_edit(&session, layer).is_err());
    for (kind, params, _) in cases() {
        assert_rejected_without_edit(&mut session, &format!("image.adjustments.{kind}"), params);
    }
    assert_rejected_without_edit(&mut session, "layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"gamma": 0.8}}));
}

#[test]
fn linked_source_authorization_checks_the_document_path_before_resolving_it() {
    fn authorize(_: &str, params: &Value) -> photocraft_engine::Result<()> {
        if params.get("path").is_some() { Err(EngineError::Other("synthetic source authorization denied".into())) } else { Ok(()) }
    }
    let mut session = session(8);
    let layer = convert(&mut session);
    session.execute("image.adjustments.levels", json!({"gamma": 1.3})).unwrap();
    replace_with_missing_link(&mut session, layer);
    session.authorize = Some(authorize);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let steps = session.active().unwrap().history.past_len();
    assert!(photocraft_engine::smart_cmds::check_filter_edit(&session, layer).unwrap_err().to_string().contains("synthetic source authorization denied"));
    for (command, params) in [
        ("image.adjustments.curves", json!({"points": [[0, 20], [255, 255]]})),
        ("layer.smartFilter.setParams", json!({"layer": layer.0, "index": 0, "params": {"gamma": 0.8}})),
    ] {
        let error = session.execute(command, params).unwrap_err();
        assert!(error.to_string().contains("synthetic source authorization denied"), "authorize the stored source before attempting its read: {error}");
        assert_eq!(*session.active().unwrap().doc, *before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.past_len(), steps);
    }
}
