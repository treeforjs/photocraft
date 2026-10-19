//! Artboard tool (Move group, V): drag on the pasteboard to draw a new artboard; click a board
//! to select it and drag it elsewhere with its contents (⌥-drag drags out a copy); drag the
//! selected board's handles to resize it (its contents stay put); click a + beside it to add a
//! board of the same size there. Every change is an engine command (`layer.new.artboard`,
//! `layer.artboard.set`, `layer.duplicate`); this module turns pointer gestures into them and
//! draws the overlay.

use egui::{Color32, Pos2, Rect, Stroke, vec2};
use photocraft_doc::{Document, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// Artboard tool state (serde: readable through the control channel).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtboardUi {
    #[serde(skip)]
    pub drag: Option<BoardDrag>,
}

/// A gesture in progress (document coordinates).
#[derive(Clone, Debug, PartialEq)]
pub enum BoardDrag {
    /// Drawing a new board from the pasteboard.
    New { start: [f64; 2], cur: [f64; 2] },
    /// Moving a board (`copy`: ⌥ held, a copy moves and the original stays).
    Move { id: LayerId, start: [f64; 2], cur: [f64; 2], rect: photocraft_geom::Rect, copy: bool },
    /// Dragging handle `(hx, hy)` (each -1, 0 or 1: which edges move) of a board.
    Resize { id: LayerId, handle: (i8, i8), start: [f64; 2], cur: [f64; 2], rect: photocraft_geom::Rect },
    /// Pressed a + button; released over it, adds a board on that side.
    Plus { id: LayerId, side: &'static str },
}

/// Sides with a + button, in drawing order.
pub const SIDES: [&str; 4] = ["right", "below", "left", "above"];

/// Screen points between a board's edge and its + buttons, and their radius.
const PLUS_OFFSET: f32 = 22.0;
const PLUS_RADIUS: f32 = 9.0;
/// Screen points a handle reaches from its centre.
const HANDLE: f32 = 6.0;

/// The selected board: the active layer's artboard.
pub fn selected(app: &PhotocraftApp) -> Option<(LayerId, photocraft_geom::Rect)> {
    let st = app.session.active()?;
    let id = st.doc.artboard_of(st.active_layer?)?;
    Some((id, st.doc.layer(id)?.artboard()?.rect))
}

/// The topmost board under document point `(x, y)`.
pub fn board_at(doc: &Document, x: f64, y: f64) -> Option<(LayerId, photocraft_geom::Rect)> {
    // `artboards()` lists the Layers panel from the bottom; the last hit is on top.
    doc.artboards().into_iter().rfind(|b| contains(b.2.rect, x, y)).map(|b| (b.0, b.2.rect))
}

fn contains(r: photocraft_geom::Rect, x: f64, y: f64) -> bool {
    x >= f64::from(r.x0) && x < f64::from(r.x1) && y >= f64::from(r.y0) && y < f64::from(r.y1)
}

/// Document point of handle `(hx, hy)` of `r`.
pub(crate) fn handle_point(r: photocraft_geom::Rect, (hx, hy): (i8, i8)) -> [f64; 2] {
    let pick = |a: i32, b: i32, h: i8| match h {
        -1 => f64::from(a),
        1 => f64::from(b),
        _ => (f64::from(a) + f64::from(b)) / 2.0,
    };
    [pick(r.x0, r.x1, hx), pick(r.y0, r.y1, hy)]
}

/// The eight handles, corners first.
pub(crate) const HANDLES: [(i8, i8); 8] = [(-1, -1), (1, -1), (1, 1), (-1, 1), (0, -1), (1, 0), (0, 1), (-1, 0)];

/// The handle of `r` within `tol` document pixels of `(x, y)`.
pub(crate) fn handle_at(r: photocraft_geom::Rect, x: f64, y: f64, tol: f64) -> Option<(i8, i8)> {
    HANDLES.into_iter().find(|h| {
        let p = handle_point(r, *h);
        (p[0] - x).abs() <= tol && (p[1] - y).abs() <= tol
    })
}

/// Centre of the + button on `side` of `r`, `offset` document pixels outside it.
fn plus_point(r: photocraft_geom::Rect, side: &str, offset: f64) -> [f64; 2] {
    let [cx, cy] = handle_point(r, (0, 0));
    match side {
        "right" => [f64::from(r.x1) + offset, cy],
        "left" => [f64::from(r.x0) - offset, cy],
        "below" => [cx, f64::from(r.y1) + offset],
        _ => [cx, f64::from(r.y0) - offset],
    }
}

/// The + buttons a board offers: the sides with room for a board of its size.
pub fn plus_sides(doc: &Document, r: photocraft_geom::Rect) -> Vec<&'static str> {
    let (w, h) = (i64::from(r.width()), i64::from(r.height()));
    SIDES.into_iter().filter(|s| photocraft_engine::artboard_cmds::slot_beside(doc, r, w, h, s).is_some()).collect()
}

