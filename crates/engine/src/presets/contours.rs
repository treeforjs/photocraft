//! Layer Style contours: the transfer curves Drop Shadow, Inner Shadow, the glows, Satin and
//! Bevel & Emboss apply to their coverage. The built-in set is
//! [`crate::layer_style::CONTOURS`] (our own curves); contours made in the Contour Editor are
//! saved here as presets, in groups like the other preset panels, and persist with the
//! preferences.
//!
//! Effects take a contour as a name (a built-in) or as `{"name", "points": [[in, out], …]}`
//! (see [`contour_value`]), so a saved preset reaches an effect as its points: removing the
//! preset later never changes a document.

use photocraft_doc::adjust::CurvePoint;
use photocraft_doc::effects::Contour;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Group, Named, always, bad, edit_groups, group_index, req_str, str_param, unique_name};
use crate::commands::CommandSpec;
use crate::{Result, Session};

/// Most points a contour keeps (the editor's limit, and a guard against hostile params).
pub const MAX_POINTS: usize = 32;
/// Most contour presets kept (hand-edited preferences can't make the list unbounded).
const MAX_PRESETS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContourPreset {
    pub name: String,
    /// `(input, output)` in `0..=1`, sorted by input, 2..=[`MAX_POINTS`] of them.
    pub points: Vec<(f32, f32)>,
}

impl Named for ContourPreset {
    fn name(&self) -> &str {
        &self.name
    }
    fn set_name(&mut self, n: String) {
        self.name = n;
    }
}

impl ContourPreset {
    pub fn contour(&self) -> Contour {
        to_contour(&self.name, &self.points)
    }
}

/// The user's contours start empty: the built-ins are always offered by the picker.
pub fn builtin() -> Vec<Group<ContourPreset>> {
    vec![Group::new("Custom Contours", Vec::new())]
}

/// Clean contour points: finite, clamped to `0..=1`, sorted by input, one point per input
/// (the last wins). `None` when fewer than two distinct inputs remain or there are too many.
pub fn sanitize(points: &[(f32, f32)]) -> Option<Vec<(f32, f32)>> {
    if points.len() > MAX_POINTS * 4 {
        return None;
    }
    let mut v: Vec<(f32, f32)> = points.iter().filter(|(i, o)| i.is_finite() && o.is_finite()).map(|(i, o)| (i.clamp(0.0, 1.0), o.clamp(0.0, 1.0))).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(v.len());
    for p in v {
        match out.last_mut() {
            Some(last) if (last.0 - p.0).abs() < 1e-4 => *last = p,
            _ => out.push(p),
        }
    }
    (out.len() >= 2 && out.len() <= MAX_POINTS).then_some(out)
}

fn to_contour(name: &str, points: &[(f32, f32)]) -> Contour {
    let linear = points.len() == 2 && points.first() == Some(&(0.0, 0.0)) && points.last() == Some(&(1.0, 1.0));
    if linear && (name.is_empty() || name == "Linear") {
        return Contour::Linear;
    }
    Contour::Custom { name: name.to_string(), points: points.iter().map(|&(input, output)| CurvePoint { input, output }).collect() }
}

/// `[[in, out], …]` from params.
fn points_param(v: &Value) -> Option<Vec<(f32, f32)>> {
    let a = v.as_array()?;
    if a.len() > MAX_POINTS * 4 {
        return None;
    }
    a.iter().map(|p| Some((p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))).collect()
}

/// A contour from an effect param: a built-in's name, or `{"name"?, "points": [[in, out], …]}`.
/// `None` for an unknown name (imported contours are then carried rather than clobbered) or
/// unusable points.
pub fn contour_value(v: &Value) -> Option<Contour> {
    match v {
        Value::String(name) => crate::layer_style::builtin_contour(name),
        Value::Object(o) => {
            let points = sanitize(&points_param(o.get("points")?)?)?;
            let name = o.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Custom");
            Some(to_contour(name, &points))
        }
        _ => None,
    }
}

/// The param form of `c`: a built-in's name when it is one, else its name and points.
pub fn contour_json(c: &Contour) -> Value {
    match c {
        Contour::Linear => json!("Linear"),
        Contour::Custom { name, points } => {
            if crate::layer_style::builtin_contour(name).as_ref() == Some(c) {
                return json!(name);
            }
            json!({"name": name, "points": points.iter().map(|p| json!([p.input, p.output])).collect::<Vec<_>>()})
        }
    }
}

fn preset_json(p: &ContourPreset) -> Value {
    json!({"name": p.name, "points": p.points.iter().map(|(i, o)| json!([i, o])).collect::<Vec<_>>()})
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let builtins: Vec<Value> =
        crate::layer_style::CONTOURS.iter().map(|(n, pts)| json!({"name": n, "points": pts.iter().map(|(i, o)| json!([i, o])).collect::<Vec<_>>()})).collect();
    let groups: Vec<Value> =
        s.presets.contours.iter().map(|g| json!({"name": g.name, "presets": g.items.iter().map(preset_json).collect::<Vec<_>>()})).collect();
    Ok(json!({"builtin": builtins, "groups": groups}))
}

