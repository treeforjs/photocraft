//! Color panel modes: the spectrum field (in `panels.rs`), a colour wheel, and RGB, HSB, CMYK,
//! Lab, Grayscale and Web Color slider sets, picked from the dropdown at the top of the panel.
//! Each slider's track shows the colours that channel reaches from the current one. Values that
//! don't survive the round trip through sRGB (hue on a grey, CMYK's black split, Lab a/b on black)
//! are kept while they still name the current colour, so a slider doesn't jump on release.

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;
use crate::color_picker_ui::{hsv_to_rgb, rgb_to_hsv};
use crate::theme::Tokens;
use crate::widgets;
use photocraft_color::convert::{cmyk_to_rgb, lab_to_srgb, rgb_to_cmyk, rgb_to_gray, srgb_to_lab};

/// What the Color panel shows under its chips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorPanelMode {
    /// Saturation/brightness field plus a hue strip.
    #[default]
    Spectrum,
    /// Hue around, saturation outwards, with a brightness slider.
    Wheel,
    Rgb,
    Hsb,
    Cmyk,
    Lab,
    Gray,
    /// RGB sliders that snap to the 216 web colours (steps of 51).
    Web,
}

/// The dropdown's options, in menu order.
pub const MODES: &[(ColorPanelMode, &str)] = &[
    (ColorPanelMode::Spectrum, "Spectrum"),
    (ColorPanelMode::Wheel, "Color Wheel"),
    (ColorPanelMode::Rgb, "RGB Sliders"),
    (ColorPanelMode::Hsb, "HSB Sliders"),
    (ColorPanelMode::Cmyk, "CMYK Sliders"),
    (ColorPanelMode::Lab, "Lab Sliders"),
    (ColorPanelMode::Gray, "Grayscale Slider"),
    (ColorPanelMode::Web, "Web Color Sliders"),
];

/// One slider: its label, range and unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub suffix: &'static str,
}

const fn ch(label: &'static str, min: f32, max: f32, suffix: &'static str) -> Channel {
    Channel { label, min, max, suffix }
}

const RGB: [Channel; 3] = [ch("R", 0.0, 255.0, ""), ch("G", 0.0, 255.0, ""), ch("B", 0.0, 255.0, "")];
const HSB: [Channel; 3] = [ch("H", 0.0, 360.0, "°"), ch("S", 0.0, 100.0, "%"), ch("B", 0.0, 100.0, "%")];
const CMYK: [Channel; 4] = [ch("C", 0.0, 100.0, "%"), ch("M", 0.0, 100.0, "%"), ch("Y", 0.0, 100.0, "%"), ch("K", 0.0, 100.0, "%")];
const LAB: [Channel; 3] = [ch("L", 0.0, 100.0, ""), ch("a", -128.0, 127.0, ""), ch("b", -128.0, 127.0, "")];
const GRAY: [Channel; 1] = [ch("K", 0.0, 100.0, "%")];

/// The sliders `mode` shows (none for the spectrum; the wheel's brightness slider is HSB's B).
pub fn channels(mode: ColorPanelMode) -> &'static [Channel] {
    match mode {
        ColorPanelMode::Spectrum | ColorPanelMode::Wheel => &[],
        ColorPanelMode::Rgb | ColorPanelMode::Web => &RGB,
        ColorPanelMode::Hsb => &HSB,
        ColorPanelMode::Cmyk => &CMYK,
        ColorPanelMode::Lab => &LAB,
        ColorPanelMode::Gray => &GRAY,
    }
}

/// `rgb` (sRGB-encoded 0..1) as `mode`'s slider values, in the channels' units. The wheel uses HSB.
pub fn to_values(mode: ColorPanelMode, rgb: [f32; 3]) -> [f32; 4] {
    let rgb = rgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    match mode {
        ColorPanelMode::Spectrum | ColorPanelMode::Rgb | ColorPanelMode::Web => {
            let [r, g, b] = rgb.map(|v| (v * 255.0).round());
            [r, g, b, 0.0]
        }
        ColorPanelMode::Wheel | ColorPanelMode::Hsb => {
            let [h, s, v] = rgb_to_hsv(rgb);
            [h.round().rem_euclid(360.0), (s * 100.0).round(), (v * 100.0).round(), 0.0]
        }
        ColorPanelMode::Cmyk => rgb_to_cmyk(rgb).map(|v| (v.clamp(0.0, 1.0) * 100.0).round()),
        ColorPanelMode::Lab => {
            let [l, a, b] = srgb_to_lab(rgb);
            [l.round().clamp(0.0, 100.0), a.round().clamp(-128.0, 127.0), b.round().clamp(-128.0, 127.0), 0.0]
        }
        ColorPanelMode::Gray => [((1.0 - rgb_to_gray(rgb)) * 100.0).round().clamp(0.0, 100.0), 0.0, 0.0, 0.0],
    }
}

