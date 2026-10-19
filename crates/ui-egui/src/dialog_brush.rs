//! A brush that paints on the image under an open dialog and collects its dabs into one of the
//! dialog's command parameters: Select and Mask's Refine Edge brush (`edgeBrush`) and Content-Aware
//! Fill's Sampling Brush (`sampleBrush`). A dialog opts in by naming the parameter in [`PARAM`];
//! the brush paints only while armed, from presses that start on the free canvas (⌥ erases).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// The command parameter the dabs go to (`[[x, y, radius, add], …]`, document pixels).
pub const PARAM: &str = "__brushParam";
/// Checkbox label naming the brush ("Refine Edge Brush").
pub const LABEL: &str = "__brushLabel";
/// One-line hint shown while armed.
pub const HINT: &str = "__brushHint";
/// Show the strokes on the image (green adds, red erases).
pub const OVERLAY: &str = "__brushOverlay";
pub const ARMED: &str = "__brushArmed";
pub const SIZE: &str = "__brushSize";

/// Every parameter a dialog brush fills, for previews that rescale them.
const PARAMS: [&str; 2] = ["edgeBrush", "sampleBrush"];

/// Strokes drawn per frame at most (the newest).
const DRAWN: usize = 4000;

/// Makes dialog fields `f` carry a brush filling `param`.
pub fn attach(f: &mut Map<String, Value>, param: &str, label: &str, hint: &str, overlay: bool) {
    f.insert(PARAM.into(), json!(param));
    f.insert(LABEL.into(), json!(label));
    f.insert(HINT.into(), json!(hint));
    f.insert(OVERLAY.into(), json!(overlay));
}

fn param(f: &Map<String, Value>) -> Option<&str> {
    f.get(PARAM)?.as_str().filter(|p| PARAMS.contains(p))
}

fn size_of(f: &Map<String, Value>) -> f32 {
    f.get(SIZE).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(40.0, |v| v.clamp(1.0, 500.0) as f32)
}

fn strokes(f: &Map<String, Value>) -> Option<&Vec<Value>> {
    f.get(param(f)?)?.as_array()
}

/// The brush's checkbox, Size slider and hint, and Clear Strokes once anything is painted.
pub fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let Some(key) = param(f).map(str::to_string) else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    let label = f.get(LABEL).and_then(Value::as_str).unwrap_or("Brush").to_string();
    let mut armed = f.get(ARMED).and_then(Value::as_bool).unwrap_or(false);
    crate::widgets::checkbox(ui, &mut armed, tl!(&label));
    f.insert(ARMED.into(), json!(armed));
    if armed {
        let mut size = size_of(f);
        crate::widgets::slider_row(ui, tl!("Size"), &mut size, 1.0..=500.0, "px", None);
        f.insert(SIZE.into(), json!(size));
        if let Some(hint) = f.get(HINT).and_then(Value::as_str).filter(|h| !h.is_empty()) {
            ui.label(egui::RichText::new(tl!(hint)).color(t.text_dim).small());
        }
    }
    if strokes(f).is_some_and(|s| !s.is_empty()) && crate::widgets::secondary_button(ui, tl!("Clear Strokes"), 0.0).clicked() {
        f.remove(&key);
    }
    ui.separator();
}

/// The brush diameter in document pixels while the top dialog's brush is armed.
pub fn armed(app: &PhotocraftApp) -> Option<f32> {
    let d = app.ui.dialogs.last()?;
    (param(&d.fields).is_some() && d.fields.get(ARMED).and_then(Value::as_bool) == Some(true)).then(|| size_of(&d.fields))
}

/// One brush position on the image (document pixels) during a drag: adds a dab, or with `erase`
/// an erasing one, unless it is too close to the previous dab to change anything.
pub fn paint(app: &mut PhotocraftApp, at: [f32; 2], erase: bool) {
    let Some(size) = armed(app) else { return };
    let Some(d) = app.ui.dialogs.last_mut() else { return };
    let Some(key) = param(&d.fields).map(str::to_string) else { return };
    let r = size / 2.0;
    let entry = d.fields.entry(key).or_insert_with(|| json!([]));
    let Some(list) = entry.as_array_mut() else { return };
    if list.len() >= photocraft_engine::smartselect_cmds::MAX_DABS {
        return;
    }
    let add = !erase;
    let close = list.last().and_then(Value::as_array).is_some_and(|l| {
        let n = |i: usize| l.get(i).and_then(Value::as_f64).unwrap_or(f64::NAN) as f32;
        let same = l.get(3).and_then(Value::as_bool).unwrap_or(true) == add;
        same && (n(0) - at[0]).hypot(n(1) - at[1]) < (r * 0.25).max(0.5) && (n(2) - r).abs() < 0.01
    });
    if !close && at.iter().all(|v| v.is_finite()) {
        list.push(json!([at[0], at[1], r, add]));
    }
}

/// Proxy previews run at 1/`k` scale: scales any brush dabs in `params` to match.
pub fn scale(params: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let k = f64::from(k);
    for key in PARAMS {
        let Some(list) = params.get_mut(key).and_then(Value::as_array_mut) else { continue };
        for dab in list.iter_mut().filter_map(Value::as_array_mut) {
            for v in dab.iter_mut().take(3) {
                if let Some(n) = v.as_f64() {
                    *v = json!(n / k);
                }
            }
        }
    }
}

/// The painted strokes over the image, for brushes that show them.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(d) = app.ui.dialogs.last() else { return };
    if d.fields.get(OVERLAY).and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(list) = strokes(&d.fields) else { return };
    let start = list.len().saturating_sub(DRAWN);
    for dab in list.iter().skip(start).filter_map(Value::as_array) {
        let n = |i: usize| dab.get(i).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(-1e7, 1e7) as f32);
        let (Some(x), Some(y), Some(r)) = (n(0), n(1), n(2)) else { continue };
        let add = dab.get(3).and_then(Value::as_bool).unwrap_or(true);
        let c = xf.to_screen(x, y);
        let rs = (xf.to_screen(x + r, y) - c).length();
        if !rs.is_finite() || rs > 1e5 {
            continue;
        }
        let fill = if add { egui::Color32::from_rgba_unmultiplied(40, 200, 90, 70) } else { egui::Color32::from_rgba_unmultiplied(230, 60, 60, 70) };
        painter.circle_filled(c, rs.max(0.5), fill);
    }
}

/// The brush outline and cursor at `p` (screen) on the free canvas, and a dab on press.
pub fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, painter: &egui::Painter, p: egui::Pos2, press: Option<egui::Pos2>) {
    let Some(size) = armed(app) else { return };
    let d = xf.to_doc(p);
    let (dx, dy) = (d[0] as f32, d[1] as f32);
    let radius = (xf.to_screen(dx + size / 2.0, dy) - xf.to_screen(dx, dy)).length().max(1.0);
    painter.circle_stroke(p, radius + 1.0, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)));
    painter.circle_stroke(p, radius, egui::Stroke::new(1.0, egui::Color32::WHITE));
    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    if let Some(q) = press {
        let at = xf.to_doc(q);
        let erase = ctx.input(|i| i.modifiers.alt);
        paint(app, [at[0] as f32, at[1] as f32], erase);
    }
}

#[cfg(test)]
#[path = "dialog_brush_tests.rs"]
mod tests;