fn new(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "contour.presets.new";
    let raw = p.get("points").and_then(points_param).ok_or_else(|| bad(CMD, "`points` must be [[input, output], …] in 0..1"))?;
    let points = sanitize(&raw).ok_or_else(|| bad(CMD, format!("a contour needs 2 to {MAX_POINTS} points with different inputs")))?;
    if s.presets.contours.iter().map(|g| g.items.len()).sum::<usize>() >= MAX_PRESETS {
        return Err(bad(CMD, format!("at most {MAX_PRESETS} contour presets")));
    }
    let base = req_str(p, "name", CMD)?;
    let name = unique_name(&s.presets.contours, base);
    let gi = group_index(&mut s.presets.contours, str_param(p, "group"));
    let group = s.presets.contours.get_mut(gi).ok_or_else(|| bad(CMD, "no group"))?;
    group.items.push(ContourPreset { name: name.clone(), points });
    let group = group.name.clone();
    s.presets_changed();
    Ok(json!({"name": name, "group": group}))
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let action = req_str(p, "action", "contour.presets.edit")?.to_string();
    let r = edit_groups(&mut s.presets.contours, &action, p, "contour.presets.edit")?;
    s.presets_changed();
    Ok(r)
}

/// Caps a persisted list (hand-edited preferences).
pub(crate) fn cap(groups: &mut Vec<Group<ContourPreset>>) {
    let mut left = MAX_PRESETS;
    groups.truncate(MAX_PRESETS);
    for g in groups.iter_mut() {
        g.items.retain(|c| sanitize(&c.points).as_deref() == Some(c.points.as_slice()));
        g.items.truncate(left);
        left -= g.items.len();
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "contour.presets.list",
            label: "Contour Presets",
            menu: &[],
            shortcut: None,
            params: "{} → {builtin:[{name,points}],groups:[{name,presets:[{name,points:[[in,out],…]}]}]}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "contour.presets.new",
            label: "New Contour Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str,"points":[[input 0..1,output 0..1],…] (2..32),"group":name?} → {name,group}. Saves a contour from the Contour Editor; effects take it as {"contour":{"name","points"}}."##,
            enabled: always,
            run: new,
            journal: true,
        },
        CommandSpec {
            id: "contour.presets.edit",
            label: "Edit Contour Presets",
            menu: &[],
            shortcut: None,
            params: super::GROUP_EDIT_PARAMS,
            enabled: always,
            run: edit,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_points_are_cleaned_or_refused() {
        assert_eq!(sanitize(&[(1.5, -2.0), (f32::NAN, 0.5), (0.0, 0.25), (0.5, 0.5), (0.50001, 0.75)]), Some(vec![(0.0, 0.25), (0.50001, 0.75), (1.0, 0.0)]));
        assert_eq!(sanitize(&[(0.3, 0.3), (0.3, 0.9)]), None, "one input is not a curve");
        assert_eq!(sanitize(&[(f32::INFINITY, 0.0)]), None);
        let many: Vec<(f32, f32)> = (0..40).map(|i| (i as f32 / 39.0, 0.5)).collect();
        assert_eq!(sanitize(&many), None, "too many points");
        assert!(contour_value(&json!({"points": [[0, 0], ["x", 1]]})).is_none());
        assert!(contour_value(&json!({"points": "nope"})).is_none());
        assert!(contour_value(&json!(42)).is_none());
    }

    #[test]
    fn param_form_round_trips() {
        let c = contour_value(&json!({"name": "Steps", "points": [[0, 0], [0.5, 1], [1, 0.2]]})).unwrap();
        let Contour::Custom { name, points } = &c else { panic!("{c:?}") };
        assert_eq!((name.as_str(), points.len()), ("Steps", 3));
        assert_eq!(contour_value(&contour_json(&c)), Some(c));
        assert_eq!(contour_json(&crate::layer_style::builtin_contour("Cone").unwrap()), json!("Cone"));
        assert_eq!(
            contour_value(&json!({"points": [[0, 0], [1, 1]]})),
            Some(Contour::Custom { name: "Custom".into(), points: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }] })
        );
        assert_eq!(contour_value(&json!("Linear")), Some(Contour::Linear));
    }

    #[test]
    fn presets_save_list_rename_and_persist() {
        let mut s = Session::new();
        let r = s.execute("contour.presets.new", json!({"name": "Ridge", "points": [[0, 0], [0.5, 1], [1, 0]]})).unwrap();
        assert_eq!(r["name"], "Ridge");
        assert_eq!(s.execute("contour.presets.new", json!({"name": "Ridge", "points": [[0, 1], [1, 0]]})).unwrap()["name"], "Ridge 2");
        assert!(s.execute("contour.presets.new", json!({"name": "Bad", "points": [[0.5, 0.5]]})).is_err());
        let l = s.execute("contour.presets.list", json!({})).unwrap();
        assert_eq!(l["builtin"].as_array().unwrap().len(), crate::layer_style::CONTOURS.len());
        assert_eq!(l["groups"][0]["presets"][0]["points"], json!([[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]));
        s.execute("contour.presets.edit", json!({"action": "rename", "preset": "Ridge 2", "name": "Fall"})).unwrap();
        let saved = s.presets.to_json(&s);
        let mut t = Session::new();
        t.load_presets_json(saved);
        let names: Vec<String> = t.presets.contours.iter().flat_map(|g| g.items.iter().map(|c| c.name.clone())).collect();
        assert_eq!(names, ["Ridge", "Fall"]);
    }
}
