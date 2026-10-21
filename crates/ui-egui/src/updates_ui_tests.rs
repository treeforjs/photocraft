use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const REPO: &str = "https://github.com/treeforjs/photocraft/releases";

/// GitHub as the updater sees it: a release list, and nothing else downloadable.
struct FakeGitHub {
    /// The answer to the release list request, or the error it fails with.
    releases: Result<String, String>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl craft_update::Source for FakeGitHub {
    fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, craft_update::Error> {
        self.asked.lock().unwrap().push(url.to_string());
        match &self.releases {
            Ok(json) if url.starts_with("https://api.github.com/repos/treeforjs/photocraft/releases") => Ok(json.clone().into_bytes()),
            Ok(_) => Err(craft_update::Error::Network(format!("no such file: {url}"))),
            Err(e) => Err(craft_update::Error::Network(e.clone())),
        }
    }

    fn download(&self, url: &str, _limit: u64, _sink: &mut dyn FnMut(&[u8]) -> Result<(), craft_update::Error>) -> Result<(), craft_update::Error> {
        Err(craft_update::Error::Network(format!("no such file: {url}")))
    }
}

fn info() -> craft_update::AppInfo {
    craft_update::AppInfo {
        id: "photocraft".into(),
        display_name: "PhotoCraft".into(),
        repo: "treeforjs/photocraft".into(),
        current: env!("CARGO_PKG_VERSION").into(),
        // RFC 8032 test vector 1: a valid, non-weak Ed25519 public key.
        // No private key or signature is needed for release-discovery UI tests.
        public_key: Some("11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=".into()),
        prereleases: false,
    }
}

/// A GitHub release list with one release, `tag`, signed and carrying this system's `.deb`.
fn release_list(tag: &str) -> String {
    let version = craft_update::Version::parse(tag).unwrap();
    let deb = craft_update::InstallKind::Deb
        .asset_name(&info(), &version, craft_update::Arch::current())
        .unwrap_or_else(|| format!("photocraft-{version}-linux-x86_64.deb"));
    let asset = |name: &str, size: u64| json!({"name": name, "browser_download_url": format!("{REPO}/download/{tag}/{name}"), "size": size});
    json!([{
        "tag_name": tag,
        "html_url": format!("{REPO}/tag/{tag}"),
        "body": "## What's new\n\n- Faster brushes\n- In-app updates",
        "draft": false,
        "prerelease": false,
        "assets": [asset(&deb, 40 << 20), asset("SHA256SUMS.txt", 200), asset("SHA256SUMS.txt.sig", 88)],
    }])
    .to_string()
}

fn app_with(releases: Result<String, String>) -> (PhotocraftApp, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let source = Arc::new(FakeGitHub { releases, asked: asked.clone() });
    let updater = craft_update::Updater::new(info(), source).with_install_kind(craft_update::InstallKind::Deb);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.set_updater(updater, &egui::Context::default());
    (app, asked)
}

fn wall() -> u64 {
    2_000_000_000
}

/// Runs frames (well past the start delay) until the updater has finished its check.
fn settle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    for _ in 0..500 {
        tick_at(app, ctx, 1_000.0, wall);
        if !app.updates.updater.as_ref().unwrap().busy() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the update check didn't finish: {}", status(app));
}

/// The first frame, then one after the start delay: the automatic check starts if it is due.
fn start_and_wait(app: &mut PhotocraftApp, ctx: &egui::Context) {
    tick_at(app, ctx, 0.0, wall);
    tick_at(app, ctx, craft_update::AutoCheck::START_DELAY_SECS as f64 + 0.5, wall);
}

fn harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    // The theme's fonts arrive a frame after `setup_context`: draw nothing until then.
    let (dialogs, notices) = (std::mem::take(&mut app.ui.dialogs), std::mem::take(&mut app.ui.notices));
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            app.run_pending_invoke(ui.ctx());
            crate::dialogs::show(app, ui.ctx());
            crate::notices::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    h.run_steps(2);
    h.state_mut().ui.dialogs = dialogs;
    h.state_mut().ui.notices = notices;
    h.run_steps(3);
    h
}

