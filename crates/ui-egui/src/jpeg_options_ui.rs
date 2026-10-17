//! Save as JPEG: the options File › Save As asks for before writing a JPEG (FILE-215-19).
//!
//! JPEG is lossy, so a save picks how much detail to keep and how the file is laid out: a
//! quality slider with named stops and a live file-size estimate, and the encoding (standard,
//! smaller, or progressive, which shows a rough image first while it loads). The answer is
//! remembered in the preferences (`dialogs["saveJpeg"]`) for the next save.
//!
//! As with TIFF Options, the save is parked in [`Prompt`] while the modal is up and written once
//! confirmed. Saves with an explicit path (the control channel, MCP, actions) don't ask: they use
//! `file.saveACopy`'s `quality` / `jpegFormat` / `scans` params.

use crate::{ExportSettings, PhotocraftApp};

/// How the JPEG's data is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Readable everywhere.
    Baseline,
    /// Baseline with tables computed for this image: a few percent smaller, same pixels.
    Optimized,
    /// Loads as a rough image that sharpens in `n` passes (3–5).
    Progressive(u8),
}

impl Encoding {
    pub fn from_pref(name: &str, scans: u32) -> Self {
        match name {
            "optimized" => Encoding::Optimized,
            "progressive" => Encoding::Progressive(scans.clamp(3, 5) as u8),
            _ => Encoding::Baseline,
        }
    }

    pub fn pref_name(self) -> &'static str {
        match self {
            Encoding::Baseline => "baseline",
            Encoding::Optimized => "optimized",
            Encoding::Progressive(_) => "progressive",
        }
    }
}

/// Where the last answer is remembered (`Preferences::dialogs`).
const PREF: &str = "saveJpeg";

/// A save waiting on the JPEG answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// 1–100.
    pub quality: u8,
    pub encoding: Encoding,
    /// The estimate for `(quality, encoding)`, so it is encoded once per change, not per frame.
    estimate: Option<((u8, Encoding), Option<u64>)>,
}

/// `true` when `path` is a JPEG name (`.jpg`, `.jpeg`, `.jpe`).
pub fn is_jpeg_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && ["jpg", "jpeg", "jpe"].iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Parks a save to `path` behind the prompt, starting from the remembered answer.
pub fn park(app: &mut PhotocraftApp, path: String) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    let saved = app.session.prefs().dialogs.get(PREF).cloned().unwrap_or_default();
    let quality = saved.get("quality").and_then(serde_json::Value::as_u64).map_or(85, |q| q.clamp(1, 100) as u8);
    let scans = saved.get("scans").and_then(serde_json::Value::as_u64).map_or(3, |n| n.clamp(3, 5) as u32);
    let encoding = Encoding::from_pref(saved.get("encoding").and_then(serde_json::Value::as_str).unwrap_or("baseline"), scans);
    app.jpeg_options = Some(Prompt { doc, path, quality, encoding, estimate: None });
    Ok(())
}

/// The encoder settings for an answer.
pub fn settings(quality: u8, encoding: Encoding) -> ExportSettings {
    let (jpeg_progressive_scans, jpeg_optimize) = match encoding {
        Encoding::Baseline => (0, false),
        Encoding::Optimized => (0, true),
        Encoding::Progressive(n) => (n.clamp(3, 5), false),
    };
    ExportSettings { jpeg_quality: Some(quality.clamp(1, 100)), jpeg_progressive_scans, jpeg_optimize, ..Default::default() }
}

/// The named stop a quality falls in.
pub fn quality_name(q: u8) -> &'static str {
    match q {
        0..=39 => tl!("Low"),
        40..=69 => tl!("Medium"),
        70..=89 => tl!("High"),
        _ => tl!("Maximum"),
    }
}

/// Snap stops under the slider: (label, quality).
const STOPS: [(&str, u8); 4] = [("Low", 30), ("Medium", 60), ("High", 85), ("Maximum", 100)];

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b.div_ceil(1 << 10)),
        b => format!("{b} B"),
    }
}