/// Screen points per document pixel (never zero).
fn scale(app: &PhotocraftApp) -> f64 {
    let z = f64::from(app.point_zoom());
    if z.is_finite() && z > 1e-6 { z } else { 1.0 }
}

fn rect_from(a: [f64; 2], b: [f64; 2]) -> photocraft_geom::Rect {
    let (x0, x1) = (a[0].min(b[0]).round() as i32, a[0].max(b[0]).round() as i32);
    let (y0, y1) = (a[1].min(b[1]).round() as i32, a[1].max(b[1]).round() as i32);
    photocraft_geom::Rect::new(x0, y0, x1, y1)
}

/// `rect` with handle `h` dragged by `(dx, dy)` document pixels: the edges it names move, and
/// the board never shrinks below one pixel or flips.
pub fn resized(rect: photocraft_geom::Rect, (hx, hy): (i8, i8), dx: i32, dy: i32) -> photocraft_geom::Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    match hx {
        -1 => x0 = x0.saturating_add(dx).min(x1.saturating_sub(1)),
        1 => x1 = x1.saturating_add(dx).max(x0.saturating_add(1)),
        _ => {}
    }
    match hy {
        -1 => y0 = y0.saturating_add(dy).min(y1.saturating_sub(1)),
        1 => y1 = y1.saturating_add(dy).max(y0.saturating_add(1)),
        _ => {}
    }
    photocraft_geom::Rect::new(x0.max(0), y0.max(0), x1, y1)
}

/// Rounded drag offset, clamped so the board stays right of and below the origin.
fn offset(start: [f64; 2], end: [f64; 2], rect: photocraft_geom::Rect) -> (i32, i32) {
    let d = |a: f64, b: f64, min: i32| {
        let v = (b - a).round();
        let v = if v.is_finite() { v.clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32 } else { 0 };
        v.max(min.saturating_neg())
    };
    (d(start[0], end[0], rect.x0), d(start[1], end[1], rect.y0))
}

/// Pointer input for the Artboard tool. Returns true when the event was its.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.ui.tool != Tool::Artboard {
        return false;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return true };
    let z = scale(app);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let sel = selected(app);
            let plus = sel.and_then(|(id, r)| {
                let tol = f64::from(PLUS_RADIUS + 2.0) / z;
                plus_sides(&doc, r)
                    .into_iter()
                    .find(|s| {
                        let p = plus_point(r, s, f64::from(PLUS_OFFSET) / z);
                        (p[0] - x).hypot(p[1] - y) <= tol
                    })
                    .map(|side| BoardDrag::Plus { id, side })
            });
            let handle = sel.and_then(|(id, r)| {
                handle_at(r, x, y, f64::from(HANDLE) / z).map(|handle| BoardDrag::Resize { id, handle, start: [x, y], cur: [x, y], rect: r })
            });
            app.ui.artboards.drag = Some(match (plus, handle, board_at(&doc, x, y)) {
                (Some(p), _, _) => p,
                (None, Some(h), _) => h,
                (None, None, Some((id, rect))) => {
                    if sel.map(|s| s.0) != Some(id) {
                        let _ = app.run("layer.select", json!({"layer": id.0}));
                    }
                    BoardDrag::Move { id, start: [x, y], cur: [x, y], rect, copy: mods.alt }
                }
                (None, None, None) => BoardDrag::New { start: [x, y], cur: [x, y] },
            });
        }
        ToolEvent::Move { x, y, .. } => match &mut app.ui.artboards.drag {
            Some(BoardDrag::New { cur, .. } | BoardDrag::Move { cur, .. } | BoardDrag::Resize { cur, .. }) => *cur = [x, y],
            Some(BoardDrag::Plus { .. }) | None => {}
        },
        ToolEvent::Up { x, y } => match app.ui.artboards.drag.take() {
            Some(BoardDrag::New { start, .. }) => {
                let r = rect_from(start, [x.max(0.0), y.max(0.0)]);
                let r = photocraft_geom::Rect::new(r.x0.max(0), r.y0.max(0), r.x1, r.y1);
                // A click (or a sliver) on the pasteboard draws nothing.
                if f64::from(r.width()) * z >= 4.0 && f64::from(r.height()) * z >= 4.0 {
                    let _ = app.run("layer.new.artboard", json!({"rect": [r.x0, r.y0, r.width(), r.height()]}));
                }
            }
            Some(BoardDrag::Move { id, start, rect, copy, .. }) => {
                let (dx, dy) = offset(start, [x, y], rect);
                if (dx, dy) != (0, 0) {
                    let target = if copy {
                        app.run("layer.duplicate", json!({"layer": id.0, "inPlace": true}))
                            .ok()
                            .and_then(|_| app.session.active().and_then(|st| st.active_layer))
                    } else {
                        Some(id)
                    };
                    if let Some(t) = target {
                        let _ = app.run("layer.artboard.set", json!({"layer": t.0, "x": rect.x0.saturating_add(dx), "y": rect.y0.saturating_add(dy)}));
                    }
                }
            }
            Some(BoardDrag::Resize { id, handle, start, rect, .. }) => {
                let (dx, dy) = ((x - start[0]).round(), (y - start[1]).round());
                let (dx, dy) = (dx.clamp(-1e9, 1e9) as i32, dy.clamp(-1e9, 1e9) as i32);
                let r = resized(rect, handle, dx, dy);
                if r != rect {
                    let _ = app.run("layer.artboard.set", json!({"layer": id.0, "rect": [r.x0, r.y0, r.width(), r.height()], "moveContents": false}));
                }
            }
            Some(BoardDrag::Plus { id, side }) => {
                let r = app.session.active().and_then(|st| st.doc.layer(id)).and_then(|l| l.artboard()).map(|a| a.rect);
                let over = r.is_some_and(|r| {
                    let p = plus_point(r, side, f64::from(PLUS_OFFSET) / z);
                    (p[0] - x).hypot(p[1] - y) <= f64::from(PLUS_RADIUS + 2.0) / z
                });
                if over {
                    let _ = app.run("layer.new.artboard", json!({"beside": id.0, "side": side}));
                }
            }
            None => {}
        },
    }
    true
}

