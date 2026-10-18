//! Preferences › Interface appearance switches that act every frame: Animations (egui's fades and
//! slides; off for people who prefer reduced motion) and the canvas surround's dot pattern, which
//! dark and light themes switch separately.

use crate::PhotocraftApp;

/// Whether the canvas surround draws its dot pattern this frame (see [`apply`]).
fn pattern_key() -> egui::Id {
    egui::Id::new("appearance-canvas-pattern")
}

/// Push the Interface appearance preferences into the context for this frame.
pub(crate) fn apply(app: &PhotocraftApp, ctx: &egui::Context) {
    let i = &app.session.prefs().interface;
    let want = if i.animations { egui::Style::default().animation_time } else { 0.0 };
    if ctx.global_style().animation_time != want {
        ctx.global_style_mut(|s| s.animation_time = want);
    }
    let pattern = if app.ui.theme.is_dark() { i.canvas_pattern_in_dark_themes } else { i.canvas_pattern_in_light_themes };
    ctx.data_mut(|d| d.insert_temp(pattern_key(), pattern));
}

/// Whether the canvas surround shows its dot pattern (on until [`apply`] says otherwise).
pub(crate) fn canvas_pattern(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(pattern_key())).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn animations_and_the_canvas_pattern_follow_the_preferences() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let default_time = egui::Style::default().animation_time;
        apply(&app, &ctx);
        assert_eq!(ctx.global_style().animation_time, default_time);
        assert!(canvas_pattern(&ctx));
        app.run("prefs.set", json!({"values": {"interface.animations": false}})).unwrap();
        apply(&app, &ctx);
        assert_eq!(ctx.global_style().animation_time, 0.0, "no animations");
        app.run("prefs.set", json!({"values": {"interface.animations": true}})).unwrap();
        apply(&app, &ctx);
        assert_eq!(ctx.global_style().animation_time, default_time, "animations come back");
        // Dark and light themes switch the pattern separately.
        app.run("prefs.set", json!({"values": {"interface.canvasPatternInDarkThemes": false}})).unwrap();
        for (theme, shown) in [(crate::theme::ThemeKind::Craft, false), (crate::theme::ThemeKind::CraftLight, true)] {
            app.ui.theme = theme;
            apply(&app, &ctx);
            assert_eq!(canvas_pattern(&ctx), shown, "{theme:?}");
        }
        app.run("prefs.set", json!({"values": {"interface.canvasPatternInLightThemes": false}})).unwrap();
        apply(&app, &ctx);
        assert!(!canvas_pattern(&ctx));
    }
}
