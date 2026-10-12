//! A mask stroke rebases stale/imported or externally changed linked appearances once, then
//! updates local damage. HDR pixels remain unbounded through the mask's interpolation.

use photocraft_doc::{Document, LayerContent, LayerId, SmartObject, SmartSource};
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, parse_points};
use serde_json::json;

fn fixture(depth: u64) -> (Session, LayerId) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 520, "height": 24, "background": "transparent", "depth": depth})).unwrap();
    session.execute("edit.fill", json!({"color": "#405080"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.execute("image.adjustments.levels", json!({"gamma": 1.7})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    (session, layer)
}

fn smart(doc: &Document, layer: LayerId) -> &SmartObject {
    match &doc.layer(layer).unwrap().content {
        LayerContent::Smart(smart) => smart,
        _ => panic!("Smart fixture"),
    }
}

fn stroke(layer: LayerId) -> serde_json::Value {
    json!({"target":"smartFilterMask","layer":layer.0,"points":[[12,12]],"size":8,"hardness":1,"smoothing":0,"color":"#000000","seed":27})
}

fn assert_render(doc: &Document, layer: LayerId) {
    let object = smart(doc, layer);
    let expected = photocraft_engine::smart_cmds::render(doc, object).unwrap().unwrap();
    let cache = object.cache.as_ref().unwrap();
    let bounds = doc.bounds().union(&expected.content_bounds()).union(&cache.content_bounds());
    let max = cache.read_region(bounds).iter().zip(expected.read_region(bounds)).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(max < 1e-6, "cache must equal a coherent full source/filter render; max={max}");
}

fn check_rebase(session: &mut Session, layer: LayerId) {
    let before = session.active().unwrap().doc.clone();
    let steps = session.active().unwrap().history.past_len();
    let mut params = stroke(layer);
    let mut live = LiveStroke::begin(session, &params).unwrap();
    let old_bounds = smart(&before, layer).cache.as_ref().unwrap().content_bounds();
    assert!(live.bounds().contains_rect(&before.bounds().union(&old_bounds)), "first refresh invalidates the old and newly rendered image bounds");
    if old_bounds.x1 > before.bounds().x1 {
        assert_eq!(smart(&live.doc, layer).cache.as_ref().unwrap().pixel(800, 12), vec![0.0; 4], "rebasing removes stale off-canvas cache pixels");
    }
    assert_render(&live.doc, layer);
    assert_ne!(smart(&live.doc, layer).cache.as_ref().unwrap().pixel(450, 12), smart(&before, layer).cache.as_ref().unwrap().pixel(450, 12));
    let points = parse_points(&json!({"points":[[18,12]]}), "paint.stroke").unwrap();
    let damage = live.push(&points).unwrap();
    assert!(damage.width() < 100, "later dabs report local damage, got {damage:?}");
    assert_render(&live.doc, layer);
    assert_eq!(session.active().unwrap().doc, before, "cancel keeps the original imported/cached document");
    assert_eq!(session.active().unwrap().history.past_len(), steps);
    let expected = live.doc.clone();
    drop(live);
    params["points"] = json!([[12, 12], [18, 12]]);
    session.execute("paint.stroke", params).unwrap();
    assert_eq!(session.active().unwrap().doc, expected);
    assert_eq!(session.active().unwrap().history.past_len(), steps + 1);
    session.undo();
    assert_eq!(session.active().unwrap().doc, before);
}

#[test]
fn stale_imported_style_cache_is_rebased_once_before_local_mask_dabs() {
    let (mut session, layer) = fixture(8);
    session
        .edit("Synthetic imported cached appearance", |doc, _| {
            let bounds = doc.bounds();
            let LayerContent::Smart(smart) = &mut doc.layer_mut(layer).unwrap().content else { panic!("Smart fixture") };
            let cache = smart.cache.as_mut().unwrap();
            cache.write_region(bounds, &[0.9, 0.1, 0.2, 1.0].repeat((bounds.width() * bounds.height()) as usize));
            cache.write_region(photocraft_geom::Rect::new(800, 12, 801, 13), &[0.9, 0.1, 0.2, 1.0]);
            Ok(())
        })
        .unwrap();
    check_rebase(&mut session, layer);
}

#[test]
fn externally_changed_linked_source_is_rebased_coherently_and_cancel_preserves_old_cache() {
    let (mut session, layer) = fixture(8);
    let object = smart(&session.active().unwrap().doc, layer);
    let SmartSource::Embedded { file_name, bytes } = &object.source else { panic!("embedded fixture") };
    let mut source = photocraft_engine::smart_cmds::decode_source(file_name, bytes).unwrap();
    let path = std::env::temp_dir().join(format!("photocraft-mask-rebase-{}-{}.pcraft", std::process::id(), layer.0));
    std::fs::write(&path, &**bytes).unwrap();
    session
        .edit("Linked fixture", |doc, _| {
            let LayerContent::Smart(smart) = &mut doc.layer_mut(layer).unwrap().content else { panic!("Smart fixture") };
            smart.source = SmartSource::Linked { path: path.to_string_lossy().into_owned() };
            Ok(())
        })
        .unwrap();
    let bounds = source.bounds();
    for layer in &mut source.layers {
        if let Some(surface) = layer.surface_mut() {
            surface.write_region(bounds, &[0.1, 0.7, 0.2, 1.0].repeat((bounds.width() * bounds.height()) as usize));
        }
    }
    std::fs::write(&path, photocraft_format::save_to_bytes(&source, &Default::default()).unwrap()).unwrap();
    check_rebase(&mut session, layer);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hdr_mask_interpolation_preserves_negative_and_above_one_color_channels() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"depth":32,"background":"transparent"})).unwrap();
    session
        .edit("HDR fixture", |doc, active| {
            let bounds = doc.bounds();
            doc.layer_mut(active.unwrap())
                .unwrap()
                .surface_mut()
                .unwrap()
                .write_region(bounds, &[2.0, -0.25, 0.5, 0.5].repeat((bounds.width() * bounds.height()) as usize));
            Ok(())
        })
        .unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.execute("image.adjustments.levels", json!({"gamma":1.7})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    let before = session.active().unwrap().doc.clone();
    let base = photocraft_engine::smart_cmds::render_below_filter(&before, smart(&before, layer), 0).unwrap().unwrap();
    let pixel = base.pixel(12, 12);
    assert!(pixel[0] > 1.0 && pixel[1] < 0.0, "fixture must exercise HDR source pixels");
    session.execute("paint.stroke", stroke(layer)).unwrap();
    let after = &session.active().unwrap().doc;
    let actual = smart(after, layer).cache.as_ref().unwrap().pixel(12, 12);
    assert!(actual.iter().zip(pixel).all(|(a, b)| (a - b).abs() < 1e-6), "black mask must restore the exact HDR source, got {actual:?}");
    assert_render(after, layer);
}
