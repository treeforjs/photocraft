//! Adjustment parameters (data only). Evaluation lives in `photocraft-compose`.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub input: f32,
    pub output: f32,
}

/// Whether `points` is the straight 0→0, 1→1 line (or too short to be a curve).
pub fn is_identity_curve(points: &[CurvePoint]) -> bool {
    match points {
        [a, b] => a.input == 0.0 && a.output == 0.0 && b.input == 1.0 && b.output == 1.0,
        p => p.len() < 2,
    }
}

/// Points in a Curves arbitrary map (a curve drawn with the pencil): one output per 8-bit input
/// level, at inputs 0, 1/255, …, 1. Photoshop stores such a curve as 256 bytes per channel.
pub const CURVE_MAP_LEN: usize = 256;

/// Whether `points` is an arbitrary map ([`CURVE_MAP_LEN`] points, one per input level), which
/// renders by straight lines between its points instead of the spline through a point curve.
pub fn is_curve_map(points: &[CurvePoint]) -> bool {
    let last = (CURVE_MAP_LEN - 1) as f32;
    points.len() == CURVE_MAP_LEN && points.iter().enumerate().all(|(i, p)| (p.input * last - i as f32).abs() < 0.25)
}

/// The arbitrary map with these outputs (0..=1) for input levels 0..=255; missing levels repeat
/// the last value given (identity when `outputs` is empty).
pub fn curve_map(outputs: &[f32]) -> Vec<CurvePoint> {
    let last = (CURVE_MAP_LEN - 1) as f32;
    (0..CURVE_MAP_LEN)
        .map(|i| {
            let output = outputs.get(i).or(outputs.last()).copied().filter(|v| v.is_finite()).unwrap_or(i as f32 / last);
            CurvePoint { input: i as f32 / last, output: output.clamp(0.0, 1.0) }
        })
        .collect()
}

/// At most `max` (≥ 2) points that follow the arbitrary map `map` within half a level where
/// they can: the ends, then repeatedly the level farthest from the straight lines between the
/// points chosen so far (Photoshop's switch from the pencil back to points). Anything that is
/// not a map comes back unchanged.
pub fn curve_map_points(map: &[CurvePoint], max: usize) -> Vec<CurvePoint> {
    if !is_curve_map(map) {
        return map.to_vec();
    }
    let (Some(first), Some(last)) = (map.first(), map.last()) else { return map.to_vec() };
    let mut chosen = vec![0usize, map.len() - 1];
    let mut points = vec![first.clone(), last.clone()];
    while chosen.len() < max.max(2) {
        let mut worst: Option<(usize, f32)> = None;
        for pair in chosen.windows(2) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            let (Some(pa), Some(pb)) = (map.get(a), map.get(b)) else { continue };
            for i in a + 1..b {
                let Some(p) = map.get(i) else { continue };
                let t = (i - a) as f32 / (b - a) as f32;
                let error = (p.output - (pa.output + (pb.output - pa.output) * t)).abs();
                if worst.is_none_or(|(_, e)| error > e) {
                    worst = Some((i, error));
                }
            }
        }
        let Some((i, _)) = worst.filter(|(_, e)| *e > 0.5 / 255.0) else { break };
        let at = chosen.partition_point(|&c| c < i);
        chosen.insert(at, i);
        if let Some(p) = map.get(i) {
            points.insert(at, p.clone());
        }
    }
    points
}

/// Number of values in the table of a `size`³ Color Lookup (`size³ × 3`). `None` for a size
/// below 2 or one whose table length overflows: a stored document's size is untrusted.
pub fn lut3d_len(size: u32) -> Option<usize> {
    if size < 2 {
        return None;
    }
    usize::try_from(size).ok()?.checked_pow(3)?.checked_mul(3)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelsChannel {
    pub in_black: f32,
    pub in_white: f32,
    pub gamma: f32,
    pub out_black: f32,
    pub out_white: f32,
}

impl Default for LevelsChannel {
    fn default() -> Self {
        Self { in_black: 0.0, in_white: 1.0, gamma: 1.0, out_black: 0.0, out_white: 1.0 }
    }
}

/// Which document channels the per-channel records of Levels and Curves address. Compositing
/// happens in display RGB; `Cmyk` and `Lab` records are applied to the pixel converted into that
/// space (the same conversions the document's surfaces use), so ink and Lab channel edits behave
/// like Photoshop's in CMYK and Lab documents. Grayscale documents use `Rgb` with the master record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneSpace {
    /// `per_channel` = red, green, blue.
    #[default]
    Rgb,
    /// `per_channel` = cyan, magenta, yellow plus `black`; values are channel brightness
    /// (1 - ink), and the master record applies to all four inks.
    Cmyk,
    /// `per_channel` = lightness, a, b (stored 0..=1 like Lab surfaces); there is no composite
    /// record in Lab, so the master is ignored.
    Lab,
}

