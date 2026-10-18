//! On-canvas corner radius widgets for live rectangles (Photoshop's live shape corners): with the
//! Path Selection tool or a shape tool, the active rectangle shape shows a small circle inside each
//! corner. Dragging one rounds all four corners together; ⌥ (Alt) drags just that corner. The drag
//! previews the outline and commits one `shape.edit` on release.

use egui::{Color32, Pos2, Stroke};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// How far (screen points) a widget sits inside a sharp corner, so it never covers the anchor.
const MIN_INSET: f64 = 12.0;
/// Grab distance around a widget, in screen points.
const GRAB: f64 = 7.0;

/// A corner radius drag in progress.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RadiusDrag {
    layer: LayerId,
    corner: usize,
    /// ⌥ was held at the press: only `corner` changes.
    one: bool,
    rect: [f64; 4],
    before: [f64; 4],
    /// The radii the drag shows now: [top-left, top-right, bottom-right, bottom-left].
    pub(crate) radii: [f64; 4],
}

/// The active layer's live rectangle while the corner widgets show: (layer, [x, y, w, h], radii).
fn target(app: &PhotocraftApp) -> Option<(LayerId, [f64; 4], [f64; 4])> {
    let tool = app.ui.tool;
    if !(tool == Tool::PathSelection || crate::vector_ui::is_shape_tool(tool)) || !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    let layer = st.doc.layer(id)?;
    let LayerContent::Shape(sh) = &layer.content else { return None };
    match sh.live {
        Some(LiveShape::Rect { rect, radii }) if layer.visible && rect[2] > 0.0 && rect[3] > 0.0 => Some((id, rect, radii)),
        _ => None,
    }
}

/// The largest radius a corner of `rect` can take.
fn max_radius(rect: [f64; 4]) -> f64 {
    (rect[2].min(rect[3]) / 2.0).max(0.0)
}

/// Corner `i` of `rect` (top-left, top-right, bottom-right, bottom-left) and the direction into the
/// rectangle from it.
fn corner(rect: [f64; 4], i: usize) -> ([f64; 2], [f64; 2]) {
    let [x, y, w, h] = rect;
    match i {
        0 => ([x, y], [1.0, 1.0]),
        1 => ([x + w, y], [-1.0, 1.0]),
        2 => ([x + w, y + h], [-1.0, -1.0]),
        _ => ([x, y + h], [1.0, -1.0]),
    }
}

/// Where corner `i`'s widget sits (document pixels): at the centre of the corner's arc, at least
/// [`MIN_INSET`] screen points in, and never past the middle of the rectangle.
fn widget_at(rect: [f64; 4], radii: [f64; 4], i: usize, zoom: f64) -> [f64; 2] {
    let ([cx, cy], [sx, sy]) = corner(rect, i);
    let r = radii.get(i).copied().unwrap_or(0.0);
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let inset = r.max(MIN_INSET / zoom).min(max_radius(rect));
    [cx + sx * inset, cy + sy * inset]
}

/// The radius a pointer at `p` asks of corner `i`: how far it is into the rectangle from the
/// corner, along the corner's diagonal.
fn radius_at(rect: [f64; 4], i: usize, p: [f64; 2]) -> f64 {
    let ([cx, cy], [sx, sy]) = corner(rect, i);
    let along = ((p[0] - cx) * sx + (p[1] - cy) * sy) / 2.0;
    if along.is_finite() { along.round().clamp(0.0, max_radius(rect)) } else { 0.0 }
}

/// Handle a pointer event; true when the corner widgets took it.
pub(crate) fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let Some((layer, rect, radii)) = target(app) else { return false };
            let zoom = f64::from(app.current_zoom());
            let grab = GRAB / zoom.max(1e-6);
            let hit = (0..4).find(|&i| {
                let [wx, wy] = widget_at(rect, radii, i, zoom);
                (wx - x).hypot(wy - y) <= grab
            });
            let Some(corner) = hit else { return false };
            app.radius_drag = Some(RadiusDrag { layer, corner, one: mods.alt, rect, before: radii, radii });
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let Some(d) = app.radius_drag.as_mut() else { return false };
            let r = radius_at(d.rect, d.corner, [x, y]);
            d.radii = if d.one {
                let mut radii = d.before;
                if let Some(v) = radii.get_mut(d.corner) {
                    *v = r;
                }
                radii
            } else {
                [r; 4]
            };
            true
        }
        ToolEvent::Up { x, y } => {
            if app.radius_drag.is_none() {
                return false;
            }
            pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
            if let Some(d) = app.radius_drag.take()
                && d.radii != d.before
            {
                let _ = app.run("shape.edit", json!({"layer": d.layer, "radii": d.radii}));
            }
            true
        }
    }
}

