//! Paintable Smart Filter masks: synthetic sources, transactional edits, live/commit fidelity,
//! and source authority. A fresh source render is the cache oracle; no proprietary fixtures.

use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{BlendMode, Document, Layer, LayerContent, LayerId, LayerMask, SmartFilter, SmartObject, SmartSource};
use photocraft_engine::brush_cmds::{LiveStroke, parse_points};
use photocraft_engine::{EngineError, Session};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const WIDTH: i32 = 48;
const HEIGHT: i32 = 24;

fn fixture(depth: u64, width: i32) -> (Session, LayerId) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": width, "height": HEIGHT, "depth": depth})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Synthetic Smart source"})).unwrap();
    session
        .edit("Synthetic tones and alpha", |doc, active| {
            let bounds = doc.bounds();
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let format = surface.format();
            let mut pixels = Vec::new();
            for y in 0..HEIGHT {
                for x in 0..width {
                    pixels.extend(photocraft_raster::from_rgba(
                        &format,
                        [0.12 + (x % WIDTH) as f32 * 0.01, 0.18 + y as f32 * 0.01, 0.64 - (x % WIDTH) as f32 * 0.007, 0.4 + ((x + y) % 4) as f32 * 0.2],
                    ));
                }
            }
            surface.write_region(bounds, &pixels);
            Ok(())
        })
        .unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    session.execute("image.adjustments.levels", json!({"gamma": 1.7})).unwrap();
    (session, layer)
}

fn smart(doc: &Document, layer: LayerId) -> &SmartObject {
    match &doc.layer(layer).unwrap().content {
        LayerContent::Smart(smart) => smart,
        _ => panic!("expected a Smart Object"),
    }
}

fn smart_mut(doc: &mut Document, layer: LayerId) -> &mut SmartObject {
    match &mut doc.layer_mut(layer).unwrap().content {
        LayerContent::Smart(smart) => smart,
        _ => panic!("expected a Smart Object"),
    }
}

fn stroke(layer: LayerId) -> Value {
    json!({"target": "smartFilterMask", "layer": layer.0, "points": [[8, 12], [24, 12]],
           "size": 8, "hardness": 1, "opacity": 1, "flow": 1, "spacing": 0.15,
           "smoothing": 0, "color": "#000000", "seed": 123})
}

fn max_difference(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
}

fn assert_cache_matches_render(doc: &Document, layer: LayerId) {
    let sm = smart(doc, layer);
    let rendered = photocraft_engine::smart_cmds::render(doc, sm).unwrap().unwrap();
    let cache = sm.cache.as_ref().unwrap();
    let bounds = doc.bounds().union(&rendered.content_bounds()).union(&cache.content_bounds());
    // The full render also mixes untouched white pixels; premultiplication there can add a
    // floating-point rounding step that the bounded cache update correctly skips.
    let difference = max_difference(&cache.read_region(bounds), &rendered.read_region(bounds));
    assert!(difference <= 1e-6, "the mask edit must update the displayed Smart cache: difference={difference}");
}

fn assert_rejected(session: &mut Session, command: &str, params: Value) -> String {
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let history = session.active().unwrap().history.past_len();
    let active = session.active().unwrap().active_layer;
    let error = session.execute(command, params).unwrap_err().to_string();
    // Pointer identity also handles a malformed document containing a NaN mask feather.
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc), "{command}: no document replacement on rejection");
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    assert_eq!(session.active().unwrap().active_layer, active);
    error
}

