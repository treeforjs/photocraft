//! The Crop tool's W x H x Resolution and saved crop presets (options bar).
//!
//! With the ratio menu on "W x H x Resolution" (`crop_ratio` = [`SIZE`]) the bar shows a width,
//! a height and a resolution. Width and height take a unit after the number (`1200`, `1200 px`,
//! `4 in`, `10cm`, `90 mm`, `72pt`, `6 pica`; a bare number is pixels) and may be left empty. Both
//! set: the frame keeps their ratio. On ↵ the crop is resampled to them and the document gets the
//! resolution (`image.crop`'s `resizeWidth`/`resizeHeight`/`resolution`, one undo step); one side
//! alone resamples keeping the cropped aspect, and the resolution alone only sets the resolution.
//! Physical lengths turn into pixels at the resolution typed, else the document's.
//!
//! Crop presets are tool presets of the Crop tool (as in Photoshop): "New Crop Preset…" saves the
//! ratio or the W x H x Resolution, and picking a preset in the menu restores it. The Tool Presets
//! panel renames and deletes them.
use crate::state::ToolOptions;
use photocraft_engine::prefs::Unit;
use serde_json::{Value, json};

/// The ratio-menu key of W x H x Resolution.
pub const SIZE: &str = "size";
/// Ratio-menu keys of saved presets start with this.
pub const PRESET: &str = "preset:";
/// The ratio-menu key of "New Crop Preset…".
pub const NEW_PRESET: &str = "newPreset";

/// Pixels per inch in a point (PostScript points).
const PT_PER_IN: f64 = 72.0;

/// A typed length as `(value, unit)`; a bare number is pixels.
pub fn parse_len(text: &str) -> Option<(f64, Unit)> {
    let t = text.trim().to_ascii_lowercase();
    let split = t.find(|c: char| c.is_ascii_alphabetic() || c == '"').unwrap_or(t.len());
    let (num, unit) = (t.get(..split)?.trim(), t.get(split..)?.trim());
    let unit = match unit {
        "" | "px" | "pixel" | "pixels" => Unit::Pixels,
        "in" | "inch" | "inches" | "\"" => Unit::Inches,
        "cm" => Unit::Centimeters,
        "mm" => Unit::Millimeters,
        "pt" | "pts" | "points" => Unit::Points,
        "pc" | "pica" | "picas" => Unit::Picas,
        _ => return None,
    };
    let v = crate::widgets::parse_num(num).filter(|v| v.is_finite() && *v > 0.0)?;
    Some((v, unit))
}

/// The typed resolution in pixels per inch (`crop_res_unit` "ppcm" is per centimetre).
pub fn resolution_ppi(o: &ToolOptions) -> Option<f64> {
    let v = crate::widgets::parse_num(&o.crop_resolution).filter(|v| v.is_finite() && *v > 0.0)?;
    Some(if o.crop_res_unit == "ppcm" { v * 2.54 } else { v })
}

/// The W x H x Resolution target: width and height in pixels (either may be missing) and the
/// resolution (ppi), with `doc_dpi` for physical lengths when no resolution is typed.
pub fn target(o: &ToolOptions, doc_dpi: f64) -> (Option<f64>, Option<f64>, Option<f64>) {
    let res = resolution_ppi(o);
    let dpi = res.unwrap_or(doc_dpi).max(1e-6);
    let px = |s: &str| parse_len(s).map(|(v, u)| u.to_px(v, dpi, 0.0, PT_PER_IN)).filter(|v| v.is_finite() && *v >= 0.5);
    (px(&o.crop_width), px(&o.crop_height), res)
}

/// The frame ratio W x H x Resolution holds: both sides set.
pub fn ratio(o: &ToolOptions, doc_dpi: f64) -> Option<(f64, f64)> {
    match target(o, doc_dpi) {
        (Some(w), Some(h), _) => Some((w, h)),
        _ => None,
    }
}

