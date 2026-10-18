//! Preset files opened like documents (File › Open, drag-and-drop, the Brushes panel's
//! Import Brushes…, the Preset Manager's Load…): Photoshop brushes (`.abr`) go to the brush
//! library through `brush.presets.importAbr`, gradients (`.grd`) to the Gradients panel through
//! `gradient.presets.importGrd`, swatches (`.aco`, `.ase`) to the Swatches panel through
//! `swatches.import`, keyboard shortcut sets (`.kys`) to Edit › Keyboard Shortcuts through
//! [`crate::kys_import`], instead of opening as documents.

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Extensions handled here rather than by the document importer (`.aco`/`.ase` swatches go to
/// the Swatches panel through [`crate::swatches_ui::import_bytes`]).
/// `.psp` is Photoshop's live `Keyboard Shortcuts.psp` (the same XML as a `.kys`); other `.psp`
/// settings files are refused with a clear message.
pub const PRESET_EXTS: &[&str] = &["abr", "grd", "aco", "ase", "kys", "psp"];

fn ext(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Is `name` a preset file this module imports?
pub fn is_preset_file(name: &str) -> bool {
    PRESET_EXTS.contains(&ext(name).as_str())
}

/// Command params for the file: a real path keeps the journal small; browser files arrive as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import a preset file; `None` when `name` isn't one.
pub fn open(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let (cmd, what) = match ext(name).as_str() {
        "abr" => ("brush.presets.importAbr", "brushes"),
        "grd" => ("gradient.presets.importGrd", "gradients"),
        "aco" | "ase" => return Some(crate::swatches_ui::import_bytes(app, name, bytes, false).map(|_| ())),
        "kys" | "psp" => return Some(crate::kys_import::open_bytes(app, name, bytes).map(|_| ())),
        _ => return None,
    };
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Imported").into());
    let mut p = source(name, bytes);
    p["group"] = json!(stem);
    if what == "brushes" {
        p["select"] = json!(true);
    }
    Some(app.run(cmd, p).map(|r| {
        // A background import (#210): `jobs_ui` reports the count when it lands.
        if r.get("pending").and_then(Value::as_bool) == Some(true) {
            app.ui.status_error = false;
            app.ui.status = format!("Importing {what} from {stem}…");
            return;
        }
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
        app.ui.status_error = false;
        app.ui.status = match warnings.first() {
            Some(w) => format!("Imported {n} {what} from {stem} ({} notes: {w})", warnings.len()),
            None => format!("Imported {n} {what} from {stem}"),
        };
        if what == "brushes" {
            // Show them: Brush Settings › Brushes tab.
            app.ui.panels.brush_settings = true;
            app.ui.brush_tab = 1;
        }
    }))
}

/// Image files a brush tip can be made from.
pub const IMAGE_BRUSH_EXTS: &[&str] = &["png", "jpg", "jpeg", "psd", "psb", "tif", "tiff", "webp", "bmp", "gif", "pcraft"];

/// Brushes panel › Import Image as Brush…: pick an image; its dark, opaque pixels become a new
/// brush preset's tip (`brush.defineFromImage`), shown in Brush Settings › Brushes.
pub fn import_image_brush(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.pick_file_bytes_filtered(IMAGE_BRUSH_EXTS, |app, name, bytes| image_brush_from(app, &name, &bytes))
}

/// [`import_image_brush`] once the file is read.
pub fn image_brush_from(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let mut p = source(name, bytes);
    if p.get("data").is_some() {
        p["fileName"] = json!(name);
        if let Some(stem) = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().trim().to_string()).filter(|s| !s.is_empty()) {
            p["name"] = json!(stem);
        }
    }
    let r = app.run("brush.defineFromImage", p)?;
    app.ui.status_error = false;
    app.ui.status = crate::i18n::fmt(tl!("New brush “{name}” from the image"), &[("name", r["name"].as_str().unwrap_or_default())]);
    app.ui.panels.brush_settings = true;
    app.ui.brush_tab = 1;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_preset_files() {
        assert!(is_preset_file("/a/b/Set.ABR"));
        assert!(is_preset_file("sunsets.grd"));
        assert!(is_preset_file("Brand.ACO"));
        assert!(is_preset_file("exchange.ase"));
        assert!(!is_preset_file("photo.psd"));
        assert!(!is_preset_file("abr"));
    }

    /// TOOL-213-12: Import Image as Brush… makes a preset from the picked image's bytes and
    /// shows Brush Settings › Brushes; a file that isn't an image is an error.
    #[test]
    fn image_bytes_become_a_brush_preset() {
        let mut src = photocraft_engine::Session::new();
        src.execute("file.new", json!({"width": 12, "height": 9, "background": "white"})).unwrap();
        src.execute("select.rect", json!({"x": 2, "y": 2, "width": 5, "height": 3})).unwrap();
        src.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        let png = photocraft_io::export(&src.active().unwrap().doc, "Leaf.png", &Default::default()).unwrap().bytes;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let r = image_brush_from(&mut app, "Leaf.png", &png).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(5), Some(3)));
        assert_eq!(app.session.tools.current_preset.as_deref(), Some("Leaf"));
        assert!(app.ui.panels.brush_settings && app.ui.brush_tab == 1);
        assert!(image_brush_from(&mut app, "notes.png", b"hello").is_err());
    }
}