/// One of Hue/Saturation's six colour ranges (reds, yellows, greens, cyans, blues, magentas).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HueRange {
    /// Hue shift in degrees (-180..=180).
    pub hue: f32,
    /// Saturation change in percent (-100..=100).
    pub saturation: f32,
    /// Lightness change in percent (-100..=100).
    pub lightness: f32,
    /// Hue degrees where the range starts fading in, is fully on, starts fading out and ends
    /// (Photoshop's four range sliders). May wrap past 0/360.
    pub bounds: [f32; 4],
}

impl HueRange {
    /// The neutral range `i` (0 = reds … 5 = magentas) with Photoshop's default extent: fully on
    /// for 30° around its centre, fading over 30° each side.
    pub fn neutral(i: usize) -> Self {
        let c = 60.0 * i as f32;
        Self { hue: 0.0, saturation: 0.0, lightness: 0.0, bounds: [c - 45.0, c - 15.0, c + 15.0, c + 45.0] }
    }

    pub fn defaults() -> [HueRange; 6] {
        std::array::from_fn(Self::neutral)
    }

    /// The canonical form of four range sliders: increasing, within one turn, centre in
    /// -30..330 (so the default reds are -45, -15, 15, 45 whichever way they were written).
    pub fn canonical_bounds(b: [f32; 4]) -> [f32; 4] {
        if b.iter().any(|v| !v.is_finite()) {
            return [-45.0, -15.0, 15.0, 45.0];
        }
        let mut out = [b[0]; 4];
        for k in 1..4 {
            out[k] = (out[k - 1] + (b[k] - out[k - 1]).rem_euclid(360.0)).min(out[0] + 359.0);
        }
        let centre = (out[1] + out[2]) / 2.0;
        let shift = (centre + 30.0).rem_euclid(360.0) - 30.0 - centre;
        out.map(|v| v + shift)
    }

    pub fn is_neutral(&self) -> bool {
        self.hue == 0.0 && self.saturation == 0.0 && self.lightness == 0.0
    }

    /// How much a pixel of hue `deg` (degrees) belongs to this range, 0..=1.
    pub fn weight(&self, deg: f32) -> f32 {
        let [a, b, c, d] = self.bounds;
        // Measure from the range's start so wrap-around ranges (reds) work.
        let rel = |x: f32| if x.is_finite() { (x - a).rem_euclid(360.0) } else { 0.0 };
        let b = rel(b);
        let c = rel(c).max(b);
        let d = rel(d).max(c);
        let h = rel(deg);
        if h <= b {
            if b <= 0.0 { 1.0 } else { h / b }
        } else if h <= c {
            1.0
        } else if h <= d {
            if d <= c { 1.0 } else { (d - h) / (d - c) }
        } else {
            0.0
        }
    }
}

fn default_hue_ranges() -> [HueRange; 6] {
    HueRange::defaults()
}

