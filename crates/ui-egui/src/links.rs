//! Community and project links: the Help menu, the About dialog, the start screen and the title
//! bar all open these. URLs follow the crafting-app pattern (`getartcraft.com/apps/<app>`,
//! `github.com/storytold/<app>`).

use serde_json::{Value, json};

use crate::PhotocraftApp;

pub const DISCORD: &str = "https://discord.gg/artcraft";
pub const ARTCRAFT_WEBSITE: &str = "https://getartcraft.com";
pub const APP_PAGE: &str = "https://getartcraft.com/apps/photocraft";
pub const GITHUB: &str = "https://github.com/storytold/photocraft";
pub const ISSUES: &str = "https://github.com/storytold/photocraft/issues";

/// Help-menu link commands: (id, url). Labels live in `menus::UI_COMMANDS`.
pub const COMMANDS: &[(&str, &str)] = &[
    ("help.discord", DISCORD),
    ("help.website", APP_PAGE),
    ("help.artcraftWebsite", ARTCRAFT_WEBSITE),
    ("help.github", GITHUB),
    ("help.reportIssue", ISSUES),
];

pub fn url_for(id: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// Open `url` in the system browser (a new tab on the web) and note it in the status bar.
pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context, url: &str) -> Value {
    ctx.open_url(egui::OpenUrl::new_tab(url));
    app.ui.status = format!("Opened {url}");
    json!({"url": url})
}

/// The prominent "Join us on Discord" button.
pub fn discord_button(app: &mut PhotocraftApp, ui: &mut egui::Ui, min_width: f32) -> egui::Response {
    let r = crate::widgets::primary_button(ui, "Join us on Discord", min_width).on_hover_text(DISCORD);
    if r.clicked() {
        open(app, ui.ctx(), DISCORD);
    }
    r
}

/// "PhotoCraft website · GitHub · ArtCraft" as hyperlinks, centred in the available width.
pub fn link_row(ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let links = [("PhotoCraft website", APP_PAGE), ("GitHub", GITHUB), ("ArtCraft", ARTCRAFT_WEBSITE)];
    let font = egui::FontId::proportional(12.5);
    let sep = "  ·  ";
    let width: f32 = links.iter().map(|(l, _)| ui.painter().layout_no_wrap((*l).into(), font.clone(), t.text).size().x).sum::<f32>()
        + 2.0 * ui.painter().layout_no_wrap(sep.into(), font.clone(), t.text).size().x;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        for (i, (label, url)) in links.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new(sep).font(font.clone()).color(t.text_faint));
            }
            ui.hyperlink_to(egui::RichText::new(*label).font(font.clone()), *url).on_hover_text(*url);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_menu_lists_links_then_separator_then_about() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let help: Vec<String> = crate::menus::menu_items(&app).into_iter().filter(|i| i.path == ["Help"]).map(|i| i.id).collect();
        assert_eq!(help, ["help.discord", "help.website", "help.artcraftWebsite", "help.github", "help.reportIssue", "---", "help.about"]);
        for (id, _) in COMMANDS {
            assert!(crate::menus::is_live(id) && crate::menus::is_enabled(&app, id), "{id}");
        }
    }

    #[test]
    fn link_commands_open_their_urls() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        for (id, url) in [("help.discord", "https://discord.gg/artcraft"), ("help.website", "https://getartcraft.com/apps/photocraft"), ("help.github", "https://github.com/storytold/photocraft")] {
            let r = crate::menus::invoke(&mut app, &ctx, id, serde_json::json!({})).unwrap();
            assert_eq!(r["url"], url);
            assert_eq!(app.ui.status, format!("Opened {url}"));
        }
    }
}
