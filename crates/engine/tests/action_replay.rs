//! Recorded actions replay on a second document exactly as the same commands run there by hand
//! (AUTO-219-4): crop, filters, selections and styled text. The second document's layer ids
//! differ from the first's, so steps that name a layer must replay by reference.

use photocraft_engine::Session;
use serde_json::{Value, json};

/// The document an action is recorded on: a background and an "Art" layer with a stroke.
fn doc_a(s: &mut Session) {
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Art"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[6, 10], [50, 30]], "size": 9, "hardness": 1.0, "color": "#2060ff"})).unwrap();
}

/// The document it replays on: the same picture, but built so its layer ids differ.
fn doc_b(s: &mut Session) {
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for name in ["Spare 1", "Spare 2", "Spare 3"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        s.execute("layer.delete", json!({})).unwrap();
    }
    s.execute("layer.new.layer", json!({"name": "Art"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[6, 10], [50, 30]], "size": 9, "hardness": 1.0, "color": "#2060ff"})).unwrap();
}

fn layer_id(s: &Session, name: &str) -> u64 {
    let st = s.active().unwrap();
    st.doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).unwrap().id.0
}

/// Everything a replay must reproduce: size, layer names, the flattened picture and the
/// selection mask.
#[derive(Debug)]
struct Snapshot {
    size: (u32, u32),
    names: Vec<String>,
    picture: Vec<[f32; 4]>,
    selection: Option<Vec<f32>>,
}

fn snapshot(s: &Session) -> Snapshot {
    let st = s.active().unwrap();
    let doc = &st.doc;
    let canvas = photocraft_geom::Rect::new(0, 0, doc.size.width as i32, doc.size.height as i32);
    Snapshot {
        size: (doc.size.width, doc.size.height),
        names: doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect(),
        picture: photocraft_compose::flatten(doc).px,
        selection: doc.selection.as_ref().map(|m| m.read_region(canvas)),
    }
}

fn assert_same(got: &Snapshot, want: &Snapshot, what: &str) {
    assert_eq!(got.size, want.size, "{what}: size");
    assert_eq!(got.names, want.names, "{what}: layers");
    assert_eq!(got.picture.len(), want.picture.len(), "{what}: picture size");
    let worst = got.picture.iter().zip(&want.picture).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())).fold(0.0f32, f32::max);
    assert!(worst < 1e-5, "{what}: pictures differ by {worst}");
    assert_eq!(got.selection.is_some(), want.selection.is_some(), "{what}: selection presence");
    if let (Some(a), Some(b)) = (&got.selection, &want.selection) {
        let worst = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "{what}: selections differ by {worst}");
    }
}

/// Records `steps` on document A, plays the action on document B, and compares that with
/// `steps` run directly on a fresh document B. Returns the replaying session.
fn replay(what: &str, steps: impl Fn(&mut Session)) -> Session {
    let mut rec = Session::new();
    doc_a(&mut rec);
    rec.execute("actions.record", json!({"name": what})).unwrap();
    steps(&mut rec);
    rec.execute("actions.stop", json!({})).unwrap();
    let recorded = &rec.actions.list[0].steps;
    assert!(!recorded.is_empty(), "{what}: nothing was recorded");
    let a_ids: Vec<u64> = rec.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id.0).collect();

    doc_b(&mut rec);
    let b_ids: Vec<u64> = rec.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id.0).collect();
    assert_ne!(a_ids, b_ids, "{what}: the documents must not share layer ids");
    let r = rec.execute("actions.play", json!({"action": what})).unwrap();
    assert!(r.get("failed").is_none(), "{what}: {r}");

    let mut direct = Session::new();
    doc_b(&mut direct);
    steps(&mut direct);
    assert_same(&snapshot(&rec), &snapshot(&direct), what);
    rec
}

#[test]
fn crop_replays_on_a_second_document() {
    replay("Crop", |s| {
        s.execute("image.crop", json!({"x": 4, "y": 6, "width": 40, "height": 30})).unwrap();
    });
    // A crop that keeps the cropped pixels, through a selection.
    replay("Crop to selection", |s| {
        s.execute("select.rect", json!({"x": 10, "y": 8, "width": 30, "height": 20})).unwrap();
        s.execute("image.crop", json!({"deleteCroppedPixels": false})).unwrap();
    });
}

#[test]
fn filters_replay_on_a_second_document() {
    replay("Filters", |s| {
        s.execute("filter.blur.gaussianBlur", json!({"radius": 2.5})).unwrap();
        s.execute("filter.blur.motionBlur", json!({"angle": 30, "distance": 6})).unwrap();
        s.execute("filter.distort.twirl", json!({"angle": 90})).unwrap();
    });
    // A filter aimed at a layer other than the active one replays on that layer.
    replay("Filter on Art", |s| {
        let art = layer_id(s, "Art");
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s.execute("filter.other.maximum", json!({"layer": art, "radius": 2})).unwrap();
    });
}

#[test]
fn selections_replay_on_a_second_document() {
    replay("Selections", |s| {
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 30, "height": 20})).unwrap();
        s.execute("select.lasso", json!({"points": [[20, 10], [60, 12], [40, 44]], "mode": "add"})).unwrap();
        s.execute("select.modify.feather", json!({"radius": 2})).unwrap();
        s.execute("select.inverse", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
    });
    replay("Expand and contract", |s| {
        s.execute("select.rect", json!({"x": 12, "y": 12, "width": 20, "height": 16})).unwrap();
        s.execute("select.modify.expand", json!({"radius": 3})).unwrap();
        s.execute("select.modify.contract", json!({"radius": 1})).unwrap();
    });
}

#[test]
fn styled_text_replays_on_a_second_document() {
    let s = replay("Type", |s| {
        let id = s.execute("type.create", json!({"x": 4, "y": 30, "text": "Ho"})).unwrap()["layer"].as_u64().unwrap();
        // Keystrokes, as the type tool sends them.
        for (at, text) in [(2, "l"), (3, "a")] {
            s.execute("type.edit", json!({"layer": id, "replace": {"start": at, "end": at, "text": text}, "coalesce": "typing"})).unwrap();
        }
        s.execute("type.setStyle", json!({"layer": id, "range": [0, 2], "size": 24, "color": "#c02020"})).unwrap();
        s.execute("type.setStyle", json!({"layer": id, "range": [2, 4], "italic": true, "tracking": 80})).unwrap();
    });
    let info: Value = {
        let mut s = s;
        s.execute("type.info", json!({})).unwrap()
    };
    assert_eq!(info["text"], "Hola");
}