/// Photoshop adjustment layers (Layer → New Adjustment Layer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Adjustment {
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
        legacy: bool,
    },
    /// R, G, B (or the channels `space` names) first, then the master (composite) record.
    /// In CMYK this also applies to `black`; in Lab the master is ignored.
    Levels {
        master: LevelsChannel,
        per_channel: [LevelsChannel; 3],
        #[serde(default)]
        space: ToneSpace,
        /// The black ink channel when `space` is [`ToneSpace::Cmyk`].
        #[serde(default)]
        black: LevelsChannel,
    },
    /// R, G, B curves (or the channels `space` names) first, then the master (composite) curve.
    /// In CMYK this also applies to `black`; in Lab the master is ignored.
    Curves {
        master: Vec<CurvePoint>,
        per_channel: [Vec<CurvePoint>; 3],
        #[serde(default)]
        space: ToneSpace,
        /// The black ink curve when `space` is [`ToneSpace::Cmyk`] (empty = identity).
        #[serde(default)]
        black: Vec<CurvePoint>,
    },
    Exposure {
        exposure: f32,
        offset: f32,
        gamma: f32,
    },
    Vibrance {
        vibrance: f32,
        saturation: f32,
    },
    /// Master hue / saturation / lightness (or the Colorize colour) plus six hue ranges.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
        colorize: bool,
        #[serde(default = "default_hue_ranges")]
        ranges: [HueRange; 6],
    },
    ColorBalance {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        preserve_luminosity: bool,
    },
    BlackWhite {
        weights: [f32; 6],
        tint: Option<[f32; 3]>,
    },
    PhotoFilter {
        color: [f32; 3],
        density: f32,
        preserve_luminosity: bool,
    },
    ChannelMixer {
        matrix: [[f32; 4]; 3],
        monochrome: bool,
    },
    /// A 3D LUT: `size`³ RGB triplets, red varying fastest (`((b·size + g)·size + r)·3`), in 0..=1.
    /// `lut` is None for a lookup Photoshop stores as an ICC profile (identity here).
    ColorLookup {
        name: String,
        lut: Option<Arc<Vec<f32>>>,
        size: u32,
        /// Tetrahedral instead of trilinear interpolation.
        #[serde(default)]
        tetrahedral: bool,
        /// Ordered dither of ±½ an 8-bit step to hide banding (Photoshop's "Dither").
        #[serde(default)]
        dither: bool,
    },
    Invert,
    Posterize {
        levels: u32,
    },
    Threshold {
        level: f32,
    },
    GradientMap {
        stops: Vec<(f32, [f32; 3])>,
        reverse: bool,
        /// Ordered dither of ±½ an 8-bit step to hide banding.
        #[serde(default)]
        dither: bool,
    },
    /// Per range (reds, yellows, greens, cyans, blues, magentas, whites, neutrals, blacks) the
    /// cyan, magenta, yellow and black change in percent (-100..=100), as in Photoshop.
    SelectiveColor {
        relative: bool,
        adjustments: [[f32; 4]; 9],
    },
    /// A PSD adjustment we can't evaluate yet; preserved raw for round-trip.
    Unsupported {
        psd_key: String,
        raw: Vec<u8>,
    },
}

impl Adjustment {
    pub fn label(&self) -> &str {
        match self {
            Adjustment::BrightnessContrast { .. } => "Brightness/Contrast",
            Adjustment::Levels { .. } => "Levels",
            Adjustment::Curves { .. } => "Curves",
            Adjustment::Exposure { .. } => "Exposure",
            Adjustment::Vibrance { .. } => "Vibrance",
            Adjustment::HueSaturation { .. } => "Hue/Saturation",
            Adjustment::ColorBalance { .. } => "Color Balance",
            Adjustment::BlackWhite { .. } => "Black & White",
            Adjustment::PhotoFilter { .. } => "Photo Filter",
            Adjustment::ChannelMixer { .. } => "Channel Mixer",
            Adjustment::ColorLookup { .. } => "Color Lookup",
            Adjustment::Invert => "Invert",
            Adjustment::Posterize { .. } => "Posterize",
            Adjustment::Threshold { .. } => "Threshold",
            Adjustment::GradientMap { .. } => "Gradient Map",
            Adjustment::SelectiveColor { .. } => "Selective Color",
            Adjustment::Unsupported { .. } => "Adjustment",
        }
    }

