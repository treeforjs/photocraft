//! Layer Style contours: the picker row (a curve thumbnail and the preset list) and the Contour
//! Editor page. The editor's state lives in the dialog's fields under [`EDITOR`] as
//! `{effect, field, name, points: [[in, out], …], selected}` (inputs and outputs in `0..=1`), so
//! automation drives it like the rest of the dialog. OK writes the curve to the effect as
//! `{"name", "points"}`; Save Preset… keeps it in the user's contour presets
//! (`contour.presets.new`).

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::adjust::CurvePoint;
use photocraft_doc::effects::Contour;
use photocraft_engine::presets::contours::{MAX_POINTS, contour_json, contour_value, sanitize};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};

/// The dialog field holding the open editor.
pub const EDITOR: &str = "contourEditor";
/// The dropdown key of a current contour that is neither a built-in nor a saved preset.
const CURRENT: &str = "\u{0}current";
/// A pointer this far outside the graph while dragging a point removes it (as in Curves).
const REMOVE_MARGIN: f32 = 16.0;
const GRAPH: f32 = 220.0;

/// A saved contour in the picker: `(dropdown key, name, points)`.
type Saved = (String, String, Vec<(f32, f32)>);

/// The name and points of a contour param; `None` when the engine wouldn't take it.
fn named_points(v: &Value) -> Option<(String, Vec<(f32, f32)>)> {
    Some(match contour_value(v)? {
        Contour::Linear => ("Linear".to_string(), vec![(0.0, 0.0), (1.0, 1.0)]),
        Contour::Custom { name, points } => (name, points.iter().map(|p| (p.input, p.output)).collect()),
    })
}

/// The param form of an edited curve (a built-in's name when it is exactly one).
fn param_of(name: &str, points: &[(f32, f32)]) -> Option<Value> {
    let v = json!({"name": name, "points": points.iter().map(|(i, o)| json!([i, o])).collect::<Vec<_>>()});
    contour_value(&v).map(|c| contour_json(&c))
}

/// Draws the contour's curve into `rect` (the same spline the effects apply).
fn paint_curve(painter: &egui::Painter, rect: Rect, points: &[(f32, f32)], color: Color32, width: f32) {
    let cps: Vec<CurvePoint> = points.iter().map(|&(input, output)| CurvePoint { input, output }).collect();
    if cps.len() < 2 {
        return;
    }
    let lut = photocraft_compose::adjust::curve_lut(&cps);
    let steps = 64usize;
    let line: Vec<Pos2> = (0..=steps)
        .map(|k| {
            let x = k as f32 / steps as f32;
            let at = ((x * lut.len().saturating_sub(1) as f32).round() as usize).min(lut.len().saturating_sub(1));
            to_screen(rect, (x, lut.get(at).copied().unwrap_or(x)))
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(width, color)));
}

fn to_screen(rect: Rect, (i, o): (f32, f32)) -> Pos2 {
    pos2(rect.left() + i * rect.width(), rect.bottom() - o * rect.height())
}

fn from_screen(rect: Rect, p: Pos2) -> (f32, f32) {
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    (((p.x - rect.left()) / w).clamp(0.0, 1.0), ((rect.bottom() - p.y) / h).clamp(0.0, 1.0))
}

/// The user's saved contours: `(dropdown key, name, points)`.
fn saved(app: &PhotocraftApp) -> Vec<Saved> {
    let mut out = Vec::new();
    for (gi, g) in app.session.presets.contours.iter().enumerate() {
        for (ii, c) in g.items.iter().enumerate() {
            out.push((format!("preset:{gi}:{ii}"), c.name.clone(), c.points.clone()));
        }
    }
    out
}