/// `mode`'s slider values back to sRGB, clamping each to its channel's range (Web snaps to 51s).
pub fn from_values(mode: ColorPanelMode, v: [f32; 4]) -> [f32; 3] {
    let v = clamp_values(mode, v);
    let rgb = match mode {
        ColorPanelMode::Spectrum | ColorPanelMode::Rgb => [v[0], v[1], v[2]].map(|c| c / 255.0),
        ColorPanelMode::Web => [v[0], v[1], v[2]].map(|c| (c / 51.0).round() * 51.0 / 255.0),
        ColorPanelMode::Wheel | ColorPanelMode::Hsb => hsv_to_rgb(v[0], v[1] / 100.0, v[2] / 100.0),
        ColorPanelMode::Cmyk => cmyk_to_rgb(v.map(|c| c / 100.0)),
        ColorPanelMode::Lab => lab_to_srgb([v[0], v[1], v[2]]),
        ColorPanelMode::Gray => [1.0 - v[0] / 100.0; 3],
    };
    rgb.map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 })
}

/// Each value clamped to its channel (NaN to the minimum); the wheel clamps like HSB.
fn clamp_values(mode: ColorPanelMode, mut v: [f32; 4]) -> [f32; 4] {
    let chans = if mode == ColorPanelMode::Wheel { &HSB[..] } else { channels(mode) };
    for (x, c) in v.iter_mut().zip(chans) {
        *x = if x.is_finite() { x.clamp(c.min, c.max) } else { c.min };
    }
    v
}

fn bytes(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The values to show for `rgb`: the remembered ones while they still give this colour, else fresh.
pub fn shown_values(mode: ColorPanelMode, rgb: [f32; 3], remembered: Option<[f32; 4]>) -> [f32; 4] {
    match remembered {
        Some(v) if bytes(from_values(mode, v)) == bytes(rgb) => v,
        _ => to_values(mode, rgb),
    }
}

/// Track colours for channel `i`: the colour with that channel swept across its range.
pub fn track(mode: ColorPanelMode, i: usize, values: [f32; 4]) -> Vec<Color32> {
    let Some(c) = channels(mode).get(i) else { return Vec::new() };
    // Hue needs enough stops to show the spectrum; the rest are close to linear.
    let n = if mode == ColorPanelMode::Hsb && i == 0 { 13 } else { 7 };
    (0..n)
        .map(|k| {
            let mut v = values;
            if let Some(x) = v.get_mut(i) {
                *x = c.min + (c.max - c.min) * k as f32 / (n - 1) as f32;
            }
            // Keep the track readable on black and white: sweep S and H at some brightness.
            if mode == ColorPanelMode::Hsb && i != 2 {
                v[2] = v[2].max(20.0);
            }
            let [r, g, b] = bytes(from_values(mode, v));
            Color32::from_rgb(r, g, b)
        })
        .collect()
}

/// The height [`mode_row`] takes, its gap included.
pub const MODE_ROW_H: f32 = 32.0;

/// The dropdown that switches the panel's mode, right-aligned above the panel.
pub fn mode_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), MODE_ROW_H), Sense::hover());
    let row = Rect::from_min_size(row.min, vec2(row.width(), MODE_ROW_H - 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::right_to_left(egui::Align::Center)));
    widgets::dropdown(&mut child, "color-panel-mode", &mut app.ui.color_panel.mode, MODES, 150.0);
}

fn current(app: &PhotocraftApp) -> [f32; 3] {
    let c = if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground };
    [c[0], c[1], c[2]]
}

fn set(app: &mut PhotocraftApp, rgb: [f32; 3]) {
    let c = [rgb[0], rgb[1], rgb[2], 1.0];
    if app.ui.color_panel.background {
        app.session.tools.background = c;
    } else {
        app.session.tools.foreground = c;
        crate::type_tool::foreground_changed(app);
    }
}

fn memory_key(mode: ColorPanelMode, background: bool) -> egui::Id {
    // The wheel and the HSB sliders share their values, so hue survives switching between them.
    let mode = if mode == ColorPanelMode::Wheel { ColorPanelMode::Hsb } else { mode };
    egui::Id::new(("color-panel-values", mode, background))
}

/// The panel body for every mode but the spectrum, beside the colour chips. `height` is what the
/// wheel may use.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, height: f32) {
    let mode = app.ui.color_panel.mode;
    let key = memory_key(mode, app.ui.color_panel.background);
    let rgb = current(app);
    let mut values = shown_values(mode, rgb, ui.data(|d| d.get_temp(key)));
    let mut changed = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
        crate::panels::field_chips(app, ui, chips);
        ui.vertical(|ui| {
            if mode == ColorPanelMode::Wheel {
                changed |= wheel(ui, &mut values, height);
                let b = [Color32::BLACK, Color32::from(egui::ecolor::Hsva::new(values[0] / 360.0, values[1] / 100.0, 1.0, 1.0))];
                changed |= widgets::slider_row(ui, "Brightness", &mut values[2], 0.0..=100.0, "%", Some(&b)).changed();
                return;
            }
            for (i, c) in channels(mode).iter().enumerate() {
                let stops = track(mode, i, values);
                let Some(v) = values.get_mut(i) else { continue };
                changed |= widgets::slider_row(ui, c.label, v, c.min..=c.max, c.suffix, Some(&stops)).changed();
            }
        });
    });
    if changed {
        let values = clamp_values(mode, values);
        let rgb = from_values(mode, values);
        set(app, rgb);
        // Web snaps the colour; show the snapped values, not the dragged ones.
        let kept = if mode == ColorPanelMode::Web { to_values(mode, rgb) } else { values };
        ui.data_mut(|d| d.insert_temp(key, kept));
    }
}

