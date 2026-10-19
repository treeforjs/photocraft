//! Preferences › Workspace › Auto-Show Hidden Panels: with the panels hidden (Tab, or ⇧Tab for
//! the dock), resting the pointer on the window's left edge brings back the Tools panel and on
//! its right edge the panel dock, until the pointer leaves them again.
//!
//! The panels come back as they are drawn normally (beside the canvas), so this only decides,
//! once a frame, which hidden side shows. The state is per-context temp data: nothing is saved
//! and the panels' own visibility (`Panels`) is untouched.

use crate::PhotocraftApp;

/// How close to the window edge (points) the pointer has to come to reveal a side.
pub const EDGE: f32 = 4.0;
/// Extra room past a revealed panel before it hides again, so a small overshoot doesn't.
pub const SLACK: f32 = 12.0;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct State {
    /// Revealed sides: Tools panel (left), dock (right).
    left: bool,
    right: bool,
    /// The revealed panels' widths last frame.
    left_width: f32,
    right_width: f32,
}

fn id() -> egui::Id {
    egui::Id::new("auto-show-hidden-panels")
}

fn load(ctx: &egui::Context) -> State {
    ctx.data(|d| d.get_temp(id())).unwrap_or_default()
}

/// The hidden sides to show this frame as (Tools panel, dock).
pub fn sides(app: &PhotocraftApp, ctx: &egui::Context) -> (bool, bool) {
    let p = &app.ui.panels;
    let left_hidden = !p.toolbar && p.hidden_by_tab.is_some_and(|[toolbar, _, _]| toolbar);
    let right_hidden = !p.dock;
    let mut s = load(ctx);
    let enabled = app.session.prefs().workspace.auto_show_hidden_panels;
    let screen = ctx.content_rect();
    // A menu or flyout opened from a revealed panel keeps it up; so does a drag in progress.
    let busy = egui::Popup::is_any_open(ctx) || ctx.dragged_id().is_some();
    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()).filter(|_| enabled && !busy) {
        s.left = left_hidden && if s.left { pos.x <= screen.left() + s.left_width + SLACK } else { pos.x <= screen.left() + EDGE };
        s.right = right_hidden && if s.right { pos.x >= screen.right() - s.right_width - SLACK } else { pos.x >= screen.right() - EDGE };
    }
    s.left &= left_hidden && enabled;
    s.right &= right_hidden && enabled;
    ctx.data_mut(|d| d.insert_temp(id(), s));
    (s.left, s.right)
}

/// Records how wide the revealed panels came out (from the available width before and after
/// drawing them).
pub fn note_widths(ctx: &egui::Context, left: Option<f32>, right: Option<f32>) {
    ctx.data_mut(|d| {
        let s: &mut State = d.get_temp_mut_or_default(id());
        if let Some(w) = left.filter(|w| w.is_finite()) {
            s.left_width = w.max(0.0);
        }
        if let Some(w) = right.filter(|w| w.is_finite()) {
            s.right_width = w.max(0.0);
        }
    });
}

/// The revealed sides' recorded widths (tests).
#[cfg(test)]
fn shown(ctx: &egui::Context) -> (Option<f32>, Option<f32>) {
    let s = load(ctx);
    (s.left.then_some(s.left_width), s.right.then_some(s.right_width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    fn harness() -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::Craft);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
            app.sync_views();
            app
        });
        h.run_steps(4);
        h
    }

    fn hover(h: &mut Harness<'static, PhotocraftApp>, x: f32) {
        h.event(egui::Event::PointerMoved(Pos2::new(x, 400.0)));
        h.run_steps(4);
    }

    fn invoke(h: &mut Harness<'static, PhotocraftApp>, id: &str) {
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap();
        h.run_steps(4);
    }

    #[test]
    fn hidden_panels_peek_in_at_the_window_edges() {
        let mut h = harness();
        hover(&mut h, 1.0);
        assert_eq!(shown(&h.ctx), (None, None), "nothing is hidden: nothing to reveal");
        invoke(&mut h, "window.togglePanels");
        assert!(!h.state().ui.panels.toolbar && !h.state().ui.panels.dock);
        hover(&mut h, 600.0);
        assert_eq!(shown(&h.ctx), (None, None));
        // The left edge brings back the Tools panel, and it stays while the pointer is over it.
        hover(&mut h, 1.0);
        let (Some(w), None) = shown(&h.ctx) else { panic!("{:?}", shown(&h.ctx)) };
        assert!(w > 20.0, "drawn: {w}");
        hover(&mut h, w - 2.0);
        assert!(shown(&h.ctx).0.is_some(), "still over the Tools panel");
        hover(&mut h, 600.0);
        assert_eq!(shown(&h.ctx), (None, None), "leaving hides it again");
        // The right edge brings back the dock.
        hover(&mut h, 1199.0);
        let (None, Some(w)) = shown(&h.ctx) else { panic!("{:?}", shown(&h.ctx)) };
        assert!(w > 20.0, "drawn: {w}");
        hover(&mut h, 1200.0 - w + 2.0);
        assert!(shown(&h.ctx).1.is_some(), "still over the dock");
        hover(&mut h, 600.0);
        assert_eq!(shown(&h.ctx), (None, None));
        // The panels' own visibility never changed.
        assert!(!h.state().ui.panels.toolbar && !h.state().ui.panels.dock);

        // Off: the edges do nothing.
        h.state_mut().session.edit_prefs(|p| p.workspace.auto_show_hidden_panels = false);
        hover(&mut h, 1.0);
        hover(&mut h, 1199.0);
        assert_eq!(shown(&h.ctx), (None, None));
    }

    #[test]
    fn shift_tab_hides_only_the_dock_and_only_the_dock_peeks() {
        let mut h = harness();
        invoke(&mut h, "window.toggle.dock");
        hover(&mut h, 1.0);
        assert_eq!(shown(&h.ctx), (None, None), "the Tools panel is showing already");
        hover(&mut h, 1199.0);
        assert!(shown(&h.ctx).1.is_some());
    }
}
