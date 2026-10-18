//! Tool options survive a restart (UI-217-1): the selected tool, the options bar of every tool and
//! each painting tool's own brush are remembered in the preferences (`prefs.dialogs["ui.tools"]`,
//! shell-owned JSON) and restored at launch, as Photoshop keeps them between sessions.
//!
//! Brushes with a sampled or stored tip (a custom image) are left out: their pixels would bloat the
//! preferences file. That tool starts from its default brush instead.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};
use photocraft_engine::paint::BrushSettings;
use photocraft_engine::paint::brush::TipShape;

/// Key in `prefs.dialogs`.
pub const KEY: &str = "ui.tools";

/// Frames between checks: the snapshot is a small JSON compare, but there's no need for it every
/// frame.
const EVERY: u64 = 30;

/// What `prefs.dialogs["ui.tools"]` holds.
pub fn snapshot(app: &PhotocraftApp) -> Value {
    // The live brush is the brush tool's; its stored copy may be older.
    let mut brushes: Vec<(Tool, BrushSettings)> = app.ui.tool_brushes.clone();
    if crate::paint_mouse::has_brush_picker(app.ui.brush_tool) {
        let live = app.session.tools.brush.clone();
        match brushes.iter_mut().find(|(t, _)| *t == app.ui.brush_tool) {
            Some(e) => e.1 = live,
            None => brushes.push((app.ui.brush_tool, live)),
        }
    }
    brushes.retain(|(_, b)| matches!(b.tip, TipShape::Round));
    json!({"tool": app.ui.tool, "options": app.ui.tool_options, "brushTool": app.ui.brush_tool, "brushes": brushes})
}

/// Remember the tools in the preferences once the pointer is up. Cheap and throttled.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.frame.is_multiple_of(EVERY) || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().dialogs.get(KEY) != Some(&now) {
        app.session.prefs.edit(|p| p.dialogs.insert(KEY.into(), now));
    }
}

/// Restore the remembered tools at launch. Parts that don't read (an older or hand-edited file)
/// keep their defaults; a brush that doesn't read is dropped.
pub fn restore(app: &mut PhotocraftApp) {
    let Some(saved) = app.session.prefs().dialogs.get(KEY).cloned() else { return };
    if let Some(o) = saved.get("options").filter(|v| v.is_object()).and_then(|v| serde_json::from_value::<ToolOptions>(v.clone()).ok()) {
        app.ui.tool_options = o;
    }
    if let Some(list) = saved.get("brushes").and_then(Value::as_array) {
        app.ui.tool_brushes = list.iter().filter_map(|e| serde_json::from_value::<(Tool, BrushSettings)>(e.clone()).ok()).collect();
    }
    let read_tool = |k: &str| saved.get(k).and_then(|v| serde_json::from_value::<Tool>(v.clone()).ok());
    if let Some(t) = read_tool("brushTool").filter(|t| crate::paint_mouse::has_brush_picker(*t)) {
        app.ui.brush_tool = t;
        if let Some((_, b)) = app.ui.tool_brushes.iter().find(|(x, _)| *x == t) {
            app.session.tools.brush = crate::paint_mouse::session_fields(&app.session.tools.brush, b);
        }
    }
    if let Some(t) = read_tool("tool") {
        app.ui.tool = t;
        crate::paint_mouse::sync_tool_brush(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    /// Options, the selected tool and each painting tool's brush come back after a restart (the
    /// preferences JSON carried over to a fresh app), but a sampled tip is not stored.
    #[test]
    fn tools_survive_a_restart() {
        let mut a = app();
        a.ui.tool_options.tolerance = 77.0;
        a.ui.tool_options.contiguous = false;
        a.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_brush(&mut a);
        a.session.tools.brush.size = 42.0;
        a.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut a);
        a.session.tools.brush.size = 9.0;
        let snap = snapshot(&a);
        a.session.prefs.edit(|p| p.dialogs.insert(KEY.into(), snap));
        let text = a.session.prefs_value().to_string();

        let mut b = app();
        b.session.load_prefs_json(&text).unwrap();
        restore(&mut b);
        assert_eq!(b.ui.tool, Tool::Eraser);
        assert_eq!((b.ui.tool_options.tolerance, b.ui.tool_options.contiguous), (77.0, false));
        assert_eq!(b.session.tools.brush.size, 9.0, "the Eraser's brush is live");
        b.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_brush(&mut b);
        assert_eq!(b.session.tools.brush.size, 42.0, "the Brush kept its own size");

        // A sampled tip is left out; the rest of the snapshot still stores.
        let mut c = app();
        c.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_brush(&mut c);
        c.session.tools.brush.tip = TipShape::Sampled(photocraft_engine::paint::tile::GrayTile::from_fn(2, 2, |_, _| 1.0));
        let snap = snapshot(&c);
        assert!(snap["brushes"].as_array().unwrap().is_empty(), "{snap}");
    }

    /// Junk in the stored JSON never breaks the launch: unreadable parts keep their defaults.
    #[test]
    fn unreadable_saved_tools_keep_the_defaults() {
        for junk in [json!(null), json!(5), json!({"tool": "Nope", "options": [1], "brushes": [[1, 2], "x"], "brushTool": 3})] {
            let mut a = app();
            a.session.prefs.edit(|p| p.dialogs.insert(KEY.into(), junk.clone()));
            restore(&mut a);
            assert_eq!(a.ui.tool_options, ToolOptions::default(), "{junk}");
            assert!(a.ui.tool_brushes.is_empty());
        }
    }

    /// Persisting is throttled and waits for the pointer to come up.
    #[test]
    fn persist_writes_only_changes() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.frame = 0;
        persist(&mut a, &ctx);
        let first = a.session.prefs().dialogs.get(KEY).cloned();
        assert!(first.is_some());
        let rev = a.session.prefs.rev();
        persist(&mut a, &ctx);
        assert_eq!(a.session.prefs.rev(), rev, "no change, no write");
        a.ui.tool_options.tolerance = 12.0;
        a.frame = 1;
        persist(&mut a, &ctx);
        assert_eq!(a.session.prefs.rev(), rev, "throttled");
        a.frame = EVERY;
        persist(&mut a, &ctx);
        assert_eq!(a.session.prefs().dialogs.get(KEY).and_then(|v| v["options"]["tolerance"].as_f64()), Some(12.0));
    }
}
