//! Help › Check for Updates…: find, download, verify and install a new PhotoCraft with
//! [`craft_update`], the updater every craft app shares.
//!
//! The desktop app hands the shell an [`craft_update::Updater`] (with the GitHub source) through
//! [`PhotocraftApp::set_updater`]; this crate has no network code. Without one (the web build,
//! tests, headless runs) the command opens the releases page instead.
//!
//! - The command `help.checkUpdates` opens the Updates dialog and starts a check; its `action`
//!   param drives every button (`check`, `download`, `cancel`, `skip`, `later`, `restart`,
//!   `installOnQuit`, `openPage`, `status`), so agents can do whatever the dialog does.
//! - Preferences › General › Check for updates automatically (`general.updates`, a
//!   [`craft_update::AutoCheck`]): once the app has drawn its first frame and been up for
//!   [`craft_update::AutoCheck::START_DELAY_SECS`], a due check runs on the updater's own thread.
//!   Nothing touches the network or the disk for updates before that. A background check that
//!   finds a new version (not skipped) posts a quiet notice with a "View Update" button; an
//!   up-to-date or failed background check shows nothing.
//! - A verified download installs when the app quits (after the unsaved-changes prompt), or at
//!   once with Restart Now, which goes through the same close path ([`on_exit`]).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// The command id (Help menu).
pub const COMMAND: &str = "help.checkUpdates";
/// Where every PhotoCraft release is listed.
pub const RELEASES_PAGE: &str = "https://github.com/treeforjs/photocraft/releases";
const MARK: &str = "__updates";
/// Release notes shown in the dialog are cut to this many characters.
const MAX_NOTES: usize = 800;

/// The updater and what the shell remembers about it this session.
#[derive(Default)]
pub struct Updates {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Option<craft_update::Updater>,
    /// The check now running was started by the schedule, not by the user.
    background: bool,
    /// egui time of the first frame: the automatic check waits from here.
    started_at: Option<f64>,
    /// The automatic check of this run was started or found not due.
    auto_done: bool,
    /// Restart Now was chosen: the install on quit reopens PhotoCraft.
    restart: bool,
    /// The unsaved-changes prompt came up after Restart Now (cancelling it drops the restart).
    restart_prompted: bool,
    /// The "update available" notice on screen, if any.
    notice: Option<u64>,
}

impl Updates {
    /// Whether an updater is attached (the desktop app).
    pub fn available(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.updater.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
}

impl PhotocraftApp {
    /// Attach the updater (desktop app). It wakes the UI whenever there is news.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_updater(&mut self, mut updater: craft_update::Updater, ctx: &egui::Context) {
        let ctx = ctx.clone();
        updater.on_change(move || ctx.request_repaint());
        self.updates.updater = Some(updater);
    }
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn dialog_id(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| d.id)
}

fn open_dialog(app: &mut PhotocraftApp) -> u64 {
    if let Some(id) = dialog_id(app) {
        return id;
    }
    let fields = json!({MARK: true, "__label": "Check for Updates"});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default())
}

fn close_dialog(app: &mut PhotocraftApp) {
    if let Some(id) = dialog_id(app) {
        app.ui.close_dialog(id);
    }
}

fn drop_notice(app: &mut PhotocraftApp) {
    if let Some(id) = app.updates.notice.take() {
        app.ui.notices.retain(|n| n.id != id);
    }
}

/// `help.checkUpdates`: without params, the Updates dialog and a check; `{"action": …}` does what
/// the dialog's buttons do.
pub fn run(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    let action = params.get("action").and_then(Value::as_str).unwrap_or("check");
    if !app.updates.available() {
        return match action {
            "status" => Ok(json!({"state": "unavailable", "current": current()})),
            // The web build (and builds without an updater): the releases page lists every version.
            "check" | "openPage" => Ok(crate::links::open(app, ctx, RELEASES_PAGE)),
            other => Err(format!("`{other}` needs the desktop app's updater")),
        };
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        act(app, ctx, action)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err(format!("`{action}` needs the desktop app's updater"))
    }
}

fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(not(target_arch = "wasm32"))]
fn act(app: &mut PhotocraftApp, ctx: &egui::Context, action: &str) -> Result<Value, String> {
    use craft_update::State;
    let state = app.updates.updater.as_ref().map(|u| u.state().clone()).unwrap_or_default();
    match action {
        "check" => {
            let dialog = open_dialog(app);
            app.updates.background = false;
            drop_notice(app);
            if let Some(u) = app.updates.updater.as_mut()
                && matches!(state, State::Idle | State::UpToDate | State::Failed(_))
            {
                u.check();
            }
            return Ok(json!({"dialog": dialog, "update": status(app)}));
        }
        "download" => {
            let installable = matches!(&state, State::Available(o) if o.install.is_ok());
            if !installable {
                return Err("there is no update this copy can install itself".into());
            }
            if let Some(u) = app.updates.updater.as_mut() {
                u.download();
            }
        }
        "cancel" => {
            if let Some(u) = app.updates.updater.as_mut() {
                u.cancel();
            }
        }
        "skip" => {
            let version = match &state {
                State::Available(o) => o.release.version.to_string(),
                _ => return Err("there is no available update to skip".into()),
            };
            app.session.prefs.edit(|p| p.general.updates.skipped = Some(version));
            if let Some(u) = app.updates.updater.as_mut() {
                u.dismiss();
            }
            drop_notice(app);
            close_dialog(app);
        }
        "later" => {
            if let Some(u) = app.updates.updater.as_mut() {
                u.dismiss();
            }
            close_dialog(app);
        }
        "installOnQuit" => {
            if !matches!(state, State::Ready(_)) {
                return Err("no update has been downloaded yet".into());
            }
            close_dialog(app);
        }
        "restart" => {
            if !matches!(state, State::Ready(_)) {
                return Err("no update has been downloaded yet".into());
            }
            app.updates.restart = true;
            app.updates.restart_prompted = false;
            close_dialog(app);
            // The normal way out: unsaved documents are asked about first (`discard_ui`), and
            // the install happens in `on_exit` once leaving is settled.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        "openPage" => {
            let page = match &state {
                State::Available(o) | State::Ready(o) | State::Downloading { offer: o, .. } => o.release.page.clone(),
                _ => RELEASES_PAGE.to_string(),
            };
            close_dialog(app);
            crate::links::open(app, ctx, &page);
        }
        "status" => {}
        other => {
            return Err(format!("unknown action `{other}`: check|download|cancel|skip|later|restart|installOnQuit|openPage|status"));
        }
    }
    Ok(json!({"update": status(app)}))
}

/// The updater's state as JSON (`ui.inspect`-style, for agents and tests).
pub fn status(app: &PhotocraftApp) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(u) = app.updates.updater.as_ref() {
        use craft_update::State;
        let offer = |o: &craft_update::Offer| {
            json!({
                "version": o.release.version.to_string(),
                "page": o.release.page,
                "notes": short_notes(&o.release.notes),
                "install": o.install.is_ok(),
                "reason": o.install.as_ref().err(),
            })
        };
        let mut v = match u.state() {
            State::Idle => json!({"state": "idle"}),
            State::Checking => json!({"state": "checking"}),
            State::UpToDate => json!({"state": "upToDate"}),
            State::Available(o) => json!({"state": "available", "offer": offer(o)}),
            State::Downloading { offer: o, received, total } => {
                json!({"state": "downloading", "offer": offer(o), "received": received, "total": total})
            }
            State::Ready(o) => json!({"state": "ready", "offer": offer(o)}),
            State::Failed(e) => json!({"state": "failed", "error": e.to_string()}),
        };
        if let Some(m) = v.as_object_mut() {
            m.insert("current".into(), json!(current()));
            m.insert("restart".into(), json!(app.updates.restart));
        }
        return v;
    }
    let _ = app;
    json!({"state": "unavailable", "current": current()})
}

/// The start of the release notes, cut at a character boundary.
fn short_notes(notes: &str) -> String {
    let notes = notes.trim();
    match notes.char_indices().nth(MAX_NOTES) {
        Some((i, _)) => format!("{}…", notes.get(..i).unwrap_or(notes).trim_end()),
        None => notes.to_string(),
    }
}

/// Every frame: pick up the updater's news, run the automatic check when it is due, and keep the
/// Restart Now choice honest. Called from `logic`.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = ctx.input(|i| i.time);
        tick_at(app, ctx, now, craft_update::AutoCheck::now);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// [`tick`] at egui time `now` (seconds), with `wall` giving seconds since 1970 (read only once