/// One contour row: label, a thumbnail that opens the editor, and the list of built-in and saved
/// contours. Returns the new param value when one was picked.
pub fn row(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, effect: &str, key: &str, label: &str, cur: &Value) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    let current = named_points(cur);
    let saved = saved(app);
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!(label)).color(t.text_dim));
        let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 26.0), Sense::click());
        ui.painter().rect(rect, t.radius_sm, t.field, Stroke::new(1.0, if resp.hovered() { t.accent } else { t.field_border }), StrokeKind::Inside);
        if let Some((_, points)) = &current {
            paint_curve(ui.painter(), rect.shrink(4.0), points, t.text, 1.5);
        }
        let resp = resp.on_hover_text(tl!("Edit Contour…"));
        if resp.clicked() {
            let (name, points) = current.clone().unwrap_or_else(|| ("Custom".to_string(), vec![(0.0, 0.0), (1.0, 1.0)]));
            open(f, effect, key, &name, &points);
        }
        let mut opts: Vec<(String, String)> = photocraft_engine::layer_style::CONTOURS.iter().map(|(n, _)| (n.to_string(), n.to_string())).collect();
        opts.extend(saved.iter().map(|(k, n, _)| (k.clone(), n.clone())));
        let mut sel = match (cur, &current) {
            (Value::String(s), _) if opts.iter().any(|(k, _)| k == s) => s.clone(),
            (_, Some((name, points))) => saved.iter().find(|(_, n, p)| n == name && p == points).map(|(k, _, _)| k.clone()).unwrap_or_else(|| CURRENT.into()),
            _ => CURRENT.into(),
        };
        if sel == CURRENT {
            let name = current.as_ref().map_or_else(|| cur.as_str().unwrap_or("Custom").to_string(), |c| c.0.clone());
            opts.push((CURRENT.into(), name));
        }
        let shown: Vec<(String, &str)> = opts.iter().map(|(k, n)| (k.clone(), n.as_str())).collect();
        if widgets::dropdown(ui, &format!("fx-{effect}-{key}"), &mut sel, &shown, 150.0) {
            picked = if let Some((_, name, points)) = saved.iter().find(|(k, _, _)| *k == sel) {
                param_of(name, points)
            } else if sel == CURRENT {
                None
            } else {
                Some(json!(sel))
            };
        }
    });
    picked
}

/// Opens the editor on `points` for `effect`'s `field`.
pub fn open(f: &mut Map<String, Value>, effect: &str, field: &str, name: &str, points: &[(f32, f32)]) {
    let pts: Vec<Value> = points.iter().map(|(i, o)| json!([i, o])).collect();
    f.insert(EDITOR.into(), json!({"effect": effect, "field": field, "name": name, "points": pts, "selected": Value::Null}));
}

/// The editor's points, as far as they are usable (hostile `ui.dialog.set` state is dropped).
fn editor_points(e: &Value) -> Vec<(f32, f32)> {
    let raw: Vec<(f32, f32)> = e
        .get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_POINTS * 4).filter_map(|p| Some((p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))).collect())
        .unwrap_or_default();
    sanitize(&raw).unwrap_or_else(|| vec![(0.0, 0.0), (1.0, 1.0)])
}

/// An edit the editor asks the dialog to make: write `value` to `effect`'s `field`.
pub struct Apply {
    pub effect: String,
    pub field: String,
    pub value: Value,
}