/// The selected board's outline, handles and + buttons, and the outline of a gesture in progress.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::Artboard {
        return;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let t = crate::theme::Tokens::get(painter.ctx());
    let to_screen = |r: photocraft_geom::Rect| Rect::from_two_pos(xf.to_screen(r.x0 as f32, r.y0 as f32), xf.to_screen(r.x1 as f32, r.y1 as f32));
    let pt = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let line = Stroke::new(1.5, t.accent);
    let z = scale(app);
    let gesture = match &app.ui.artboards.drag {
        Some(BoardDrag::New { start, cur }) => Some(rect_from(*start, *cur)),
        Some(BoardDrag::Move { start, cur, rect, .. }) => {
            let (dx, dy) = offset(*start, *cur, *rect);
            Some(rect.translate(dx, dy))
        }
        Some(BoardDrag::Resize { start, cur, rect, handle, .. }) => {
            let (dx, dy) = ((cur[0] - start[0]).round().clamp(-1e9, 1e9) as i32, (cur[1] - start[1]).round().clamp(-1e9, 1e9) as i32);
            Some(resized(*rect, *handle, dx, dy))
        }
        _ => None,
    };
    if let Some(g) = gesture {
        painter.rect_stroke(to_screen(g), 0.0, line, egui::StrokeKind::Middle);
        return;
    }
    let Some((_, r)) = selected(app) else { return };
    painter.rect_stroke(to_screen(r), 0.0, line, egui::StrokeKind::Middle);
    for h in HANDLES {
        let c = pt(handle_point(r, h));
        let hr = Rect::from_center_size(c, vec2(7.0, 7.0));
        painter.rect_filled(hr, 0.0, Color32::WHITE);
        painter.rect_stroke(hr, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    for side in plus_sides(&doc, r) {
        let c: Pos2 = pt(plus_point(r, side, f64::from(PLUS_OFFSET) / z));
        painter.circle(c, PLUS_RADIUS, t.accent, Stroke::NONE);
        let arm = PLUS_RADIUS * 0.5;
        let s = Stroke::new(1.5, Color32::WHITE);
        painter.line_segment([c - vec2(arm, 0.0), c + vec2(arm, 0.0)], s);
        painter.line_segment([c - vec2(0.0, arm), c + vec2(0.0, arm)], s);
    }
}

#[cfg(test)]
#[path = "artboard_tool_tests.rs"]
mod tests;
