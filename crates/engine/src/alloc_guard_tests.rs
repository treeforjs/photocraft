use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s
}

fn snapshot(s: &Session) -> (usize, u64, usize, bool) {
    let st = s.active().unwrap();
    (s.documents().len(), st.revision, st.doc.layer_count(), s.clipboard.is_some())
}

fn cases() -> Vec<(&'static str, Value)> {
    let out = std::env::temp_dir().join(format!("pc-alloc-{}.png", std::process::id()));
    vec![
        ("edit.fill", json!({"color": "#ff0000"})),
        ("edit.fillForeground", json!({})),
        ("edit.clear", json!({})),
        ("edit.cut", json!({})),
        ("edit.copy", json!({})),
        ("edit.paste", json!({})),
        ("edit.transform.flipHorizontal", json!({})),
        ("image.imageSize", json!({"width": 20, "height": 20})),
        ("image.canvasSize", json!({"width": 30, "height": 30})),
        ("image.crop", json!({"x": 0, "y": 0, "width": 5, "height": 5})),
        ("image.imageRotation.90cw", json!({})),
        ("file.saveACopy", json!({"path": out.to_string_lossy()})),
    ]
}

#[test]
fn a_command_that_cannot_get_its_memory_changes_nothing() {
    for (id, p) in cases() {
        let mut s = session();
        s.alloc_limit = Some(1024);
        let before = snapshot(&s);
        let err = s.execute(id, p.clone()).expect_err(id).to_string();
        assert!(err.contains("not enough memory") && err.contains("nothing was changed"), "{id}: {err}");
        assert_eq!(snapshot(&s), before, "{id} left the session as it was");
        // With room it runs.
        s.alloc_limit = None;
        s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn unguarded_commands_and_small_jobs_pass() {
    let mut s = session();
    s.alloc_limit = Some(1024);
    s.execute("select.deselect", json!({})).unwrap();
    assert!(crate::alloc_guard::estimate(&s, "select.all", &json!({})).is_none());
    s.alloc_limit = Some(1 << 30);
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
}

#[test]
fn an_absurd_size_is_refused_without_panicking() {
    let mut s = session();
    // Past Image Size's own budget the command refuses it with its own advice.
    assert!(crate::alloc_guard::estimate(&s, "image.imageSize", &json!({"width": 1e15, "height": 1e15})).is_none());
    let before = snapshot(&s);
    assert!(s.execute("image.imageSize", json!({"width": 1e15, "height": 1e15})).is_err());
    assert_eq!(snapshot(&s), before);
    // Garbage dimensions fall back to the current size.
    let same = crate::alloc_guard::estimate(&s, "image.imageSize", &json!({"width": -1, "height": "x"})).unwrap();
    assert_eq!(same, crate::alloc_guard::estimate(&s, "image.imageSize", &json!({})).unwrap());
}

#[test]
fn estimates_scale_with_layers_only_for_whole_document_commands() {
    let mut s = session();
    let crop = |s: &Session| crate::alloc_guard::estimate(s, "image.crop", &json!({})).unwrap();
    let fill = |s: &Session| crate::alloc_guard::estimate(s, "edit.fill", &json!({})).unwrap();
    let (c, f) = (crop(&s), fill(&s));
    // An empty layer allocates nothing; painting it does.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(crop(&s), c);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert!(crop(&s) > c);
    assert_eq!(fill(&s), f);
    // Duplicating shares tiles, and a resolution-only Image Size writes no pixels.
    assert!(crate::alloc_guard::estimate(&s, "layer.duplicate", &json!({})).is_none());
    assert!(crate::alloc_guard::estimate(&s, "image.imageSize", &json!({"width": 99, "resample": "none"})).is_none());
}

#[test]
fn a_huge_empty_canvas_still_resizes_and_edits() {
    // Transparent canvas area is sparse: the guard counts allocated tiles, not the canvas.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    s.alloc_limit = Some(64 << 20);
    s.execute("image.canvasSize", json!({"width": 65536, "height": 65536, "extensionColor": "transparent"})).unwrap();
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    // Filling all of it would write 4 billion pixels: refused up front.
    s.execute("select.all", json!({})).unwrap();
    let err = s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap_err().to_string();
    assert!(err.contains("not enough memory"), "{err}");
}