/// Draws the open editor in place of the effect's page. `None` when no editor is open;
/// `Some(None)` while it stays open or was cancelled; `Some(Some(apply))` on OK.
pub fn editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<Option<Apply>> {
    let e = f.get(EDITOR)?.clone();
    let Some(eo) = e.as_object() else {
        f.remove(EDITOR);
        return None;
    };
    let t = Tokens::get(ui.ctx());
    let effect = eo.get("effect").and_then(Value::as_str).unwrap_or_default().to_string();
    let field = eo.get("field").and_then(Value::as_str).unwrap_or("contour").to_string();
    let mut name = eo.get("name").and_then(Value::as_str).unwrap_or("Custom").to_string();
    let mut points = editor_points(&e);
    let mut selected = eo.get("selected").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok()).filter(|&s| s < points.len());
    let mut result = None;
    let mut close = false;

    ui.label(RichText::new(tl!("Contour Editor")).strong().color(t.text));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Name:")).color(t.text_dim));
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0).char_limit(64));
    });
    ui.add_space(4.0);

    let (outer, resp) = ui.allocate_exact_size(vec2(GRAPH, GRAPH), Sense::click_and_drag());
    let rect = outer.shrink(6.0);
    let painter = ui.painter_at(outer.expand(4.0));
    painter.rect(outer, t.radius_sm, t.field, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    for k in 1..4 {
        let x = rect.left() + rect.width() * k as f32 / 4.0;
        let y = rect.top() + rect.height() * k as f32 / 4.0;
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, t.separator));
    }
    // Press on a point to pick it, elsewhere to add one; drag it far outside the graph to remove it.
    if (resp.drag_started() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let near = points.iter().position(|&q| to_screen(rect, q).distance(p) <= 7.0);
        selected = match near {
            Some(i) => Some(i),
            None if points.len() < MAX_POINTS => {
                let q = from_screen(rect, p);
                let at = points.iter().position(|&(i, _)| i > q.0).unwrap_or(points.len());
                if points.iter().any(|&(i, _)| (i - q.0).abs() < 0.01) {
                    None
                } else {
                    points.insert(at, q);
                    Some(at)
                }
            }
            None => None,
        };
    }
    if resp.dragged()
        && let (Some(i), Some(p)) = (selected, resp.interact_pointer_pos())
    {
        if !outer.expand(REMOVE_MARGIN).contains(p) && points.len() > 2 {
            points.remove(i);
            selected = None;
        } else {
            // A point stays between its neighbours, so the curve stays a function of its input.
            let (x, y) = from_screen(rect, p);
            let lo = if i == 0 { 0.0 } else { points.get(i - 1).map_or(0.0, |q| q.0 + 0.01) };
            let hi = points.get(i + 1).map_or(1.0, |q| q.0 - 0.01);
            if let Some(q) = points.get_mut(i) {
                *q = (x.clamp(lo, hi.max(lo)), y);
            }
        }
    }
    paint_curve(&painter, rect, &points, t.text, 1.5);
    for (i, &q) in points.iter().enumerate() {
        let c = to_screen(rect, q);
        if Some(i) == selected {
            painter.circle_filled(c, 4.5, t.accent);
        } else {
            painter.circle(c, 4.0, t.field, Stroke::new(1.5, t.text));
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let Some(&(mut i, mut o)) = selected.and_then(|s| points.get(s)) else {
            ui.label(RichText::new(tl!("Input:")).color(t.text_faint));
            return;
        };
        let (mut ip, mut op) = (i * 100.0, o * 100.0);
        ui.label(RichText::new(tl!("Input:")).color(t.text_dim));
        let a = ui.add(egui::DragValue::new(&mut ip).range(0.0..=100.0).speed(0.5).suffix("%")).changed();
        ui.label(RichText::new(tl!("Output:")).color(t.text_dim));
        let b = ui.add(egui::DragValue::new(&mut op).range(0.0..=100.0).speed(0.5).suffix("%")).changed();
        if (a || b)
            && let Some(s) = selected
        {
            let lo = if s == 0 { 0.0 } else { points.get(s - 1).map_or(0.0, |q| q.0 + 0.01) };
            let hi = points.get(s + 1).map_or(1.0, |q| q.0 - 0.01);
            i = (ip / 100.0).clamp(lo, hi.max(lo));
            o = (op / 100.0).clamp(0.0, 1.0);
            if let Some(q) = points.get_mut(s) {
                *q = (i, o);
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, tl!("OK"), 70.0).clicked() {
            // The editor keeps 2..=32 points with distinct inputs, so this only fails on a name
            // the engine refuses; the page then stays open.
            if let Some(value) = param_of(&name, &points) {
                result = Some(Apply { effect: effect.clone(), field: field.clone(), value });
                close = true;
            }
        }
        if widgets::secondary_button(ui, tl!("Cancel"), 70.0).clicked() {
            close = true;
        }
        if widgets::secondary_button(ui, tl!("Save Preset…"), 100.0).clicked() {
            let pts: Vec<Value> = points.iter().map(|(i, o)| json!([i, o])).collect();
            let preset = if name.trim().is_empty() { "Custom" } else { name.trim() };
            if let Err(err) = app.run("contour.presets.new", json!({"name": preset, "points": pts})) {
                app.ui.status = err;
            }
        }
    });

    if close {
        f.remove(EDITOR);
    } else {
        let pts: Vec<Value> = points.iter().map(|(i, o)| json!([i, o])).collect();
        f.insert(EDITOR.into(), json!({"effect": effect, "field": field, "name": name, "points": pts, "selected": selected}));
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_editor_state_falls_back_to_linear() {
        assert_eq!(editor_points(&json!({"points": "x"})), vec![(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(editor_points(&json!({"points": [[0.2, 0.4], [f64::MAX, 1], ["a", 2], [0.9, 0.1]]})), vec![(0.2, 0.4), (0.9, 0.1)]);
        assert_eq!(param_of("Cone", &[(0.0, 0.0), (0.5, 1.0), (1.0, 1.0)]), Some(json!("Cone")));
        assert_eq!(param_of("Mine", &[(0.0, 0.0), (1.0, 0.5)]), Some(json!({"name": "Mine", "points": [[0.0, 0.0], [1.0, 0.5]]})));
        assert_eq!(param_of("Bad", &[(0.5, 0.5)]), None);
    }
}