#[test]
fn first_stroke_creates_a_same_depth_white_mask_and_roundtrips_natively() {
    for depth in [8, 16, 32] {
        let (mut session, layer) = fixture(depth, WIDTH);
        let before = session.active().unwrap().doc.clone();
        let original = smart(&before, layer);
        assert!(original.filter_mask.is_none());
        let placed = photocraft_engine::smart_cmds::render_below_filter(&before, original, 0).unwrap().unwrap();
        let history = session.active().unwrap().history.past_len();
        let params = stroke(layer);
        assert!(session.is_enabled_with("paint.stroke", &params));
        session.execute("paint.stroke", params).unwrap();
        let after = session.active().unwrap().doc.clone();
        let edited = smart(&after, layer);
        let mask = edited.filter_mask.as_ref().unwrap();
        assert_eq!(mask.surface.format(), PixelFormat::new(ColorMode::Grayscale, before.depth, false));
        assert_eq!(mask.surface.default_pixel(), vec![1.0], "untouched and off-canvas pixels reveal all filters");
        assert_eq!(mask.surface.pixel(12, 12), vec![0.0]);
        assert_eq!(mask.surface.pixel(40, 4), vec![1.0]);
        assert_eq!(mask.surface.pixel(-10, -10), vec![1.0]);
        assert_eq!((mask.enabled, mask.linked, mask.density, mask.feather), (true, true, 1.0, 0.0));
        assert!(max_difference(&edited.cache.as_ref().unwrap().pixel(12, 12), &placed.pixel(12, 12)) <= 1e-6, "black reveals the unfiltered source");
        assert_ne!(edited.cache.as_ref().unwrap().pixel(12, 12), original.cache.as_ref().unwrap().pixel(12, 12));
        assert_eq!(edited.cache.as_ref().unwrap().pixel(40, 4), original.cache.as_ref().unwrap().pixel(40, 4));
        assert_eq!((&edited.source, edited.transform, &edited.smart_filters), (&original.source, original.transform, &original.smart_filters));
        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
        assert_cache_matches_render(&after, layer);

        session.undo();
        assert_eq!(*session.active().unwrap().doc, *before, "mask creation and its stroke undo together");
        session.redo();
        assert_eq!(*session.active().unwrap().doc, *after);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(smart(&reopened, layer), edited, "{depth}-bit mask, parameters, source and cache persist");
        assert_cache_matches_render(&reopened, layer);
    }
}

#[test]
fn selection_limits_mask_painting_and_eraser_paints_the_background() {
    for depth in [8, 16, 32] {
        let (mut session, layer) = fixture(depth, WIDTH);
        session.tools.background = [1.0; 4];
        session
            .edit("Existing mask density fixture", |doc, _| {
                let format = PixelFormat::new(ColorMode::Grayscale, doc.depth, false);
                smart_mut(doc, layer).filter_mask =
                    Some(LayerMask { surface: Surface::with_default(format, &[1.0]), density: 0.5, linked: false, ..LayerMask::reveal_all() });
                Ok(())
            })
            .unwrap();
        session.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": HEIGHT})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let mut params = stroke(layer);
        session.execute("paint.stroke", params.clone()).unwrap();
        let painted = session.active().unwrap().doc.clone();
        let mask = smart(&painted, layer).filter_mask.as_ref().unwrap();
        assert_eq!(mask.surface.pixel(12, 12), vec![0.0]);
        assert_eq!(mask.surface.pixel(20, 12), vec![1.0], "the active selection clips the stroke");
        assert_eq!((mask.enabled, mask.linked, mask.density, mask.feather), (true, false, 0.5, 0.0), "painting preserves existing mask controls");
        assert_cache_matches_render(&painted, layer);

        params["erase"] = json!(true);
        session.execute("paint.stroke", params).unwrap();
        let erased = session.active().unwrap().doc.clone();
        let mask = smart(&erased, layer).filter_mask.as_ref().unwrap();
        assert_eq!(mask.surface.pixel(12, 12), vec![1.0], "mask erasure restores background white instead of transparency");
        assert_eq!(mask.surface.pixel(20, 12), vec![1.0]);
        assert_eq!((mask.enabled, mask.linked, mask.density, mask.feather), (true, false, 0.5, 0.0));
        assert!(
            max_difference(&smart(&erased, layer).cache.as_ref().unwrap().pixel(12, 12), &smart(&before, layer).cache.as_ref().unwrap().pixel(12, 12)) <= 1e-6
        );
        assert_eq!(erased.selection, before.selection);
        assert_eq!(smart(&erased, layer).source, smart(&before, layer).source);
        assert_cache_matches_render(&erased, layer);
        session.undo();
        assert_eq!(*session.active().unwrap().doc, *painted);
        session.redo();
        assert_eq!(*session.active().unwrap().doc, *erased);
    }
}