/// Adds the resample to `image.crop`'s params when W x H x Resolution is on.
pub fn commit_params(o: &ToolOptions, doc_dpi: f64, p: &mut Value) {
    if o.crop_ratio != SIZE {
        return;
    }
    let (w, h, res) = target(o, doc_dpi);
    if let Some(w) = w {
        p["resizeWidth"] = json!(w.round().max(1.0));
    }
    if let Some(h) = h {
        p["resizeHeight"] = json!(h.round().max(1.0));
    }
    if let Some(r) = res {
        p["resolution"] = json!(r);
    }
}

/// Swaps the width and height (the bar's ⇄ and the X key).
pub fn swap(o: &mut ToolOptions) {
    std::mem::swap(&mut o.crop_width, &mut o.crop_height);
}

/// The Crop tool's saved presets, by name.
pub fn presets(app: &crate::PhotocraftApp) -> Vec<String> {
    app.session.presets.tool_presets.iter().filter(|p| p.tool.eq_ignore_ascii_case("crop")).map(|p| p.name.clone()).collect()
}

/// A preset's default name: its ratio or W x H x Resolution.
pub fn preset_name(o: &ToolOptions) -> String {
    if o.crop_ratio == SIZE {
        let res = if o.crop_resolution.trim().is_empty() { String::new() } else { format!(" {} {}", o.crop_resolution.trim(), o.crop_res_unit) };
        let sides: Vec<&str> = [o.crop_width.trim(), o.crop_height.trim()].into_iter().filter(|s| !s.is_empty()).collect();
        return format!("{}{res}", sides.join(" x ")).trim().to_string();
    }
    crate::chrome_ui::CROP_RATIOS.iter().find(|(k, _)| *k == o.crop_ratio).map_or_else(|| o.crop_ratio.replace(':', " : "), |(_, l)| tl!(l).to_string())
}

/// Saves the current ratio or W x H x Resolution as a crop preset (a Crop tool preset).
pub fn save_preset(app: &mut crate::PhotocraftApp) {
    let o = &app.ui.tool_options;
    let opts = json!({"toolOptions": {
        "crop_ratio": o.crop_ratio,
        "crop_width": o.crop_width,
        "crop_height": o.crop_height,
        "crop_resolution": o.crop_resolution,
        "crop_res_unit": o.crop_res_unit,
    }});
    let name = preset_name(o);
    let _ = app.run("tool.presets.new", json!({"name": if name.is_empty() { "Crop".into() } else { name }, "tool": "crop", "options": opts}));
}

/// The options bar's ratio menu after a pick: a preset or "New Crop Preset…" is applied and the
/// menu shows the result (the preset's ratio, or what was there before).
pub fn after_pick(app: &mut crate::PhotocraftApp, before: &str) {
    let key = app.ui.tool_options.crop_ratio.clone();
    if key == NEW_PRESET {
        app.ui.tool_options.crop_ratio = before.to_string();
        save_preset(app);
    } else if let Some(name) = key.strip_prefix(PRESET) {
        app.ui.tool_options.crop_ratio = before.to_string();
        crate::preset_panels::select_tool_preset(app, name);
    }
}

