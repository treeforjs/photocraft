//! Edit › Content-Aware Scale (TOOL-214-7): a transform box on the image under the dialog.
//! Dragging a handle resizes the box, dragging inside it moves it (⇧ on a corner keeps the
//! proportions); the dialog's X / Y / Width / Height follow, and so does the live proxy preview,
//! which runs the same `edit.contentAwareScale` on the downsampled document.

use egui::{Color32, Pos2, Rect, Stroke, vec2};
use photocraft_geom::Rect as DocRect;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::artboard_tool::{HANDLES, handle_at, handle_point, resized};
use crate::canvas::ViewXform;

pub const COMMAND: &str = "edit.contentAwareScale";

/// Handle grab distance in screen points.
const GRAB: f64 = 7.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    /// `None` moves the box.
    handle: Option<(i8, i8)>,
    start: [f64; 2],
    rect: DocRect,
}

fn drag_id() -> egui::Id {
    egui::Id::new("pc-ca-scale-drag")
}

fn int(f: &Map<String, Value>, k: &str) -> Option<i32> {
    f.get(k)?.as_f64().filter(|v| v.is_finite()).map(|v| v.round().clamp(-60_000.0, 60_000.0) as i32)
}

/// The box the dialog's fields describe.
pub fn box_of(f: &Map<String, Value>) -> Option<DocRect> {
    let (x, y) = (int(f, "x")?, int(f, "y")?);
    let (w, h) = (int(f, "width")?.max(1), int(f, "height")?.max(1));
    Some(DocRect::new(x, y, x.saturating_add(w), y.saturating_add(h)))
}

fn set_box(f: &mut Map<String, Value>, r: DocRect) {
    f.insert("x".into(), json!(r.x0));
    f.insert("y".into(), json!(r.y0));
    f.insert("width".into(), json!(r.width().max(1)));
    f.insert("height".into(), json!(r.height().max(1)));
}

/// The box of the top dialog when it is Content-Aware Scale.
pub fn active(app: &PhotocraftApp) -> Option<DocRect> {
    let d = app.ui.dialogs.last()?;
    (d.fields.get("__command").and_then(Value::as_str) == Some(COMMAND)).then(|| box_of(&d.fields)).flatten()
}

/// Screen points per document pixel (never zero).
fn scale(app: &PhotocraftApp) -> f64 {
    let z = f64::from(app.point_zoom());
    if z.is_finite() && z > 1e-6 { z } else { 1.0 }
}

/// `rect` resized by `handle` (or moved) for a drag from `start` to `cur`, document pixels.
fn dragged(d: &Drag, cur: [f64; 2], keep_ratio: bool) -> DocRect {
    let delta = |a: f64, b: f64| {
        let v = (b - a).round();
        if v.is_finite() { v.clamp(-120_000.0, 120_000.0) as i32 } else { 0 }
    };
    let (dx, dy) = (delta(d.start[0], cur[0]), delta(d.start[1], cur[1]));
    let Some(h) = d.handle else {
        let r = d.rect.translate(dx, dy);
        // Keep the box on the 1..60000 px coordinate range the command accepts.
        let fix = |v: i32| v.clamp(-60_000, 60_000);
        return DocRect::from_xywh(fix(r.x0), fix(r.y0), d.rect.width(), d.rect.height());
    };
    let mut out = resized(d.rect, h, dx, dy);
    if keep_ratio && h.0 != 0 && h.1 != 0 && d.rect.width() > 0 && d.rect.height() > 0 {
        // ⇧ on a corner: the height follows the width.
        let ratio = f64::from(d.rect.height()) / f64::from(d.rect.width());
        let nh = (f64::from(out.width()) * ratio).round().clamp(1.0, 60_000.0) as i32;
        out = if h.1 < 0 {
            DocRect::new(out.x0, out.y1.saturating_sub(nh).max(0), out.x1, out.y1)
        } else {
            DocRect::new(out.x0, out.y0, out.x1, out.y0.saturating_add(nh))
        };
    }
    out
}

/// Pointer on the free canvas under the dialog: `p` is where it hovers, `press` where the primary
/// button is held this frame. Returns the cursor to show.
pub fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, p: Pos2, press: Option<Pos2>) -> Option<egui::CursorIcon> {
    let rect = active(app)?;
    let tol = GRAB / scale(app);
    let (pressed, down, shift) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift));
    let mut drag: Option<Drag> = ctx.data(|m| m.get_temp(drag_id()));
    if !down {
        drag = None;
    }
    if pressed && let Some(q) = press {
        let d = xf.to_doc(q);
        let handle = handle_at(rect, d[0], d[1], tol);
        let inside = d[0] >= f64::from(rect.x0) && d[0] < f64::from(rect.x1) && d[1] >= f64::from(rect.y0) && d[1] < f64::from(rect.y1);
        drag = (handle.is_some() || inside).then_some(Drag { handle, start: d, rect });
    }
    if let (Some(d), Some(q)) = (drag, press) {
        let next = dragged(&d, xf.to_doc(q), shift);
        if next != rect
            && let Some(dlg) = app.ui.dialogs.last_mut()
        {
            set_box(&mut dlg.fields, next);
        }
    }
    ctx.data_mut(|m| match drag {
        Some(d) => {
            m.insert_temp(drag_id(), d);
        }
        None => {
            m.remove::<Drag>(drag_id());
        }
    });
    let d = xf.to_doc(p);
    Some(match drag.map(|d| d.handle).unwrap_or_else(|| handle_at(rect, d[0], d[1], tol)) {
        Some((hx, hy)) if hx * hy > 0 => egui::CursorIcon::ResizeNwSe,
        Some((hx, hy)) if hx * hy < 0 => egui::CursorIcon::ResizeNeSw,
        Some((0, _)) => egui::CursorIcon::ResizeVertical,
        Some(_) => egui::CursorIcon::ResizeHorizontal,
        None if d[0] >= f64::from(rect.x0) && d[0] < f64::from(rect.x1) && d[1] >= f64::from(rect.y0) && d[1] < f64::from(rect.y1) => egui::CursorIcon::Move,
        None => egui::CursorIcon::Default,
    })
}

/// Draws the box and its eight handles.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(r) = active(app) else { return };
    let t = crate::theme::Tokens::get(painter.ctx());
    let pt = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let corners = [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|h| pt(handle_point(r, h)));
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(90)));
        painter.line_segment([a, b], Stroke::new(1.0, t.accent));
    }
    for h in HANDLES {
        let hr = Rect::from_center_size(pt(handle_point(r, h)), vec2(7.0, 7.0));
        painter.rect_filled(hr, 0.0, Color32::WHITE);
        painter.rect_stroke(hr, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
}

/// Proxy previews run at 1/`k` scale: the box moves with them (`height` is scaled with the other
/// pixel parameters already).
pub fn scale_params(params: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let Some(o) = params.as_object_mut() else { return };
    for key in ["x", "y", "width"] {
        if let Some(v) = o.get(key).and_then(Value::as_f64) {
            let s = v / f64::from(k);
            o.insert(key.into(), json!(if key == "width" { s.max(1.0) } else { s }));
        }
    }
}

#[cfg(test)]
#[path = "ca_scale_ui_tests.rs"]
mod tests;