#[test]
fn brush_and_pencil_live_previews_match_one_committed_undo_step_at_every_depth() {
    for depth in [8, 16, 32] {
        for command in ["paint.stroke", "paint.pencil"] {
            let (mut session, layer) = fixture(depth, WIDTH);
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let history = session.active().unwrap().history.past_len();
            let mut params = stroke(layer);
            params["points"] = json!([[8, 12]]);
            params["opacity"] = json!(0.65);
            params["smoothing"] = json!(0.25);
            let mut live = LiveStroke::begin_with(&session, command, &params).unwrap();
            let more = json!({"points": [[14, 9], [22, 13], [30, 11]]});
            live.push(&parse_points(&more, command).unwrap()).unwrap();
            assert_cache_matches_render(&live.doc, layer);
            assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc), "a cancellable preview never publishes its document");
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.past_len(), history);

            params["points"] = json!([[8, 12], [14, 9], [22, 13], [30, 11]]);
            params["seed"] = json!(live.seed);
            session.execute(command, params).unwrap();
            let after = session.active().unwrap().doc.clone();
            assert_eq!(*after, *live.doc, "{command}, {depth}-bit: release must preserve the exact preview");
            assert_eq!(session.active().unwrap().history.past_len(), history + 1);
            session.undo();
            assert_eq!(*session.active().unwrap().doc, *before);
            session.redo();
            assert_eq!(*session.active().unwrap().doc, *after);
        }
    }
}

#[test]
fn explicit_nonactive_smart_mask_target_does_not_follow_the_selected_layer() {
    let (mut session, layer) = fixture(16, WIDTH);
    session.execute("layer.new.layer", json!({"name": "Remain selected"})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let active = session.active().unwrap().active_layer.unwrap();
    let live = LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).unwrap();
    session.execute("paint.stroke", stroke(layer)).unwrap();
    let after = &session.active().unwrap().doc;
    assert_eq!(session.active().unwrap().active_layer, Some(active));
    assert_eq!(after.layer(active), before.layer(active));
    assert_eq!(**after, *live.doc);
    assert!(smart(after, layer).filter_mask.as_ref().unwrap().surface.pixel(12, 12)[0] < 1.0);
    assert_cache_matches_render(after, layer);
}

#[test]
fn inherited_pixel_and_all_locks_reject_live_and_committed_mask_strokes() {
    for inherited in [false, true] {
        for all in [false, true] {
            let (mut session, layer) = fixture(8, WIDTH);
            session
                .edit("Synthetic lock", |doc, _| {
                    let locked = if inherited {
                        let index = doc.layers.iter().position(|candidate| candidate.id == layer).unwrap();
                        let child = doc.layers.remove(index);
                        let group = Layer::group("Locked parent", vec![child]);
                        let id = group.id;
                        doc.layers.insert(index, group);
                        id
                    } else {
                        layer
                    };
                    doc.layer_mut(locked).unwrap().locks.all = all;
                    doc.layer_mut(locked).unwrap().locks.pixels = !all;
                    Ok(())
                })
                .unwrap();
            for command in ["paint.stroke", "paint.pencil"] {
                assert!(LiveStroke::begin_with(&session, command, &stroke(layer)).is_err());
                assert_rejected(&mut session, command, stroke(layer));
            }
        }
    }
}

#[test]
fn invalid_strokes_and_feathered_masks_leave_the_cache_and_history_untouched() {
    let (mut session, layer) = fixture(8, WIDTH);
    for patch in [json!({"points": []}), json!({"points": [[1e30, 12]]}), json!({"size": 1e30}), json!({"layer": 0})] {
        let mut params = stroke(layer);
        params.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        assert!(LiveStroke::begin_with(&session, "paint.stroke", &params).is_err());
        assert_rejected(&mut session, "paint.stroke", params);
    }
    for feather in [2.0, -1.0, f32::NAN, f32::INFINITY] {
        let (mut session, layer) = fixture(8, WIDTH);
        session
            .edit("Synthetic feather", |doc, _| {
                smart_mut(doc, layer).filter_mask = Some(LayerMask { feather, ..LayerMask::reveal_all() });
                Ok(())
            })
            .unwrap();
        assert!(LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).is_err(), "feather={feather:?}");
        assert_rejected(&mut session, "paint.stroke", stroke(layer));
    }
}

