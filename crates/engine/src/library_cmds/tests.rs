use serde_json::json;

use super::*;
use crate::preset_store::{MemBackend, open};

fn doc_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s
}

#[test]
fn colors_are_added_used_renamed_and_deleted() {
    let mut s = Session::new();
    let r = s.execute("library.addColor", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(r["library"], DEFAULT_LIBRARY, "the first item creates My Library");
    assert_eq!(r["item"]["hex"], "#ff0000");
    let id = r["item"]["id"].as_u64().unwrap();
    s.execute("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    s.execute("library.use", json!({"item": id})).unwrap();
    assert_eq!(s.tools.foreground, [1.0, 0.0, 0.0, 1.0]);
    s.execute("library.use", json!({"item": id, "target": "background"})).unwrap();
    assert_eq!(s.tools.background, [1.0, 0.0, 0.0, 1.0]);
    assert!(s.execute("library.use", json!({"item": id, "target": "x"})).is_err());
    // The same name twice gets a number.
    let r2 = s.execute("library.addColor", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(r2["item"]["name"], "#ff0000 2");
    s.execute("library.renameItem", json!({"item": id, "name": "Brand Red"})).unwrap();
    let l = s.execute("library.list", json!({})).unwrap();
    assert_eq!(l["libraries"][0]["items"][0]["name"], "Brand Red");
    s.execute("library.deleteItem", json!({"item": id})).unwrap();
    assert!(s.execute("library.use", json!({"item": id})).is_err());
    assert!(s.execute("library.use", json!({})).is_err());
}

#[test]
fn libraries_are_created_renamed_moved_between_and_deleted() {
    let mut s = Session::new();
    assert_eq!(s.execute("library.new", json!({"name": "Client"})).unwrap()["library"], "Client");
    assert_eq!(s.execute("library.new", json!({"name": "client"})).unwrap()["library"], "client 2");
    let id = s.execute("library.addColor", json!({"library": "Client"})).unwrap()["item"]["id"].as_u64().unwrap();
    assert!(s.execute("library.addColor", json!({"library": "Nope"})).is_err());
    assert!(s.execute("library.rename", json!({"library": "Client", "name": "client 2"})).is_err(), "names stay unique");
    s.execute("library.rename", json!({"library": "Client", "name": "Acme"})).unwrap();
    let r = s.execute("library.moveItem", json!({"item": id, "library": "client 2"})).unwrap();
    assert_eq!(r["moved"], true);
    assert_eq!(s.libraries.list[0].items.len(), 0);
    assert_eq!(s.libraries.list[1].items.len(), 1);
    assert_eq!(s.execute("library.delete", json!({"library": "client 2"})).unwrap()["deleted"], 1);
    assert!(s.execute("library.delete", json!({})).is_err(), "deleting needs a name");
    assert_eq!(s.libraries.list.len(), 1);
}

#[test]
fn a_graphic_from_layers_places_into_another_document() {
    let mut s = doc_session();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 7})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("library.addGraphic", json!({"name": "Logo"})).unwrap();
    let item = &r["item"];
    assert_eq!((item["kind"].as_str(), item["width"].as_u64(), item["height"].as_u64()), (Some("graphic"), Some(10), Some(7)), "{r}");
    let id = item["id"].as_u64().unwrap();
    let (w, h, px) = preview(s.libraries.item(id).unwrap(), 20).unwrap();
    assert_eq!((w, h, px.len()), (10, 7, 10 * 7 * 4), "small graphics are not enlarged");
    assert_eq!(&px[..4], &[0, 255, 0, 255], "the preview shows the green fill");
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    let placed = s.execute("library.use", json!({"item": id})).unwrap();
    let layer = LayerId(placed["layer"].as_u64().unwrap());
    let st = s.active().unwrap();
    let l = st.doc.layer(layer).unwrap();
    assert_eq!(l.name, "Logo");
    assert!(matches!(&l.content, LayerContent::Smart(sm) if matches!(sm.source, photocraft_doc::SmartSource::Embedded { .. })));
    assert!(s.execute("library.use", json!({"item": id, "linked": true})).is_err(), "no library folder in a headless session");
    // Without a document a graphic can't be placed.
    let mut t = Session::new();
    t.libraries = s.libraries.clone();
    assert!(t.execute("library.use", json!({"item": id})).is_err());
}

#[test]
fn patterns_go_to_the_document() {
    let mut s = doc_session();
    let r = s.execute("library.addPattern", json!({})).unwrap();
    assert_eq!(r["item"]["kind"], "pattern");
    let id = r["item"]["id"].as_u64().unwrap();
    let before = s.active().unwrap().doc.patterns.len();
    let used = s.execute("library.use", json!({"item": id})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.patterns.len(), before + 1);
    assert!(doc.patterns.iter().any(|p| p.id == used["pattern"].as_str().unwrap()));
    // Using it again doesn't add a duplicate.
    s.execute("library.use", json!({"item": id})).unwrap();
    assert_eq!(s.active().unwrap().doc.patterns.len(), before + 1);
}

#[test]
fn libraries_persist_in_the_store_and_orphans_are_removed() {
    let mem = MemBackend::default();
    let mut s = doc_session();
    s.attach_preset_store(open(Box::new(mem.clone())));
    s.execute("library.addColor", json!({"color": "#336699", "name": "Navy"})).unwrap();
    let g = s.execute("library.addGraphic", json!({"name": "Bg"})).unwrap()["item"]["id"].as_u64().unwrap();
    let files = |m: &MemBackend| m.files.lock().unwrap().keys().cloned().collect::<Vec<_>>();
    assert!(files(&mem).contains(&INDEX.to_string()));
    let lib_files: Vec<String> = files(&mem).into_iter().filter(|f| is_library_file(f)).collect();
    assert_eq!(lib_files.len(), 1, "{lib_files:?}");

    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    let l = t.execute("library.list", json!({})).unwrap();
    let items = l["libraries"][0]["items"].as_array().unwrap();
    assert_eq!((items[0]["name"].as_str(), items[0]["hex"].as_str()), (Some("Navy"), Some("#336699")));
    assert_eq!(items[1]["kind"], "graphic");
    t.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    let gid = items[1]["id"].as_u64().unwrap();
    t.execute("library.use", json!({"item": gid})).unwrap();

    s.execute("library.deleteItem", json!({"item": g})).unwrap();
    assert!(files(&mem).iter().all(|f| !is_library_file(f)), "the graphic's file goes with it");
}

#[test]
fn hostile_store_files_are_skipped_not_fatal() {
    for index in [
        b"not json".to_vec(),
        br#"{"format":"photocraft-libraries","version":9,"libraries":[]}"#.to_vec(),
        br#"{"format":"photocraft-libraries","version":1,"libraries":[{"name":"A","items":[
            {"name":"x","kind":"graphic","file":"../../etc/passwd"},
            {"name":"y","kind":"graphic","file":"lib-missing.png"},
            {"name":"z","kind":"color","rgb":[1e30,-5,0.5]},
            {"name":"w","kind":"what"}]}]}"#
            .to_vec(),
    ] {
        let mem = MemBackend::default();
        mem.files.lock().unwrap().insert(INDEX.into(), index);
        mem.files.lock().unwrap().insert("lib-orphan.png".into(), vec![1, 2, 3]);
        let mut s = Session::new();
        let warnings = s.attach_preset_store(open(Box::new(mem.clone())));
        assert!(!warnings.is_empty());
        if let Some(Kind::Color { rgb }) = s.libraries.list.first().and_then(|l| l.items.first()).map(|i| &i.kind) {
            assert_eq!(*rgb, [1.0, 0.0, 0.5]);
        }
        // A failed load keeps unreferenced files: a repaired index may still need them.
        s.execute("library.addColor", json!({})).unwrap();
        assert!(mem.files.lock().unwrap().contains_key("lib-orphan.png"));
    }
}

#[test]
fn hostile_params_are_errors() {
    let mut s = doc_session();
    assert!(s.execute("library.use", json!({"item": "x"})).is_err());
    assert!(s.execute("library.use", json!({"item": u64::MAX})).is_err());
    assert!(s.execute("library.renameItem", json!({"item": 1, "name": ""})).is_err());
    assert!(s.execute("library.addFile", json!({})).is_err());
    assert!(s.execute("library.addFile", json!({"path": "/nonexistent/x.png"})).is_err());
    assert!(s.execute("library.addGraphic", json!({"layers": [999_999]})).is_err());
    assert!(s.execute("library.addPattern", json!({"pattern": "no such pattern"})).is_err());
    let long = "n".repeat(10_000);
    let r = s.execute("library.new", json!({"name": long})).unwrap();
    assert_eq!(r["library"].as_str().unwrap().chars().count(), MAX_NAME);
}
