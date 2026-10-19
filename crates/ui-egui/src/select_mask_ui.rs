//! Select › Select and Mask… (TOOL-214-1): the `select.refineEdge` dialog with Photoshop's view
//! modes over its live preview. The preview runs the refinement on the proxy document like every
//! filter preview, then [`apply_view`] restyles that proxy so the refined edge shows the way the
//! chosen view draws it. Only the preview copy changes; OK runs the plain command.

use photocraft_doc::{Color, Document, Fill, Layer, LayerContent, LayerId, LayerMask, PixelFormat};
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const COMMAND: &str = "select.refineEdge";

/// Photoshop's View Mode choices, in its order.
pub const VIEWS: [(&str, &str); 7] = [
    ("onionSkin", "Onion Skin"),
    ("marchingAnts", "Marching Ants"),
    ("overlay", "Overlay"),
    ("onBlack", "On Black"),
    ("onWhite", "On White"),
    ("blackWhite", "Black & White"),
    ("onLayers", "On Layers"),
];

const VIEW: &str = "__view";
const OPACITY: &str = "__viewOpacity";
/// The Refine Edge brush is armed: drags on the image paint `edgeBrush` dabs.
const BRUSH: &str = "__refineBrush";
const BRUSH_SIZE: &str = "__refineBrushSize";
/// The command parameter the brush fills.
const STROKES: &str = "edgeBrush";

/// Opens Select and Mask on the selection, previewing On Layers at the dialog's default opacity.
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let id = crate::filter_dialog::open(app, COMMAND)?;
    let d = app.ui.dialogs.iter_mut().find(|d| d.id == id)?;
    d.fields.insert("__label".into(), json!("Select and Mask"));
    d.fields.insert(VIEW.into(), json!("onLayers"));
    d.fields.insert(OPACITY.into(), json!(50.0));
    Some(id)
}

fn view_of(f: &Map<String, Value>) -> Option<&str> {
    let v = f.get(VIEW)?.as_str()?;
    VIEWS.iter().any(|(id, _)| *id == v).then_some(v)
}

fn opacity_of(f: &Map<String, Value>) -> f32 {
    f.get(OPACITY).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(50.0, |v| v.clamp(0.0, 100.0) as f32)
}

/// The View row (and the Opacity or Transparency slider of the views that use one) at the top of
/// the dialog.
pub fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let Some(view) = view_of(f).map(str::to_string) else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut cur = view;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("View")).color(t.text_dim));
        let opts: Vec<(String, &str)> = VIEWS.iter().map(|(id, name)| (id.to_string(), tl!(name))).collect();
        crate::widgets::dropdown(ui, "select-mask-view", &mut cur, &opts, 170.0);
    });
    let slider = match cur.as_str() {
        "onionSkin" => Some(tl!("Transparency")),
        "overlay" | "onBlack" | "onWhite" => Some(tl!("Opacity")),
        _ => None,
    };
    if let Some(name) = slider {
        let mut v = opacity_of(f);
        crate::widgets::slider_row(ui, name, &mut v, 0.0..=100.0, "%", None);
        f.insert(OPACITY.into(), json!(v));
    }
    f.insert(VIEW.into(), json!(cur));
    ui.add_space(2.0);
    let mut armed = f.get(BRUSH).and_then(Value::as_bool).unwrap_or(false);
    crate::widgets::checkbox(ui, &mut armed, tl!("Refine Edge Brush"));
    f.insert(BRUSH.into(), json!(armed));
    if armed {
        let mut size = brush_size(f);
        crate::widgets::slider_row(ui, tl!("Size"), &mut size, 1.0..=500.0, "px", None);
        f.insert(BRUSH_SIZE.into(), json!(size));
        ui.label(egui::RichText::new(tl!("Paint over hair and fur to refine them; hold Alt to erase strokes.")).color(t.text_dim).small());
    }
    if stroke_count(f) > 0 && crate::widgets::secondary_button(ui, tl!("Clear Strokes"), 0.0).clicked() {
        f.remove(STROKES);
    }
    ui.separator();
}

fn brush_size(f: &Map<String, Value>) -> f32 {
    f.get(BRUSH_SIZE).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(40.0, |v| v.clamp(1.0, 500.0) as f32)
}

fn stroke_count(f: &Map<String, Value>) -> usize {
    f.get(STROKES).and_then(Value::as_array).map_or(0, Vec::len)
}

/// The brush diameter in document pixels while the top dialog is Select and Mask with its Refine
/// Edge brush armed.
pub fn armed_brush(app: &PhotocraftApp) -> Option<f32> {
    let d = app.ui.dialogs.last()?;
    let ours = d.fields.get("__command").and_then(Value::as_str) == Some(COMMAND) && view_of(&d.fields).is_some();
    (ours && d.fields.get(BRUSH).and_then(Value::as_bool) == Some(true)).then(|| brush_size(&d.fields))
}