/// The width, ⇄, height, resolution and resolution-unit fields.
pub fn fields(o: &mut ToolOptions, ui: &mut egui::Ui) {
    let field = |ui: &mut egui::Ui, id: &str, text: &mut String, hint: &str, width: f32| {
        ui.add(egui::TextEdit::singleline(text).id_salt(id).hint_text(hint).desired_width(width))
    };
    field(ui, "crop-width", &mut o.crop_width, tl!("W"), 64.0);
    if crate::icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked() {
        swap(o);
    }
    field(ui, "crop-height", &mut o.crop_height, tl!("H"), 64.0);
    field(ui, "crop-resolution", &mut o.crop_resolution, tl!("Resolution"), 64.0);
    crate::widgets::dropdown(ui, "crop-res-unit", &mut o.crop_res_unit, &[("ppi".to_string(), "px/in"), ("ppcm".to_string(), "px/cm")], 64.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(w: &str, h: &str, res: &str, unit: &str) -> ToolOptions {
        ToolOptions {
            crop_ratio: SIZE.into(),
            crop_width: w.into(),
            crop_height: h.into(),
            crop_resolution: res.into(),
            crop_res_unit: unit.into(),
            ..ToolOptions::default()
        }
    }

    #[test]
    fn lengths_take_units() {
        assert_eq!(parse_len("1200"), Some((1200.0, Unit::Pixels)));
        assert_eq!(parse_len(" 4 in "), Some((4.0, Unit::Inches)));
        assert_eq!(parse_len("10CM"), Some((10.0, Unit::Centimeters)));
        assert_eq!(parse_len("2*3 mm"), Some((6.0, Unit::Millimeters)));
        assert_eq!(parse_len("6\""), Some((6.0, Unit::Inches)));
        for bad in ["", "in", "4 furlongs", "-3 px", "0", "1/0"] {
            assert_eq!(parse_len(bad), None, "{bad}");
        }
    }

    #[test]
    fn width_height_and_resolution_become_pixels() {
        // 4 x 6 in at 300 ppi.
        let o = opts("4 in", "6 in", "300", "ppi");
        assert_eq!(target(&o, 72.0), (Some(1200.0), Some(1800.0), Some(300.0)));
        assert_eq!(ratio(&o, 72.0), Some((1200.0, 1800.0)));
        // Without a resolution, physical lengths use the document's.
        let o = opts("2 in", "", "", "ppi");
        assert_eq!(target(&o, 150.0), (Some(300.0), None, None));
        assert_eq!(ratio(&o, 150.0), None, "one side holds no ratio");
        // Pixels per centimetre.
        let o = opts("10 cm", "500", "100", "ppcm");
        let (w, h, r) = target(&o, 72.0);
        assert!((w.unwrap() - 1000.0).abs() < 1e-6 && h == Some(500.0) && (r.unwrap() - 254.0).abs() < 1e-9);
        let mut p = json!({});
        commit_params(&o, 72.0, &mut p);
        assert_eq!((p["resizeWidth"].as_f64(), p["resizeHeight"].as_f64()), (Some(1000.0), Some(500.0)));
        // Only W x H x Resolution resamples.
        let mut p = json!({});
        commit_params(&ToolOptions { crop_ratio: "1:1".into(), ..o }, 72.0, &mut p);
        assert_eq!(p, json!({}));
    }

    #[test]
    fn presets_are_named_for_what_they_hold() {
        assert_eq!(preset_name(&opts("4 in", "6 in", "300", "ppi")), "4 in x 6 in 300 ppi");
        assert_eq!(preset_name(&ToolOptions { crop_ratio: "3:2".into(), ..ToolOptions::default() }), "3 : 2");
        let mut o = opts("1", "2", "", "ppi");
        swap(&mut o);
        assert_eq!((o.crop_width.as_str(), o.crop_height.as_str()), ("2", "1"));
    }

    /// The options bar: picking W x H x Resolution shows its fields, typed values land in the
    /// tool options, and New Crop Preset… saves a preset the menu then lists.
    #[test]
    fn options_bar_shows_the_fields_and_saves_a_preset() {
        use egui_kittest::kittest::Queryable;
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut app = crate::PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = crate::state::Tool::Crop;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 500.0)).build_ui_state(
            |ui, app: &mut crate::PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
        h.run_steps(4);
        assert!(h.query_by_label("px/in").is_none(), "no W x H x Resolution fields yet");
        h.get_by_value("Ratio").click();
        h.run_steps(2);
        h.get_by_label("W x H x Resolution").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.tool_options.crop_ratio, SIZE);
        h.get_by_value("px/in");
        h.get_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().click();
        h.run();
        h.get_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().type_text("4 in");
        h.run_steps(2);
        assert_eq!(h.state().ui.tool_options.crop_width, "4 in");
        h.get_by_value("W x H x Resolution").click();
        h.run_steps(2);
        h.get_by_label("New Crop Preset…").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.tool_options.crop_ratio, SIZE, "the menu keeps W x H x Resolution");
        assert_eq!(presets(h.state()), vec!["4 in".to_string()]);
    }
}
