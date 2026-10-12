//! Opt-in release measurement: source/filter preparation once versus a full render per dab.

use std::time::{Duration, Instant};

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, parse_points};
use serde_json::json;

#[test]
#[ignore = "24 MP release performance measurement; run explicitly with --release --ignored --nocapture"]
fn prepared_smart_mask_dabs_avoid_repeated_full_image_filtering_24mp() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 6000, "height": 4000, "background": "transparent"})).unwrap();
    session.execute("edit.fill", json!({"color": "#405080"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.execute("image.adjustments.levels", json!({"gamma": 1.7})).unwrap();
    session.execute("layer.smartFilter.blendingOptions", json!({"opacity": 0.5})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    let params = json!({"target": "smartFilterMask", "layer": layer.0, "points": [[2900, 2000]], "size": 30,
                       "hardness": 1, "smoothing": 0, "color": "#000000", "seed": 42});
    // The fixture has already rendered and populated the decoded-source cache. Preparation
    // still performs the real full filter/blend computation and original-cache comparison.
    let start = Instant::now();
    let mut live = LiveStroke::begin(&session, &params).unwrap();
    let preparation = start.elapsed();
    let mut local_dabs = Duration::ZERO;
    let mut full_renders = Duration::ZERO;
    let mut final_reference = None;
    for step in 1..=12 {
        let points = parse_points(&json!({"points": [[2900 + step * 8, 2000 + step * 2]]}), "paint.stroke").unwrap();
        let start = Instant::now();
        live.push(&points).unwrap();
        local_dabs += start.elapsed();
        let LayerContent::Smart(smart) = &live.doc.layer(layer).unwrap().content else { panic!("Smart fixture") };
        let start = Instant::now();
        final_reference = photocraft_engine::smart_cmds::render(&live.doc, smart).unwrap();
        full_renders += start.elapsed();
    }
    let LayerContent::Smart(smart) = &live.doc.layer(layer).unwrap().content else { panic!("Smart fixture") };
    assert!(smart.cache.as_ref() == final_reference.as_ref(), "prepared dabs must match the full 24 MP render oracle");
    println!("smart-mask 24MP preparation={preparation:?} local_12_dabs={local_dabs:?} reference_12_full_renders={full_renders:?}");
    assert!(local_dabs * 2 < full_renders, "prepared local dabs should take less than half the full-render reference time");
}
