//! PSD Options: the prompt a PSD or PSB save shows when Preferences › File
//! Handling › Maximize PSD and PSB File Compatibility is set to Ask, as Photoshop does.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered: with the merged composite (Maximize Compatibility on) or with a white
//! placeholder in its place (smaller, faster). "Don't show again" turns the preference into
//! the answer given (Always or Never). Cancel drops the save.

use photocraft_engine::prefs::Ask;

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the PSD Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// Maximize Compatibility.
    pub maximize: bool,
    /// Store the answer as the preference.
    pub dont_ask_again: bool,
}

/// `true` when `path` is a PSD or PSB name.
pub fn is_psd_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb")))
}

/// Whether saving to `path` should ask about Maximize Compatibility first.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_psd_path(path) && app.session.prefs().file_handling.maximize_psd_compatibility == Ask::Ask
}

/// Maximize Compatibility for a save that doesn't ask: off only when the preference is Never.
pub fn maximize_by_default(app: &PhotocraftApp) -> bool {
    app.session.prefs().file_handling.maximize_psd_compatibility != Ask::Never
}

/// Parks the save behind the prompt.
pub fn park(app: &mut PhotocraftApp, path: String) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    app.psd_options = Some(Prompt { doc, path, maximize: true, dont_ask_again: false });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.psd_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let (mut maximize, mut dont_ask) = (p.maximize, p.dont_ask_again);
    let modal = egui::Modal::new(egui::Id::new("psd-options")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("PSD Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, &mut maximize, tl!("Maximize Compatibility"));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!("Keeps a flattened copy of the image so other apps can read the file. Off saves smaller files faster.")).weak());
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, &mut dont_ask, tl!("Don't show again"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.maximize = maximize;
    p.dont_ask_again = dont_ask;
    if ok || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        let Some(p) = app.psd_options.take() else { return };
        if p.dont_ask_again {
            let answer = if p.maximize { Ask::Always } else { Ask::Never };
            app.session.edit_prefs(|prefs| prefs.file_handling.maximize_psd_compatibility = answer);
        }
        let settings = ExportSettings { maximize_compatibility: p.maximize, ..Default::default() };
        match app.with_document(p.doc, |app| app.write_document(p.path, &settings, false)) {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(None) => {}
        }
    } else if cancel {
        app.psd_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{PhotocraftApp, Services};
    use photocraft_engine::Session;
    use std::{cell::RefCell, rc::Rc};

    type Saves = Rc<RefCell<Vec<(String, bool)>>>;

    /// An app with a layered document whose saves record (path, Maximize Compatibility).
    fn app() -> (PhotocraftApp, Saves) {
        let saves: Saves = Rc::default();
        let seen = saves.clone();
        let mut app = PhotocraftApp::new(
            Session::new(),
            Services {
                export: Some(Box::new(move |_, path, settings| {
                    seen.borrow_mut().push((path.to_string(), settings.maximize_compatibility));
                    Ok((Vec::new(), Vec::new()))
                })),
                write: Some(Box::new(|_, _| Ok(()))),
                ..Default::default()
            },
        );
        app.run("file.new", serde_json::json!({"width": 4, "height": 4})).unwrap();
        app.run("layer.new.layer", serde_json::json!({})).unwrap();
        (app, saves)
    }

    fn set(app: &mut PhotocraftApp, v: Ask) {
        app.session.edit_prefs(|p| p.file_handling.maximize_psd_compatibility = v);
    }

    fn press_enter(app: &mut PhotocraftApp) {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Craft);
        for _ in 0..2 {
            let input = egui::RawInput {
                events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            };
            ctx.begin_pass(input);
            show(app, &ctx);
            ctx.end_pass().textures_delta.clear();
        }
    }

    #[test]
    fn always_and_never_save_without_asking() {
        let (mut app, saves) = app();
        app.save_as(Some("a.psd".into())).unwrap();
        set(&mut app, Ask::Never);
        app.save_as(Some("b.psb".into())).unwrap();
        // Other formats ignore it.
        set(&mut app, Ask::Ask);
        app.save_as(Some("c.png".into())).unwrap();
        assert!(app.psd_options.is_none());
        assert_eq!(saves.borrow().as_slice(), &[("a.psd".into(), true), ("b.psb".into(), false), ("c.png".into(), true)]);
    }

    #[test]
    fn ask_parks_the_save_until_answered() {
        let (mut app, saves) = app();
        set(&mut app, Ask::Ask);
        app.save_as(Some("a.psd".into())).unwrap();
        assert!(app.psd_options.is_some());
        assert!(saves.borrow().is_empty(), "nothing is written before the answer");
        assert!(app.save_as(Some("b.psd".into())).is_err(), "one save at a time");
        if let Some(p) = &mut app.psd_options {
            p.maximize = false;
        }
        press_enter(&mut app);
        assert!(app.psd_options.is_none());
        assert_eq!(saves.borrow().as_slice(), &[("a.psd".into(), false)]);
        assert_eq!(app.session.prefs().file_handling.maximize_psd_compatibility, Ask::Ask, "still asks next time");

        // "Don't show again" keeps the answer as the preference.
        app.save_as(Some("a.psd".into())).unwrap();
        if let Some(p) = &mut app.psd_options {
            p.maximize = false;
            p.dont_ask_again = true;
        }
        press_enter(&mut app);
        assert_eq!(app.session.prefs().file_handling.maximize_psd_compatibility, Ask::Never);
        app.save_as(Some("a.psd".into())).unwrap();
        assert!(app.psd_options.is_none());
        assert_eq!(saves.borrow().len(), 3);
    }

    #[test]
    fn psd_names() {
        assert!(is_psd_path("a.psd"));
        assert!(is_psd_path("dir/a.PSB"));
        assert!(is_psd_path("C:\\x\\y.psd"));
        assert!(!is_psd_path("a.psd.png"));
        assert!(!is_psd_path(".psd"));
        assert!(!is_psd_path("psd"));
    }
}
