//! Properties › Masks: Density and Feather for the layer's pixel mask and vector mask. A slider
//! follows the pointer while dragged and commits one history step on release
//! (`layer.layerMask.properties`, `layer.vectorMask.edit`).

use photocraft_doc::Layer;
use serde_json::{Value, json};

use crate::props_layout::section;
use crate::{PhotocraftApp, widgets};

/// A slider row that reports its value once the edit is committed (drag released, typed).
fn row(ui: &mut egui::Ui, id: &str, label: &str, current: f32, range: std::ops::RangeInclusive<f32>, suffix: &str) -> Option<f32> {
    let key = egui::Id::new(("mask-props", id));
    let mut v = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(current);
    let r = widgets::slider_row(ui, label, &mut v, range, suffix, None);
    if r.dragged() {
        ui.data_mut(|d| d.insert_temp(key, v));
        return None;
    }
    ui.data_mut(|d| d.remove::<f32>(key));
    (r.drag_stopped() || r.changed()).then_some(v).filter(|v| (*v - current).abs() > 1e-3)
}

/// The command and params that set one mask property, for `kind` "pixel" or "vector".
pub fn params(layer: u64, kind: &str, key: &str, v: f32) -> (&'static str, Value) {
    let cmd = if kind == "vector" { "layer.vectorMask.edit" } else { "layer.layerMask.properties" };
    (cmd, json!({"layer": layer, key: v}))
}

/// The Masks section, when the layer has a pixel or a vector mask.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let pixel = layer.mask.as_ref().map(|m| (m.density, m.feather));
    let vector = layer.vector_mask.as_ref().map(|m| (m.density, m.feather));
    if (pixel.is_none() && vector.is_none()) || !section(ui, "masks", tl!("Masks")) {
        return;
    }
    let both = pixel.is_some() && vector.is_some();
    let mut run = Vec::new();
    for (kind, title, vals) in [("pixel", tl!("Pixel mask"), pixel), ("vector", tl!("Vector mask"), vector)] {
        let Some((density, feather)) = vals else { continue };
        if both {
            let t = crate::theme::Tokens::get(ui.ctx());
            ui.label(egui::RichText::new(title).color(t.text_dim).size(11.0));
        }
        if let Some(v) = row(ui, &format!("{kind}-density"), tl!("Density"), density * 100.0, 0.0..=100.0, "%") {
            run.push(params(layer.id.0, kind, "density", v));
        }
        if let Some(v) = row(ui, &format!("{kind}-feather"), tl!("Feather"), feather, 0.0..=1000.0, "px") {
            run.push(params(layer.id.0, kind, "feather", v));
        }
    }
    for (id, p) in run {
        if let Err(e) = app.run(id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    #[test]
    fn mask_params_set_density_and_feather_in_one_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        let (cmd, p) = params(id, "pixel", "density", 25.0);
        s.execute(cmd, p).unwrap();
        let (cmd, p) = params(id, "pixel", "feather", 12.0);
        s.execute(cmd, p).unwrap();
        let mask = |s: &Session| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.as_ref().map(|m| (m.density, m.feather));
        assert_eq!(mask(&s), Some((0.25, 12.0)));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(mask(&s), Some((0.25, 0.0)));
        assert_eq!(params(id, "vector", "feather", 3.0).0, "layer.vectorMask.edit");
    }
}
