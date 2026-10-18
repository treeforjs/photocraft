//! Recently used font families (UI-217-1): every font picker lists them above the full list,
//! newest first, and they survive a restart in the preferences (`prefs.dialogs["ui.recentFonts"]`,
//! shell-owned JSON). Edit › Preferences › Type › Number of Recent Fonts caps the list; 0 turns it
//! off.
//!
//! The pickers only see an `egui::Ui`, so the live list sits in the context's temp data and
//! [`sync`] moves it to and from the preferences once per frame.

use serde_json::Value;

use crate::PhotocraftApp;

/// Key in `prefs.dialogs`.
pub const KEY: &str = "ui.recentFonts";

/// Highest cap the preference allows (`prefs::range("type.recentFonts")`).
const MAX: usize = 50;

#[derive(Clone, Default)]
struct Recents {
    list: Vec<String>,
    cap: usize,
    loaded: bool,
}

fn id() -> egui::Id {
    egui::Id::new("photocraft-recent-fonts")
}

/// Recently used families, newest first.
pub(crate) fn list(ctx: &egui::Context) -> Vec<String> {
    ctx.data(|d| d.get_temp::<Recents>(id())).map(|r| r.list).unwrap_or_default()
}

/// A family was picked: move it to the front.
pub(crate) fn note(ctx: &egui::Context, family: &str) {
    if family.is_empty() {
        return;
    }
    ctx.data_mut(|d| {
        let r = d.get_temp_mut_or_default::<Recents>(id());
        r.list.retain(|f| f != family);
        r.list.insert(0, family.to_string());
        r.list.truncate(r.cap);
    });
}

fn stored(app: &PhotocraftApp) -> Vec<String> {
    let Some(list) = app.session.prefs().dialogs.get(KEY).and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for f in list.iter().filter_map(Value::as_str).filter(|f| !f.is_empty()) {
        if !out.iter().any(|o| o == f) {
            out.push(f.to_string());
        }
        if out.len() >= MAX {
            break;
        }
    }
    out
}

/// Load the stored list on the first frame, apply the cap, and store the list when it changed.
pub fn sync(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let cap = usize::try_from(app.session.prefs().type_.recent_fonts).unwrap_or(MAX).min(MAX);
    let mut r: Recents = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    let saved = stored(app);
    if !r.loaded {
        r.list = saved.clone();
        r.loaded = true;
    }
    r.cap = cap;
    r.list.truncate(cap);
    if saved != r.list {
        let v = Value::from(r.list.clone());
        app.session.prefs.edit(|p| p.dialogs.insert(KEY.into(), v));
    }
    ctx.data_mut(|d| d.insert_temp(id(), r));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn picks_move_to_the_front_and_survive_a_restart() {
        let mut a = app();
        let ctx = egui::Context::default();
        sync(&mut a, &ctx);
        assert!(list(&ctx).is_empty());
        for f in ["Alpha", "Beta", "Alpha", "Gamma"] {
            note(&ctx, f);
        }
        assert_eq!(list(&ctx), ["Gamma", "Alpha", "Beta"]);
        sync(&mut a, &ctx);
        // A new window (a restart) reads them back from the preferences.
        let fresh = egui::Context::default();
        sync(&mut a, &fresh);
        assert_eq!(list(&fresh), ["Gamma", "Alpha", "Beta"]);
    }

    #[test]
    fn the_preference_caps_the_list_and_zero_turns_it_off() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.session.prefs.edit(|p| p.type_.recent_fonts = 2);
        sync(&mut a, &ctx);
        for f in ["A", "B", "C"] {
            note(&ctx, f);
        }
        assert_eq!(list(&ctx), ["C", "B"]);
        sync(&mut a, &ctx);
        assert_eq!(a.session.prefs().dialogs.get(KEY), Some(&serde_json::json!(["C", "B"])));
        a.session.prefs.edit(|p| p.type_.recent_fonts = 0);
        sync(&mut a, &ctx);
        note(&ctx, "D");
        assert!(list(&ctx).is_empty());
        assert_eq!(a.session.prefs().dialogs.get(KEY), Some(&Value::from(Vec::<String>::new())));
    }

    #[test]
    fn a_hand_edited_list_is_read_safely() {
        let mut a = app();
        let ctx = egui::Context::default();
        let junk = serde_json::json!(["A", 3, null, "", "A", "B"]);
        a.session.prefs.edit(|p| p.dialogs.insert(KEY.into(), junk));
        sync(&mut a, &ctx);
        assert_eq!(list(&ctx), ["A", "B"]);
    }
}