/// Draw the corner widgets, and the rounded outline a drag would make.
pub(crate) fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, accent: Color32) {
    let Some((layer, rect, mut radii)) = target(app) else { return };
    let to_scr = |q: [f64; 2]| -> Pos2 { xf.to_screen(q[0] as f32, q[1] as f32) };
    if let Some(d) = app.radius_drag.as_ref().filter(|d| d.layer == layer) {
        radii = d.radii;
        let path = photocraft_vector::shapes::live_path(&LiveShape::Rect { rect, radii });
        crate::vector_ui::draw_outline(painter, &path, &to_scr, accent);
    }
    let zoom = f64::from(app.current_zoom());
    for i in 0..4 {
        let c = to_scr(widget_at(rect, radii, i, zoom));
        painter.circle_filled(c, 4.0, Color32::WHITE);
        painter.circle_stroke(c, 4.0, Stroke::new(1.5, accent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::tool_event;

    fn app_with_rect() -> (PhotocraftApp, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [100, 50, 200, 120]})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.ui.tool = Tool::PathSelection;
        (app, id)
    }

    fn radii(app: &PhotocraftApp, id: LayerId) -> [f64; 4] {
        match &app.session.active().unwrap().doc.layer(id).unwrap().content {
            LayerContent::Shape(sh) => match sh.live {
                Some(LiveShape::Rect { radii, .. }) => radii,
                ref other => panic!("not a live rect: {other:?}"),
            },
            _ => panic!("not a shape"),
        }
    }

    fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) {
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
        tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
        tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
    }

    #[test]
    fn dragging_a_corner_widget_rounds_every_corner() {
        let (mut app, id) = app_with_rect();
        let zoom = f64::from(app.current_zoom());
        let w = widget_at([100.0, 50.0, 200.0, 120.0], [0.0; 4], 0, zoom);
        // Dragging the top-left widget to 20 px in from the corner on both axes: radius 20.
        drag(&mut app, w, [120.0, 70.0], egui::Modifiers::NONE);
        assert_eq!(radii(&app, id), [20.0; 4]);
        assert!(app.radius_drag.is_none());
        // One undo step.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(radii(&app, id), [0.0; 4]);
    }

    #[test]
    fn alt_drags_one_corner_and_the_radius_stops_at_half_the_short_side() {
        let (mut app, id) = app_with_rect();
        app.run("shape.edit", json!({"layer": id, "radii": 10})).unwrap();
        let zoom = f64::from(app.current_zoom());
        // The bottom-right widget sits at the arc's centre, (290, 160), or further in at low zoom.
        let w = widget_at([100.0, 50.0, 200.0, 120.0], [10.0; 4], 2, zoom);
        drag(&mut app, w, [0.0, 0.0], egui::Modifiers::ALT);
        assert_eq!(radii(&app, id), [10.0, 10.0, 60.0, 10.0], "only bottom-right, capped at 120 / 2");
        // Dragging out past the corner squares it again.
        let w = widget_at([100.0, 50.0, 200.0, 120.0], [10.0; 4], 0, zoom);
        drag(&mut app, w, [0.0, 0.0], egui::Modifiers::ALT);
        assert_eq!(radii(&app, id), [0.0, 10.0, 60.0, 10.0]);
    }

    #[test]
    fn widgets_show_only_for_a_live_rectangle_with_a_vector_tool() {
        let (mut app, _) = app_with_rect();
        assert!(target(&app).is_some());
        app.ui.tool = Tool::Brush;
        assert!(target(&app).is_none(), "painting tools don't show them");
        app.ui.tool = Tool::Rectangle;
        assert!(target(&app).is_some());
        app.run("shape.create", json!({"kind": "ellipse", "rect": [10, 10, 50, 50]})).unwrap();
        assert!(target(&app).is_none(), "an ellipse has no corners");
        // A press away from the widgets is the tool's own.
        let mut app = app_with_rect().0;
        assert!(!pointer(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, egui::Modifiers::NONE));
        assert!(app.radius_drag.is_none());
    }

    #[test]
    fn hostile_pointer_positions_keep_the_radius_in_range() {
        let rect = [100.0, 50.0, 200.0, 120.0];
        for p in [[f64::NAN, 0.0], [f64::INFINITY, f64::INFINITY], [-1e300, 1e300], [1e300, 1e300]] {
            for i in 0..4 {
                let r = radius_at(rect, i, p);
                assert!((0.0..=60.0).contains(&r), "{p:?} corner {i}: {r}");
            }
        }
        // A zero-sized or broken zoom still places the widgets inside the rectangle.
        for zoom in [0.0, f64::NAN, -1.0, 1e-9] {
            let [x, y] = widget_at(rect, [0.0; 4], 0, zoom);
            assert!((100.0..=160.0).contains(&x) && (50.0..=110.0).contains(&y), "{zoom}: {x},{y}");
        }
    }
}