#[test]
fn unsupported_target_commands_cannot_modify_a_smart_mask_or_its_layer() {
    let (mut session, layer) = fixture(8, WIDTH);
    session.execute("paint.stroke", stroke(layer)).unwrap();
    for (command, mut params) in [
        ("paint.gradient", json!({"from": [0, 0], "to": [WIDTH, HEIGHT], "colors": ["#000000", "#ffffff"]})),
        ("filter.blur.gaussianBlur", json!({"radius": 2})),
        ("image.adjustments.levels", json!({"gamma": 0.5})),
        ("paint.blur", stroke(layer)),
        ("paint.smudge", stroke(layer)),
        ("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 3]})),
    ] {
        params["target"] = json!("smartFilterMask");
        params["layer"] = json!(layer.0);
        assert_rejected(&mut session, command, params);
    }
}

fn add_unsupported(session: &mut Session, layer: LayerId, visible: bool, enabled: bool) {
    session
        .edit("Synthetic imported unsupported filter", |doc, _| {
            let sm = smart_mut(doc, layer);
            sm.filters_enabled = enabled;
            sm.smart_filters.push(SmartFilter {
                command: photocraft_engine::smart_cmds::UNSUPPORTED_FILTER.into(),
                params: json!({"name": "Synthetic preserved filter", "psd": "00000000"}),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible,
            });
            Ok(())
        })
        .unwrap();
}

#[test]
fn missing_sources_and_visible_unsupported_filters_preserve_imported_caches() {
    for missing_source in [false, true] {
        let (mut session, layer) = fixture(8, WIDTH);
        if missing_source {
            session
                .edit("Synthetic missing linked source", |doc, _| {
                    smart_mut(doc, layer).source = SmartSource::Linked { path: "/__photocraft_synthetic_missing_source__/mask.pcraft".into() };
                    Ok(())
                })
                .unwrap();
        } else {
            add_unsupported(&mut session, layer, true, true);
        }
        assert!(LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).is_err());
        assert_rejected(&mut session, "paint.stroke", stroke(layer));
    }
}