/// The colour wheel: hue around (red at the top, clockwise), saturation from the centre out, at
/// the current brightness. Returns whether a click or drag picked a colour.
fn wheel(ui: &mut egui::Ui, values: &mut [f32; 4], height: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let d = ui.available_width().min(height).max(60.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), d), Sense::click_and_drag());
    let rect = Rect::from_center_size(rect.center(), vec2(d, d));
    let (c, r) = (rect.center(), d / 2.0 - 2.0);
    let v = (values[2] / 100.0).clamp(0.0, 1.0);
    let mut mesh = egui::Mesh::default();
    let (rings, segs) = (6usize, 48usize);
    for j in 0..=rings {
        let s = j as f32 / rings as f32;
        for k in 0..=segs {
            let h = k as f32 / segs as f32;
            let a = h * std::f32::consts::TAU;
            let col = Color32::from(egui::ecolor::Hsva::new(h, s, v, 1.0));
            mesh.colored_vertex(pos2(c.x + a.sin() * r * s, c.y - a.cos() * r * s), col);
        }
    }
    for j in 0..rings {
        for k in 0..segs {
            let a = (j * (segs + 1) + k) as u32;
            let b = a + 1;
            let cc = a + (segs + 1) as u32;
            mesh.add_triangle(a, b, cc + 1);
            mesh.add_triangle(a, cc + 1, cc);
        }
    }
    ui.painter().add(mesh);
    ui.painter().circle_stroke(c, r, Stroke::new(1.0, t.separator));
    let mut picked = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (dx, dy) = (p.x - c.x, p.y - c.y);
        let dist = (dx * dx + dy * dy).sqrt();
        if r > 0.0 {
            values[1] = (dist / r * 100.0).clamp(0.0, 100.0);
            if dist > 0.5 {
                values[0] = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
            }
            picked = true;
        }
    }
    let a = values[0].to_radians();
    let s = (values[1] / 100.0).clamp(0.0, 1.0);
    let knob = pos2(c.x + a.sin() * r * s, c.y - a.cos() * r * s);
    ui.painter().circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
    ui.painter().circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tl!("Color Wheel")));
    ui.add_space(6.0);
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_round_trips_a_colour() {
        let rgb = [0.8, 0.4, 0.2];
        for (mode, _) in MODES {
            // Web snaps and Grayscale drops the hue; their own tests cover them.
            if matches!(mode, ColorPanelMode::Web | ColorPanelMode::Gray) {
                continue;
            }
            let back = from_values(*mode, to_values(*mode, rgb));
            // Rounded slider values (Lab and CMYK whole numbers) stay within a few levels.
            for (a, b) in back.iter().zip(rgb) {
                assert!((a - b).abs() < 0.03, "{mode:?}: {back:?} vs {rgb:?}");
            }
        }
    }

    #[test]
    fn web_sliders_snap_to_web_colours() {
        assert_eq!(bytes(from_values(ColorPanelMode::Web, [200.0, 30.0, 128.0, 0.0])), [204, 51, 153]);
    }

    #[test]
    fn hostile_values_clamp() {
        for (mode, _) in MODES {
            let rgb = from_values(*mode, [f32::NAN, 1e30, -1e30, f32::INFINITY]);
            assert!(rgb.iter().all(|c| (0.0..=1.0).contains(c)), "{mode:?}: {rgb:?}");
        }
        assert!(track(ColorPanelMode::Rgb, 9, [0.0; 4]).is_empty());
    }

    #[test]
    fn hue_survives_a_grey_while_it_still_names_the_colour() {
        let grey = [0.5, 0.5, 0.5];
        let kept = [200.0, 0.0, 50.0, 0.0];
        assert_eq!(shown_values(ColorPanelMode::Hsb, grey, Some(kept)), kept);
        // A different colour discards the remembered values.
        assert_eq!(shown_values(ColorPanelMode::Hsb, [1.0, 0.0, 0.0], Some(kept)), [0.0, 100.0, 100.0, 0.0]);
    }

    #[test]
    fn gray_is_ink_coverage() {
        assert_eq!(to_values(ColorPanelMode::Gray, [1.0; 3])[0], 0.0);
        assert_eq!(to_values(ColorPanelMode::Gray, [0.0; 3])[0], 100.0);
        assert_eq!(bytes(from_values(ColorPanelMode::Gray, [25.0, 0.0, 0.0, 0.0])), [191, 191, 191]);
    }

    #[test]
    fn tracks_sweep_their_channel() {
        let red = track(ColorPanelMode::Rgb, 0, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(red.first(), Some(&Color32::BLACK));
        assert_eq!(red.last(), Some(&Color32::from_rgb(255, 0, 0)));
        assert_eq!(track(ColorPanelMode::Hsb, 0, [0.0, 100.0, 100.0, 0.0]).len(), 13);
    }
}
