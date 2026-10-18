//! Pen pressure curves (TOOL-213-11): how hard a press must be for a given effect.
//!
//! A curve is five outputs at the inputs 0, ¼, ½, ¾ and 1, joined by a monotone cubic (a curve
//! that rises everywhere stays rising, with no overshoot). It is used twice: the global curve in
//! Preferences › Tools reshapes every pen sample before it reaches a tool, and each brush dynamic
//! driven by Pen Pressure ([`crate::Dynamic::pressure_curve`]) reshapes it again for that control
//! only. Outputs outside 0–1 are clamped and a non-finite one reads as the straight line, so a
//! hostile preset or setting can never produce a value outside 0–1.
use serde::{Deserialize, Serialize};

/// Number of points on a curve.
pub const POINTS: usize = 5;

/// Five outputs (0–1) at evenly spaced pressures 0, ¼, ½, ¾, 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PressureCurve(pub [f32; POINTS]);

impl Default for PressureCurve {
    fn default() -> Self {
        Self::LINEAR
    }
}

impl PressureCurve {
    /// Output = input.
    pub const LINEAR: Self = Self([0.0, 0.25, 0.5, 0.75, 1.0]);
    /// A light touch already paints firmly.
    pub const SOFT: Self = Self([0.0, 0.45, 0.7, 0.88, 1.0]);
    /// It takes a firm press to paint fully.
    pub const FIRM: Self = Self([0.0, 0.1, 0.28, 0.55, 1.0]);

    /// The input (pressure) of point `i`.
    pub fn input(i: usize) -> f32 {
        i as f32 / (POINTS - 1) as f32
    }

    /// Point `i`'s output, made safe: clamped to 0–1, the straight line when non-finite.
    pub fn output(&self, i: usize) -> f32 {
        match self.0.get(i) {
            Some(v) if v.is_finite() => v.clamp(0.0, 1.0),
            _ => Self::input(i),
        }
    }

    /// Whether this is the straight line (presets and settings leave it out).
    pub fn is_linear(&self) -> bool {
        (0..POINTS).all(|i| (self.output(i) - Self::input(i)).abs() < 1e-4)
    }

    /// The curve's output for `pressure` (clamped to 0–1; a non-finite pressure reads as full).
    pub fn apply(&self, pressure: f32) -> f32 {
        let p = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
        if self.is_linear() {
            return p;
        }
        let y: [f32; POINTS] = std::array::from_fn(|i| self.output(i));
        let h = 1.0 / (POINTS - 1) as f32;
        // Secant slopes, then Fritsch–Carlson tangents (monotone where the points are).
        let d: [f32; POINTS - 1] = std::array::from_fn(|i| (y[i + 1] - y[i]) / h);
        let m: [f32; POINTS] = std::array::from_fn(|i| {
            if i == 0 {
                d[0]
            } else if i == POINTS - 1 {
                d[POINTS - 2]
            } else if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                let (a, b) = (d[i - 1], d[i]);
                // Harmonic mean keeps the cubic inside the segment's range.
                2.0 * a * b / (a + b)
            }
        });
        let seg = ((p / h) as usize).min(POINTS - 2);
        let t = (p - seg as f32 * h) / h;
        let (t2, t3) = (t * t, t * t * t);
        let (y0, y1, m0, m1) = (y[seg], y[seg + 1], m[seg] * h, m[seg + 1] * h);
        let v = (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * m1;
        if v.is_finite() { v.clamp(0.0, 1.0) } else { p }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_is_the_identity_and_presets_bend_it() {
        for k in 0..=20 {
            let p = k as f32 / 20.0;
            assert!((PressureCurve::LINEAR.apply(p) - p).abs() < 1e-6);
            assert!(PressureCurve::SOFT.apply(p) >= p - 1e-4, "soft paints more at {p}");
            assert!(PressureCurve::FIRM.apply(p) <= p + 1e-4, "firm paints less at {p}");
        }
        assert!(PressureCurve::default().is_linear() && !PressureCurve::SOFT.is_linear());
        // It passes through its points.
        for i in 0..POINTS {
            assert!((PressureCurve::FIRM.apply(PressureCurve::input(i)) - PressureCurve::FIRM.0[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn a_rising_curve_stays_rising() {
        let c = PressureCurve([0.0, 0.05, 0.1, 0.9, 1.0]);
        let mut last = 0.0;
        for k in 0..=200 {
            let v = c.apply(k as f32 / 200.0);
            assert!(v + 1e-6 >= last, "{k}: {v} < {last}");
            last = v;
        }
    }

    #[test]
    fn hostile_values_stay_in_range() {
        let c = PressureCurve([f32::NAN, 9.0, -3.0, f32::INFINITY, 0.2]);
        for p in [f32::NAN, -1.0, 0.0, 0.3, 0.5, 0.99, 1.0, 7.0, f32::NEG_INFINITY] {
            let v = c.apply(p);
            assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{p} -> {v}");
        }
        assert_eq!(c.output(0), 0.0, "NaN reads as the straight line");
        assert_eq!(c.output(1), 1.0);
        let json = serde_json::to_string(&PressureCurve::SOFT).unwrap();
        assert_eq!(json, "[0.0,0.45,0.7,0.88,1.0]");
        assert_eq!(serde_json::from_str::<PressureCurve>(&json).unwrap(), PressureCurve::SOFT);
    }

    /// A dynamic's curve reshapes pen pressure for that control only: Size through a firm curve
    /// is smaller at half pressure than through the straight line, and the options-bar size
    /// shortcut follows Size's curve too.
    #[test]
    fn a_dynamics_curve_reshapes_its_control() {
        use crate::brush::{BrushSettings, Control, Dynamic};
        use crate::dynamics::{StepInput, base_diameter, control_value};
        let s = StepInput { point: crate::StrokePoint { pressure: 0.5, ..Default::default() }, direction: 0.0, initial_direction: 0.0, step: 0 };
        let linear = Dynamic::controlled(Control::PenPressure);
        let firm = Dynamic { pressure_curve: PressureCurve::FIRM, ..linear };
        assert!((control_value(&linear, &s) - 0.5).abs() < 1e-6);
        assert!((control_value(&firm, &s) - 0.28).abs() < 1e-5);
        let mut b = BrushSettings { size: 100.0, pressure_size: true, ..BrushSettings::default() };
        b.shape_dynamics.enabled = false;
        assert!((base_diameter(&b, &s) - 50.0).abs() < 1e-3);
        b.shape_dynamics.size.pressure_curve = PressureCurve::FIRM;
        assert!((base_diameter(&b, &s) - 28.0).abs() < 1e-3);
        // Left out of the JSON when straight, kept when bent.
        let v = serde_json::to_value(linear).unwrap();
        assert!(v.get("pressureCurve").is_none(), "{v}");
        let v = serde_json::to_value(firm).unwrap();
        assert_eq!(serde_json::from_value::<Dynamic>(v).unwrap(), firm);
    }
}