#[test]
fn hidden_unknown_filters_and_disabled_stacks_keep_their_descriptors_when_painted() {
    for (visible, enabled) in [(false, true), (true, false)] {
        let (mut session, layer) = fixture(8, WIDTH);
        add_unsupported(&mut session, layer, visible, enabled);
        // This fixture models the cache of an already disabled stack, not a stale cache from
        // directly toggling the data field above.
        session
            .edit("Synthetic current stack cache", |doc, _| {
                assert!(photocraft_engine::smart_cmds::refresh(doc, layer)?);
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let live = LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).unwrap();
        session.execute("paint.stroke", stroke(layer)).unwrap();
        let after = &session.active().unwrap().doc;
        assert_eq!(**after, *live.doc);
        assert_eq!(smart(after, layer).smart_filters, smart(&before, layer).smart_filters);
        assert_eq!(smart(after, layer).filters_enabled, enabled);
        assert_cache_matches_render(after, layer);
    }
}

fn deny_linked_source(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    if params.get("path").is_some() {
        assert_eq!(command, "layer.smartFilter.setParams", "linked source authority uses the canonical Smart edit command");
        Err(EngineError::Other("synthetic mask source denied".into()))
    } else {
        Ok(())
    }
}

fn deny_filter_input(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    if params.get("mapPath").is_some() {
        assert_eq!(command, "filter.distort.displace", "stored filter inputs retain their own command policy");
        Err(EngineError::Other("synthetic mask filter input denied".into()))
    } else {
        Ok(())
    }
}

#[test]
fn stored_sources_and_filter_inputs_are_authorized_before_preview_or_commit_reads() {
    for external_filter in [false, true] {
        let (mut session, layer) = fixture(8, WIDTH);
        session
            .edit("Synthetic denied external input", |doc, _| {
                let sm = smart_mut(doc, layer);
                if external_filter {
                    sm.smart_filters.push(SmartFilter {
                        command: "filter.distort.displace".into(),
                        params: json!({"mapPath": "/__photocraft_synthetic_missing_source__/map.png"}),
                        blend: BlendMode::Normal,
                        opacity: 1.0,
                        visible: true,
                    });
                } else {
                    sm.source = SmartSource::Linked { path: "/__photocraft_synthetic_missing_source__/mask.pcraft".into() };
                }
                Ok(())
            })
            .unwrap();
        // Keep a different layer active: source authorization must inspect the explicit target.
        session.execute("layer.new.layer", json!({"name": "Unrelated selected layer"})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let authorize: fn(&str, &Value) -> photocraft_engine::Result<()> = if external_filter { deny_filter_input } else { deny_linked_source };
        let expected = if external_filter { "synthetic mask filter input denied" } else { "synthetic mask source denied" };
        let error = LiveStroke::begin_with_authorize(&session, "paint.stroke", &stroke(layer), Some(authorize)).err().unwrap();
        assert!(error.to_string().contains(expected), "authorize before trying to resolve the nonexistent input: {error}");
        assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
        session.authorize = Some(authorize);
        let error = LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).err().unwrap();
        assert!(error.to_string().contains(expected));
        let error = assert_rejected(&mut session, "paint.stroke", stroke(layer));
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn live_and_committed_remixing_preserve_untouched_far_cache_tiles() {
    let (mut session, layer) = fixture(16, 544);
    let before = session.active().unwrap().doc.clone();
    let far = TileCoord::new(2, 0);
    let original = smart(&before, layer).cache.as_ref().unwrap().tile(far).unwrap();
    let mut live = LiveStroke::begin_with(&session, "paint.stroke", &stroke(layer)).unwrap();
    assert!(Arc::ptr_eq(original, smart(&live.doc, layer).cache.as_ref().unwrap().tile(far).unwrap()));
    live.push(&parse_points(&json!({"points": [[30, 12]]}), "paint.stroke").unwrap()).unwrap();
    assert!(Arc::ptr_eq(original, smart(&live.doc, layer).cache.as_ref().unwrap().tile(far).unwrap()), "new dabs remix only their damage");
    let mut params = stroke(layer);
    params["points"] = json!([[8, 12], [24, 12], [30, 12]]);
    params["seed"] = json!(live.seed);
    session.execute("paint.stroke", params).unwrap();
    let after = &session.active().unwrap().doc;
    assert!(Arc::ptr_eq(original, smart(after, layer).cache.as_ref().unwrap().tile(far).unwrap()), "commit also preserves untouched cache tiles");
    assert_eq!(**after, *live.doc);
    assert_cache_matches_render(after, layer);
}

#[test]
fn default_mask_paint_survives_psd_at_all_depths() {
    for depth in [8, 16, 32] {
        let (mut session, layer) = fixture(depth, WIDTH);
        session.execute("paint.stroke", stroke(layer)).unwrap();
        let doc = &session.active().unwrap().doc;
        let exported = photocraft_io::export(doc, "psd", &Default::default()).unwrap();
        assert!(exported.warnings.is_empty(), "{depth}-bit: {:?}", exported.warnings);
        let parsed = photocraft_psd::PsdFile::from_bytes(&exported.bytes).unwrap();
        for block in &parsed.global_blocks {
            block.check_structure().unwrap();
        }
        for layer in parsed.layers() {
            for block in &layer.blocks {
                block.check_structure().unwrap();
            }
        }
        let reopened = photocraft_io::import("synthetic-painted-filter-mask.psd", &exported.bytes).unwrap().document;
        let imported = reopened
            .walk()
            .into_iter()
            .find_map(|(_, _, layer)| match &layer.content {
                LayerContent::Smart(sm) => Some((layer.id, sm)),
                _ => None,
            })
            .unwrap();
        let actual = imported.1.filter_mask.as_ref().unwrap();
        let expected = smart(doc, layer).filter_mask.as_ref().unwrap();
        let area = Rect::new(-8, -8, WIDTH + 8, HEIGHT + 8);
        assert_eq!(actual.surface.read_region(area), expected.surface.read_region(area), "{depth}-bit painted FEid mask and white extension");
        assert_eq!((actual.enabled, actual.linked, actual.density, actual.feather), (expected.enabled, expected.linked, expected.density, expected.feather));
        let rendered = photocraft_engine::smart_cmds::render(&reopened, imported.1).unwrap().unwrap();
        let expected = smart(doc, layer).cache.as_ref().unwrap();
        let difference = max_difference(&rendered.read_region(doc.bounds()), &expected.read_region(doc.bounds()));
        assert!(difference <= 2.0 / 255.0 + 1e-6, "{depth}-bit source-based PSD render differs by {difference}");
    }
}