#[test]
fn the_dialog_shows_the_new_version_and_a_download_button() {
    let (mut app, _) = app_with(Ok(release_list("v99.0.0")));
    let ctx = egui::Context::default();
    let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert!(r["dialog"].as_u64().is_some(), "{r}");
    assert_eq!(r["update"]["state"], "checking");
    settle(&mut app, &ctx);
    let s = status(&app);
    assert_eq!(s["state"], "available", "{s}");
    assert_eq!(s["offer"]["version"], "99.0.0");
    assert_eq!(s["offer"]["install"], true, "{s}");
    assert!(s["offer"]["notes"].as_str().unwrap().contains("Faster brushes"));
    assert!(app.session.prefs().general.updates.last_checked.is_some(), "a finished check is remembered");
    assert!(app.ui.notices.is_empty(), "the user asked: the dialog answers, no notice");

    let mut h = harness(app);
    h.run_steps(2);
    h.get_by_label("PhotoCraft 99.0.0 is available");
    h.get_by_label("Download and Install");
    h.get_by_label("Skip This Version");
    h.get_by_label("Later").click();
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "Later closes the dialog");
}

#[test]
fn a_copy_that_cant_install_offers_the_release_page() {
    let (mut app, _) = app_with(Ok(release_list("v99.0.0")));
    app.updates.updater = app.updates.updater.take().map(|u| u.with_install_kind(craft_update::InstallKind::Flatpak));
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(status(&app)["offer"]["install"], false);
    assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"action": "download"})).is_err(), "nothing to install");
    let h = harness(app);
    h.get_by_label("Open Release Page");
    assert!(h.query_by_label("Download and Install").is_none());
    h.get_by_label_contains("Flatpak");
}

#[test]
fn a_background_check_shows_a_notice_not_a_modal() {
    let (mut app, _) = app_with(Ok(release_list("v99.0.0")));
    let ctx = egui::Context::default();
    start_and_wait(&mut app, &ctx);
    settle(&mut app, &ctx);
    assert!(app.ui.dialogs.is_empty(), "never a modal over the user's work");
    assert_eq!(app.ui.notices.len(), 1);
    let notice = &app.ui.notices[0];
    assert_eq!(notice.text(&notice.title), "PhotoCraft 99.0.0 is available");
    assert_eq!(notice.action.as_ref().map(|a| a.command.as_str()), Some(COMMAND));

    // Its button opens the dialog on the offer it found, without asking GitHub again.
    let mut h = harness(app);
    h.get_by_label("View Update").click();
    h.run_steps(3);
    let app = h.state();
    assert!(app.ui.notices.is_empty());
    assert_eq!(app.ui.dialogs.len(), 1);
    assert_eq!(status(app)["state"], "available");
    h.get_by_label("Download and Install");
}

#[test]
fn up_to_date_or_failed_background_checks_show_nothing() {
    for releases in [Ok(release_list("v0.0.1")), Err("offline".to_string())] {
        let (mut app, asked) = app_with(releases.clone());
        let ctx = egui::Context::default();
        start_and_wait(&mut app, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(asked.lock().unwrap().len(), 1, "{releases:?}: one request");
        assert!(app.ui.notices.is_empty() && app.ui.dialogs.is_empty(), "{releases:?}");
        assert_eq!(status(&app)["state"], "idle", "{releases:?}: forgotten, so Help › Check for Updates starts afresh");
        assert_eq!(app.session.prefs().general.updates.last_checked, Some(wall()));
    }
}

#[test]
fn skipping_a_version_hides_it_until_a_newer_one() {
    let (mut app, _) = app_with(Ok(release_list("v99.0.0")));
    let ctx = egui::Context::default();
    start_and_wait(&mut app, &ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.ui.notices.len(), 1);
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"action": "skip"})).unwrap();
    assert_eq!(app.session.prefs().general.updates.skipped.as_deref(), Some("99.0.0"));
    assert!(app.ui.notices.is_empty() && app.ui.dialogs.is_empty());
    assert_eq!(status(&app)["state"], "idle");

    // The next day's background check finds the same version: nothing is shown.
    app.session.prefs.edit(|p| p.general.updates.last_checked = None);
    app.updates.auto_done = false;
    app.updates.started_at = None;
    start_and_wait(&mut app, &ctx);
    settle(&mut app, &ctx);
    assert!(app.ui.notices.is_empty() && app.ui.dialogs.is_empty(), "a skipped version isn't announced again");
    // Asking still shows it.
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(status(&app)["offer"]["version"], "99.0.0");
}