/// One brush position on the image (document pixels) during a drag: adds a dab, or with `erase`
/// an erasing one, unless it is too close to the previous dab to change anything.
pub fn paint(app: &mut PhotocraftApp, at: [f32; 2], erase: bool) {
    let Some(size) = armed_brush(app) else { return };
    let Some(d) = app.ui.dialogs.last_mut() else { return };
    let r = size / 2.0;
    let entry = d.fields.entry(STROKES).or_insert_with(|| json!([]));
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

/// Proxy previews run at 1/`k` scale: scales the brush dabs to match.
pub fn scale_strokes(params: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let k = f64::from(k);
    let Some(list) = params.get_mut(STROKES).and_then(Value::as_array_mut) else { return };
    for dab in list.iter_mut().filter_map(Value::as_array_mut) {
        for v in dab.iter_mut().take(3) {
            if let Some(n) = v.as_f64() {
                *v = json!(n / k);
            }
        }
    }
}

/// The preview request for the dialog's fields: the view rides along in private keys, which
/// [`take_view`] strips again before the command runs on the proxy.
pub fn preview_params(f: &Map<String, Value>, mut params: Value) -> Value {
    if let (Some(view), Some(o)) = (view_of(f), params.as_object_mut()) {
        o.insert(VIEW.into(), json!(view));
        o.insert(OPACITY.into(), json!(opacity_of(f)));
    }
    params
}

/// Removes the preview-only view keys from `params`. With a view, the preview shows the refined
/// mask, so the command runs with Output To Selection whatever the dialog's output is.
pub fn take_view(params: &mut Value) -> Option<(String, f32)> {
    let o = params.as_object_mut()?;
    let view = o.remove(VIEW).and_then(|v| v.as_str().map(str::to_string));
    let opacity = o.remove(OPACITY).and_then(|v| v.as_f64()).filter(|v| v.is_finite()).map_or(50.0, |v| v.clamp(0.0, 100.0) as f32);
    let view = view.filter(|v| VIEWS.iter().any(|(id, _)| id == v))?;
    o.insert("output".into(), json!("selection"));
    o.insert("decontaminate".into(), json!(false));
    Some((view, opacity / 100.0))
}

/// A GRAY8 mask over the document holding `f(coverage)` for every pixel's refined coverage.
fn mask_from(sel: Option<&Surface>, doc: &Document, f: impl Fn(f32) -> f32) -> LayerMask {
    let bounds = doc.bounds();
    let cover = photocraft_algo::selection::mask_from_surface(sel, bounds);
    let bytes: Vec<u8> = cover.iter().map(|c| (f(c.clamp(0.0, 1.0)).clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    let mut surface = Surface::from_interleaved(PixelFormat::GRAY8, bounds, &bytes);
    surface.prune();
    LayerMask { surface, ..LayerMask::reveal_all() }
}

fn solid(doc: &Document, name: &str, rgb: [f32; 3], mask: LayerMask) -> Layer {
    let fill = Fill::Solid(Color::rgba(rgb[0], rgb[1], rgb[2], 1.0)).in_mode(doc.mode);
    let mut l = Layer::new(name, LayerContent::Fill(fill));
    l.mask = Some(mask);
    l
}

/// Multiplies the active layer's mask (or full reveal) by `mask`.
fn mask_layer(doc: &mut Document, id: LayerId, mask: LayerMask) -> bool {
    let bounds = doc.bounds();
    let Some(layer) = doc.layer_mut(id) else { return false };
    let mask = match layer.mask.take() {
        Some(old) => {
            let mut v = mask.surface.read_region(bounds);
            let oldv = old.surface.read_region(bounds);
            let on = old.enabled;
            for (a, b) in v.iter_mut().zip(oldv) {
                *a *= if on { b * old.density + (1.0 - old.density) } else { 1.0 };
            }
            let bytes: Vec<u8> = v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            let mut surface = Surface::from_interleaved(PixelFormat::GRAY8, bounds, &bytes);
            surface.prune();
            LayerMask { surface, ..old }
        }
        None => mask,
    };
    layer.mask = Some(mask);
    true
}

/// Restyles the preview copy `doc`, whose selection holds the refined mask, as `view` draws it.
/// `opacity` (0–1) is the Overlay / On Black / On White opacity and the Onion Skin transparency.
pub fn apply_view(doc: &mut Document, active: Option<LayerId>, view: &str, opacity: f32) {
    if view == "marchingAnts" {
        return;
    }
    let sel = doc.selection.take();
    let outside = |o: f32| move |c: f32| (1.0 - c) * o;
    let top = match view {
        "overlay" => vec![solid(doc, "Overlay", [1.0, 0.0, 0.0], mask_from(sel.as_ref(), doc, outside(opacity)))],
        "onBlack" => vec![solid(doc, "On Black", [0.0; 3], mask_from(sel.as_ref(), doc, outside(opacity)))],
        "onWhite" => vec![solid(doc, "On White", [1.0; 3], mask_from(sel.as_ref(), doc, outside(opacity)))],
        "blackWhite" => vec![solid(doc, "White", [1.0; 3], mask_from(sel.as_ref(), doc, |c| c)), solid(doc, "Black", [0.0; 3], LayerMask::reveal_all())],
        _ => Vec::new(),
    };
    if !top.is_empty() {
        // Topmost first: layers are stored bottom to top.
        doc.layers.extend(top.into_iter().rev());
        return;
    }
    let Some(id) = active.filter(|id| doc.layer(*id).is_some()) else {
        // No layer to mask: show the selection as Overlay does.
        doc.selection = sel;
        return apply_view(doc, None, "overlay", opacity);
    };
    if view == "onionSkin" {
        // Only the active layer, outside the selection at (1 − transparency).
        let keep = 1.0 - opacity;
        let walk = doc.walk();
        let path = walk.iter().find(|(_, _, l)| l.id == id).map(|(p, _, _)| p.clone()).unwrap_or_default();
        // Everything but the active layer and the groups holding it.
        let others: Vec<LayerId> = walk.iter().filter(|(p, _, _)| !path.starts_with(p)).map(|(_, _, l)| l.id).collect();
        for lid in others {
            if let Some(l) = doc.layer_mut(lid) {
                l.visible = false;
            }
        }
        let m = mask_from(sel.as_ref(), doc, |c| c + (1.0 - c) * keep);
        mask_layer(doc, id, m);
    } else {
        let m = mask_from(sel.as_ref(), doc, |c| c);
        mask_layer(doc, id, m);
    }
}

#[cfg(test)]
#[path = "select_mask_ui_tests.rs"]
mod tests;