/// the start delay has passed).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn tick_at(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64, wall: fn() -> u64) {
    use craft_update::State;
    let Some(u) = app.updates.updater.as_mut() else { return };
    let was_checking = matches!(u.state(), State::Checking);
    if u.poll() && was_checking && !matches!(u.state(), State::Checking) {
        finished_check(app, wall());
    }
    // Restart Now, then Cancel in the unsaved-changes prompt: stay, and don't restart later.
    if app.updates.restart {
        if app.discard.is_some() {
            app.updates.restart_prompted = true;
        } else if app.updates.restart_prompted && !app.allow_close {
            app.updates.restart = false;
            app.updates.restart_prompted = false;
        }
    }
    if app.updates.auto_done {
        return;
    }
    // The first frame only notes the time: nothing for updates runs before the app is drawn.
    let Some(started) = app.updates.started_at else {
        app.updates.started_at = Some(now);
        ctx.request_repaint_after(std::time::Duration::from_secs(craft_update::AutoCheck::START_DELAY_SECS));
        return;
    };
    let wait = craft_update::AutoCheck::START_DELAY_SECS as f64 - (now - started);
    if wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wait).unwrap_or_default());
        return;
    }
    app.updates.auto_done = true;
    if !app.session.prefs().general.updates.due(wall()) {
        return;
    }
    if let Some(u) = app.updates.updater.as_mut()
        && matches!(u.state(), State::Idle)
    {
        u.check();
        app.updates.background = true;
    }
}

/// A check ended: remember when, and tell the user about a new version a background check found
/// (quietly, never with a modal).
#[cfg(not(target_arch = "wasm32"))]
fn finished_check(app: &mut PhotocraftApp, wall: u64) {
    use craft_update::State;
    app.session.prefs.edit(|p| p.general.updates.last_checked = Some(wall));
    if !std::mem::take(&mut app.updates.background) || dialog_id(app).is_some() {
        return;
    }
    let found = match app.updates.updater.as_ref().map(|u| u.state()) {
        Some(State::Available(o)) if !app.session.prefs().general.updates.is_skipped(&o.release.version) => Some(o.release.version.to_string()),
        _ => None,
    };
    let Some(version) = found else {
        // Up to date, failed or skipped: nothing to show; forget it so the next check starts clean.
        if let Some(u) = app.updates.updater.as_mut() {
            u.dismiss();
        }
        return;
    };
    drop_notice(app);
    let id = crate::notices::post(app, "PhotoCraft {version} is available", vec!["Open it to see what's new and install it.".into()], false, None);
    if let Some(n) = app.ui.notices.iter_mut().find(|n| n.id == id) {
        n.args = vec![("version".into(), version)];
        n.action = Some(crate::notices::NoticeAction { label: "View Update".into(), command: COMMAND.into() });
    }
    app.updates.notice = Some(id);
}

/// The app is leaving (the unsaved-changes prompt is settled): install a verified download, and
/// reopen PhotoCraft if Restart Now was chosen. Failures are logged; leaving goes on.
pub fn on_exit(app: &mut PhotocraftApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let restart = app.updates.restart;
        let Some(u) = app.updates.updater.as_mut() else { return };
        match u.apply(restart) {
            Ok(Some(applied)) => log::info!("update installed on quit ({applied:?}, restart: {restart})"),
            Ok(None) => {}
            Err(e) => log::error!("couldn't install the update: {e}"),
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
}

/// The Updates dialog's contents for the updater's state.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.set_min_width(400.0);
    let s = status(app);
    let state = s.get("state").and_then(Value::as_str).unwrap_or("unavailable");
    let offer = s.get("offer");
    let version = offer.and_then(|o| o.get("version")).and_then(Value::as_str).unwrap_or("");
    let cur = current();
    match state {
        "checking" => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tl!("Checking for updates…"));
            });
        }
        "upToDate" => {
            ui.label(crate::i18n::fmt(tl!("PhotoCraft {version} is up to date."), &[("version", cur)]));
        }
        "available" | "downloading" | "ready" => {
            ui.label(
                egui::RichText::new(crate::i18n::fmt(tl!("PhotoCraft {version} is available"), &[("version", version)])).font(crate::theme::semibold(13.5)),
            );
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("You have version {version}."), &[("version", cur)])).color(t.text_dim));
            let notes = offer.and_then(|o| o.get("notes")).and_then(Value::as_str).unwrap_or("");
            if !notes.is_empty() {
                ui.add_space(6.0);
                egui::Frame::NONE
                    .fill(t.field)
                    .stroke(egui::Stroke::new(1.0, t.field_border))
                    .corner_radius(t.radius_sm)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("update-notes").max_height(150.0).show(ui, |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(notes).color(t.text_dim)).wrap());
                        });
                    });
            }
            ui.add_space(6.0);
            match state {
                "downloading" => {
                    let received = s.get("received").and_then(Value::as_u64).unwrap_or(0);
                    let total = s.get("total").and_then(Value::as_u64).unwrap_or(0);
                    progress(ui, &t, received, total);
                }
                "ready" => {
                    ui.label(tl!("The update is downloaded and verified. It installs when PhotoCraft quits."));
                }
                _ => {
                    if let Some(reason) = offer.and_then(|o| o.get("reason")).and_then(Value::as_str) {
                        ui.label(egui::RichText::new(reason).color(t.text_dim));
                    }
                }
            }
        }
        "failed" => {
            let e = s.get("error").and_then(Value::as_str).unwrap_or("");
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Couldn't check for updates: {error}"), &[("error", e)])).color(t.warning));
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("You have version {version}."), &[("version", cur)])).color(t.text_dim));
        }
        _ => {
            ui.label(crate::i18n::fmt(tl!("You have version {version}."), &[("version", cur)]));
        }
    }
}