static WALL_READ: AtomicBool = AtomicBool::new(false);

fn watched_wall() -> u64 {
    WALL_READ.store(true, Ordering::SeqCst);
    wall()
}

#[test]
fn the_automatic_check_waits_for_the_start_delay_and_the_preference() {
    let (mut app, asked) = app_with(Ok(release_list("v0.0.1")));
    let ctx = egui::Context::default();
    // The first frame and the seconds after it: no request, not even a clock read for updates.
    for now in [0.0, 1.0, craft_update::AutoCheck::START_DELAY_SECS as f64 - 0.5] {
        tick_at(&mut app, &ctx, now, watched_wall);
    }
    assert!(!WALL_READ.load(Ordering::SeqCst), "nothing for updates runs during the start delay");
    assert_eq!(status(&app)["state"], "idle");
    tick_at(&mut app, &ctx, craft_update::AutoCheck::START_DELAY_SECS as f64, watched_wall);
    assert_eq!(status(&app)["state"], "checking");
    settle(&mut app, &ctx);
    assert_eq!(asked.lock().unwrap().len(), 1);

    // Checked within the last day, or switched off: no check.
    for (enabled, last) in [(true, Some(wall() - 60)), (false, None)] {
        let (mut app, asked) = app_with(Ok(release_list("v0.0.1")));
        app.session.prefs.edit(|p| {
            p.general.updates.enabled = enabled;
            p.general.updates.last_checked = last;
        });
        start_and_wait(&mut app, &ctx);
        tick_at(&mut app, &ctx, 1_000.0, wall);
        assert_eq!(status(&app)["state"], "idle");
        assert!(asked.lock().unwrap().is_empty(), "enabled {enabled}, last {last:?}");
    }
}

#[test]
fn the_preference_switch_is_in_general_and_round_trips() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"path": "general.updates.enabled", "value": false})).unwrap();
    let saved = app.session.prefs().to_json().to_string();
    let mut loaded = photocraft_engine::Session::new();
    loaded.load_prefs_json(&saved).unwrap();
    assert!(!loaded.prefs().general.updates.enabled);

    // Preferences › General shows it as a checkbox; unticking it and pressing OK turns checks off.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.preferences.general", json!({})).unwrap();
    let mut h = harness(app);
    h.get_by_label("Check for updates automatically").click();
    h.run_steps(2);
    let id = h.state().ui.dialogs[0].id;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert!(!h.state().session.prefs().general.updates.enabled);
}

#[test]
fn without_an_updater_the_command_opens_the_releases_page() {
    let opened: Arc<Mutex<Vec<String>>> = Arc::default();
    let rec = opened.clone();
    let services = crate::Services {
        open_url: Some(Box::new(move |u: &str| {
            rec.lock().unwrap().push(u.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert_eq!(opened.lock().unwrap().as_slice(), [RELEASES_PAGE]);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"action": "status"})).unwrap()["state"], "unavailable");
    // Ticking and quitting without an updater do nothing.
    tick_at(&mut app, &ctx, 100.0, wall);
    on_exit(&mut app);
}

#[test]
fn bad_actions_are_errors_not_crashes() {
    let (mut app, _) = app_with(Ok(release_list("v99.0.0")));
    let ctx = egui::Context::default();
    for action in [json!("restart"), json!("installOnQuit"), json!("skip"), json!("download"), json!("nonsense"), json!(7)] {
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"action": action}));
        // A non-string action means the default (check); everything else needs the right state.
        if action.is_string() {
            assert!(r.is_err(), "{action}: {r:?}");
        }
    }
    assert!(!app.updates.restart, "Restart Now needs a downloaded update");
    settle(&mut app, &ctx);
    on_exit(&mut app);
    assert_eq!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"action": "cancel"})).unwrap()["update"]["state"], "available");
}