    pub fn default_hue_saturation() -> Self {
        Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges: HueRange::defaults() }
    }

    pub fn identity_curves() -> Self {
        let line = || vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }];
        Adjustment::Curves { master: line(), per_channel: [line(), line(), line()], space: ToneSpace::Rgb, black: Vec::new() }
    }

    pub fn identity_levels() -> Self {
        Adjustment::Levels { master: LevelsChannel::default(), per_channel: Default::default(), space: ToneSpace::Rgb, black: LevelsChannel::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_range_weights() {
        let reds = HueRange::neutral(0);
        assert_eq!(reds.weight(0.0), 1.0);
        assert_eq!(reds.weight(350.0), 1.0);
        assert!((reds.weight(30.0) - 0.5).abs() < 1e-5);
        assert!((reds.weight(330.0) - 0.5).abs() < 1e-5);
        assert_eq!(reds.weight(60.0), 0.0);
        assert_eq!(reds.weight(180.0), 0.0);
        let blues = HueRange::neutral(4);
        assert_eq!(blues.weight(240.0), 1.0);
        assert_eq!(blues.weight(120.0), 0.0);
        // Degenerate bounds never divide by zero.
        let odd = HueRange { bounds: [10.0, 10.0, 10.0, 10.0], ..HueRange::neutral(0) };
        assert!(odd.weight(10.0).is_finite() && odd.weight(200.0).is_finite());
        let nan = HueRange { bounds: [f32::NAN; 4], ..HueRange::neutral(0) };
        assert!(nan.weight(10.0).is_finite());
    }

    #[test]
    fn old_files_get_neutral_new_fields() {
        let a: Adjustment = serde_json::from_str(r#"{"HueSaturation":{"hue":1.0,"saturation":2.0,"lightness":3.0,"colorize":false}}"#).unwrap();
        assert!(matches!(a, Adjustment::HueSaturation { ranges, .. } if ranges == HueRange::defaults()));
        let c: Adjustment = serde_json::from_str(r#"{"Curves":{"master":[],"per_channel":[[],[],[]]}}"#).unwrap();
        assert!(matches!(c, Adjustment::Curves { space: ToneSpace::Rgb, .. }));
        let g: Adjustment = serde_json::from_str(r#"{"GradientMap":{"stops":[],"reverse":true}}"#).unwrap();
        assert!(matches!(g, Adjustment::GradientMap { dither: false, .. }));
    }

    #[test]
    fn lut3d_len_rejects_tiny_and_overflowing_sizes() {
        assert_eq!(lut3d_len(0), None);
        assert_eq!(lut3d_len(1), None);
        assert_eq!(lut3d_len(2), Some(24));
        assert_eq!(lut3d_len(33), Some(33 * 33 * 33 * 3));
        assert_eq!(lut3d_len(1 << 22), None, "(2²²)³ overflows a 64-bit length");
        assert_eq!(lut3d_len(u32::MAX), None);
    }

    #[test]
    fn curve_maps_have_one_point_per_level() {
        let map = curve_map(&[1.0, 0.5, f32::NAN, 2.0]);
        assert!(is_curve_map(&map));
        assert_eq!(map[0].output, 1.0);
        assert_eq!(map[1].output, 0.5);
        assert!((map[2].output - 2.0 / 255.0).abs() < 1e-6, "a non-finite output falls back to the identity");
        assert_eq!(map[3].output, 1.0, "clamped");
        assert_eq!(map[255].output, 1.0, "the last value repeats");
        assert!(is_curve_map(&curve_map(&[])));
        assert!(!is_curve_map(&map[..255]));
        let mut shifted = map.clone();
        shifted[10].input += 0.5 / 255.0;
        assert!(!is_curve_map(&shifted));
        assert!(!is_curve_map(&[CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }]));
    }

    #[test]
    fn curve_maps_reduce_to_the_points_that_shape_them() {
        let identity = curve_map_points(&curve_map(&[]), 16);
        assert_eq!(identity.len(), 2);
        assert!(is_identity_curve(&identity));
        // A tent: up to white at 100, back down to black at 200, black after.
        let tent: Vec<f32> = (0..256).map(|i: i32| (1.0 - ((i - 100) as f32 / 100.0).abs()).max(0.0)).collect();
        let pts = curve_map_points(&curve_map(&tent), 16);
        let levels: Vec<i32> = pts.iter().map(|p| (p.input * 255.0).round() as i32).collect();
        assert_eq!(levels, [0, 100, 200, 255]);
        let noisy: Vec<f32> = (0..256).map(|i| if i % 2 == 0 { 0.0 } else { 1.0 }).collect();
        let pts = curve_map_points(&curve_map(&noisy), 16);
        assert_eq!(pts.len(), 16, "capped");
        assert!(pts.windows(2).all(|w| w[0].input < w[1].input));
        let short = [CurvePoint { input: 0.0, output: 0.2 }, CurvePoint { input: 1.0, output: 0.8 }];
        assert_eq!(curve_map_points(&short, 16), short.to_vec(), "point curves pass through");
    }
}