/// A download's progress: a bar in the theme's accent and "12.3 of 45.6 MB".
fn progress(ui: &mut egui::Ui, t: &crate::theme::Tokens, received: u64, total: u64) {
    let width = ui.available_width().max(200.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.field);
    let fraction = if total > 0 { (received as f64 / total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let mut done = rect;
    done.set_width(rect.width() * fraction);
    ui.painter().rect_filled(done, t.radius_sm, t.accent);
    let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0);
    let text = if total > 0 {
        crate::i18n::fmt(tl!("Downloading… {received} of {total} MB"), &[("received", &mb(received)), ("total", &mb(total))])
    } else {
        crate::i18n::fmt(tl!("Downloading… {received} MB"), &[("received", &mb(received))])
    };
    ui.label(egui::RichText::new(text).color(t.text_dim));
}

/// The Updates dialog's buttons for the updater's state, in the platform's order. A click runs
/// the same `help.checkUpdates` action an agent would.
pub fn buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};
    let s = status(app);
    let state = s.get("state").and_then(Value::as_str).unwrap_or("unavailable");
    let installable = s.pointer("/offer/install").and_then(Value::as_bool) == Some(true);
    let row: Vec<(ButtonRole, &str, &str)> = match state {
        "checking" => vec![(ButtonRole::Cancel, tl!("Close"), "later")],
        "upToDate" => vec![(ButtonRole::Default, tl!("OK"), "later")],
        "available" if installable => {
            vec![
                (ButtonRole::Default, tl!("Download and Install"), "download"),
                (ButtonRole::Alternate, tl!("Skip This Version"), "skip"),
                (ButtonRole::Cancel, tl!("Later"), "later"),
            ]
        }
        "available" => {
            vec![
                (ButtonRole::Default, tl!("Open Release Page"), "openPage"),
                (ButtonRole::Alternate, tl!("Skip This Version"), "skip"),
                (ButtonRole::Cancel, tl!("Later"), "later"),
            ]
        }
        "downloading" => vec![(ButtonRole::Cancel, tl!("Cancel"), "cancel")],
        "ready" => vec![(ButtonRole::Default, tl!("Restart Now"), "restart"), (ButtonRole::Cancel, tl!("Install When I Quit"), "installOnQuit")],
        "failed" => vec![(ButtonRole::Default, tl!("Open Release Page"), "openPage"), (ButtonRole::Cancel, tl!("Close"), "later")],
        _ => vec![(ButtonRole::Default, tl!("Check Now"), "check"), (ButtonRole::Cancel, tl!("Close"), "later")],
    };
    let specs: Vec<DialogButton> = row.iter().map(|(role, label, _)| DialogButton::new(*role, label, 84.0)).collect();
    let Some(hit) = dialog_buttons(ui, &specs) else { return };
    let Some((_, _, action)) = row.iter().find(|(role, _, _)| *role == hit) else { return };
    let ctx = ui.ctx().clone();
    if let Err(e) = run(app, &ctx, &json!({"action": action})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// OK from automation (`ui.dialog.confirm`) or Enter: close, like Later.
pub fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(u) = app.updates.updater.as_mut() {
        u.dismiss();
    }
    Ok(json!({"update": status(app)}))
}

#[cfg(test)]
#[path = "updates_ui_tests.rs"]
mod tests;