/// The estimate for the prompt's current answer (computed once per change).
fn estimate(app: &mut PhotocraftApp) -> Option<u64> {
    let p = app.jpeg_options.as_ref()?;
    let key = (p.quality, p.encoding);
    if let Some((k, v)) = p.estimate
        && k == key
    {
        return v;
    }
    let doc = p.doc;
    let st = app.session.documents().iter().find(|d| d.doc.id == doc)?;
    let mut f = serde_json::Map::new();
    f.insert("format".into(), serde_json::json!("jpg"));
    let v = crate::export_dialog::estimate_with(app, &st.doc, &f, &settings(key.0, key.1));
    if let Some(p) = app.jpeg_options.as_mut() {
        p.estimate = Some((key, v));
    }
    v
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.jpeg_options.is_none() {
        return;
    }
    let size = estimate(app);
    let Some(p) = &mut app.jpeg_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut quality = f32::from(p.quality);
    let mut encoding = p.encoding;
    let modal = egui::Modal::new(egui::Id::new("jpeg-options")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(tl!("Save as JPEG")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Quality")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} · {}", quality.round() as u8, quality_name(quality.round() as u8)));
            });
        });
        crate::widgets::slider(ui, &mut quality, 1.0..=100.0, None);
        ui.horizontal(|ui| {
            for (label, q) in STOPS {
                if ui.small_button(tl!(label)).clicked() {
                    quality = f32::from(q);
                }
            }
        });
        ui.add_space(4.0);
        let size_text = match size {
            Some(b) => crate::i18n::fmt(tl!("About {size}"), &[("size", &human_size(b))]),
            None => tl!("Size estimate unavailable").to_string(),
        };
        ui.label(egui::RichText::new(size_text).weak());
        ui.add_space(12.0);
        ui.label(egui::RichText::new(tl!("Encoding")).strong());
        let progressive = matches!(encoding, Encoding::Progressive(_));
        let scans = if let Encoding::Progressive(n) = encoding { n } else { 3 };
        if ui.radio(encoding == Encoding::Baseline, tl!("Standard: opens in every app")).clicked() {
            encoding = Encoding::Baseline;
        }
        if ui.radio(encoding == Encoding::Optimized, tl!("Smaller file: same image, a little slower to save")).clicked() {
            encoding = Encoding::Optimized;
        }
        if ui.radio(progressive, tl!("Progressive: shows a preview while it loads on the web")).clicked() {
            encoding = Encoding::Progressive(scans);
        }
        // Always laid out (greyed unless progressive), so choosing an encoding never moves the
        // dialog's buttons.
        ui.add_enabled_ui(progressive, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.label(tl!("Passes"));
                let mut n = scans;
                if crate::widgets::dropdown(ui, "jpeg-options-scans", &mut n, &[(3, "3"), (4, "4"), (5, "5")], 60.0) {
                    encoding = Encoding::Progressive(n);
                }
            });
        });
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("Save"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.quality = quality.round().clamp(1.0, 100.0) as u8;
    p.encoding = encoding;
    if ok || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        confirm(app, ctx);
    } else if cancel {
        app.jpeg_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

/// Write the parked save with the prompt's answer and remember the answer.
pub fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.jpeg_options.take() else { return };
    let scans = if let Encoding::Progressive(n) = p.encoding { n } else { 3 };
    let answer = serde_json::json!({"quality": p.quality, "encoding": p.encoding.pref_name(), "scans": scans});
    app.session.edit_prefs(|prefs| {
        prefs.dialogs.insert(PREF.into(), answer);
    });
    let settings = settings(p.quality, p.encoding);
    match app.with_document(p.doc, |app| app.write_document(p.path, &settings, false)) {
        Err(e) => {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
        Ok(Some(_)) => crate::discard_ui::saved_document(app, ctx, p.doc),
        // A background save releases the close prompt when it finishes (`jobs_ui`).
        Ok(None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jpeg_names() {
        assert!(is_jpeg_path("a.jpg"));
        assert!(is_jpeg_path("dir/a.JPEG"));
        assert!(is_jpeg_path("C:\\x\\y.jpe"));
        assert!(!is_jpeg_path("a.jpg.psd"));
        assert!(!is_jpeg_path(".jpg"));
        assert!(!is_jpeg_path("a.png"));
    }

    #[test]
    fn answers_map_to_encoder_settings() {
        assert_eq!((settings(85, Encoding::Baseline).jpeg_progressive_scans, settings(85, Encoding::Baseline).jpeg_optimize), (0, false));
        assert!(settings(85, Encoding::Optimized).jpeg_optimize);
        assert_eq!(settings(85, Encoding::Progressive(5)).jpeg_progressive_scans, 5);
        // Hostile values from a hand-edited preferences file are clamped.
        assert_eq!(settings(0, Encoding::Progressive(200)).jpeg_progressive_scans, 5);
        assert_eq!(settings(0, Encoding::Baseline).jpeg_quality, Some(1));
        assert_eq!(Encoding::from_pref("progressive", 99), Encoding::Progressive(5));
        assert_eq!(Encoding::from_pref("nonsense", 4), Encoding::Baseline);
        assert_eq!(quality_name(100), "Maximum");
        assert_eq!(human_size(1536), "2 KB");
    }

    fn app() -> PhotocraftApp {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, crate::ExportSettings)>::new()));
        let w = written.clone();
        let services = crate::Services {
            export: Some(Box::new(move |_doc: &photocraft_doc::Document, path: &str, s: &crate::ExportSettings| {
                w.lock().unwrap().push((path.to_string(), s.clone()));
                Ok((vec![0; usize::from(s.jpeg_quality.unwrap_or(1))], vec![]))
            })),
            write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app
    }

    /// Choosing a .jpg name parks the save; Save writes it with the answer and remembers it;
    /// Cancel writes nothing.
    #[test]
    fn saving_as_jpeg_asks_then_writes_with_the_answer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.save_to_from_dialog("/tmp/x/photo.jpg".into()).unwrap();
        assert!(app.jpeg_options.is_some(), "parked");
        assert!(app.session.active().unwrap().path.is_none(), "nothing written yet");
        // A second save waits for the answer.
        assert!(app.save_as(None).is_err());
        let p = app.jpeg_options.as_mut().unwrap();
        assert_eq!((p.quality, p.encoding), (85, Encoding::Baseline), "the defaults");
        p.quality = 60;
        p.encoding = Encoding::Progressive(4);
        assert!(estimate(&mut app).is_some_and(|b| b > 0));
        confirm(&mut app, &ctx);
        assert!(app.jpeg_options.is_none());
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/x/photo.jpg"));
        assert_eq!(app.session.prefs().dialogs.get(PREF), Some(&json!({"quality": 60, "encoding": "progressive", "scans": 4})));
        // The next save starts from the remembered answer; Cancel (Escape) drops it.
        app.save_to_from_dialog("/tmp/x/other.jpeg".into()).unwrap();
        let p = app.jpeg_options.as_ref().unwrap();
        assert_eq!((p.quality, p.encoding), (60, Encoding::Progressive(4)));
        app.jpeg_options = None;
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/x/photo.jpg"));
        // Other formats and explicit paths don't ask.
        app.save_to_from_dialog("/tmp/x/a.png".into()).unwrap();
        assert!(app.jpeg_options.is_none());
        app.save_as(Some("/tmp/x/b.jpg".into())).unwrap();
        assert!(app.jpeg_options.is_none());
    }

    /// Driven through the modal: an encoding, a quality stop, then Save.
    #[test]
    fn the_modal_saves_with_the_chosen_encoding() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
        h.state_mut().save_to_from_dialog("/tmp/x/photo.jpg".into()).unwrap();
        h.run_steps(2);
        h.get_by_label("Progressive: shows a preview while it loads on the web").click();
        h.run_steps(2);
        h.get_by_label("Low").click();
        h.run_steps(2);
        assert_eq!(h.state().jpeg_options.as_ref().map(|p| p.quality), Some(30));
        h.get_by_label("Save").click();
        h.run_steps(2);
        let app = h.state();
        assert!(app.jpeg_options.is_none());
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/x/photo.jpg"));
        let saved = app.session.prefs().dialogs.get(PREF).cloned().unwrap_or_default();
        assert_eq!((saved["quality"].clone(), saved["encoding"].clone()), (json!(30), json!("progressive")));
    }
}
