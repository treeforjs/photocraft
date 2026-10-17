//! The Curves pencil: a curve drawn freehand is an arbitrary map, one output per input level
//! ([`photocraft_doc::adjust::CURVE_MAP_LEN`] points). These helpers edit the editor's
//! `[input, output]` pairs (0–255); the Curves editor owns the gesture and rendering.
use photocraft_doc::adjust::{CURVE_MAP_LEN, CurvePoint, curve_map, curve_map_points, is_curve_map};

fn to_doc(pts: &[[f32; 2]]) -> Vec<CurvePoint> {
    pts.iter().map(|q| CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect()
}

fn from_doc(pts: &[CurvePoint]) -> Vec<[f32; 2]> {
    pts.iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect()
}

/// Whether `pts` is a pencil-drawn map.
pub fn is_map(pts: &[[f32; 2]]) -> bool {
    is_curve_map(&to_doc(pts))
}

/// `pts` as a map: a map stays as it is, a point curve is sampled level by level (switching to
/// the pencil keeps the curve's shape).
pub fn to_map(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let cp = to_doc(pts);
    if is_curve_map(&cp) {
        return pts.to_vec();
    }
    let lut = photocraft_compose::adjust::curve_lut(&cp);
    let last = lut.len().saturating_sub(1) as f32;
    let outputs: Vec<f32> = (0..CURVE_MAP_LEN)
        .map(|k| {
            let at = k as f32 / (CURVE_MAP_LEN - 1) as f32 * last;
            let i = at.floor() as usize;
            let y0 = lut.get(i).copied().unwrap_or(k as f32 / 255.0);
            let y1 = lut.get(i + 1).copied().unwrap_or(y0);
            y0 + (y1 - y0) * (at - i as f32)
        })
        .collect();
    from_doc(&curve_map(&outputs))
}

/// `pts` as at most `max` points that follow it (switching back from the pencil).
pub fn to_points(pts: &[[f32; 2]], max: usize) -> Vec<[f32; 2]> {
    from_doc(&curve_map_points(&to_doc(pts), max))
}

/// Draws on `map` from `from` (the previous pencil position, if the stroke continues) to `to`,
/// both `[input, output]` in 0–255: every level between gets the output on the straight line.
/// Returns whether any level changed.
pub fn draw(map: &mut [[f32; 2]], from: Option<[f32; 2]>, to: [f32; 2]) -> bool {
    if !to.iter().all(|v| v.is_finite()) || map.len() != CURVE_MAP_LEN {
        return false;
    }
    let from = from.filter(|f| f.iter().all(|v| v.is_finite())).unwrap_or(to);
    let level = |x: f32| x.round().clamp(0.0, 255.0) as usize;
    let (a, b) = (level(from[0]), level(to[0]));
    let (lo, hi) = (a.min(b), a.max(b));
    let mut changed = false;
    for i in lo..=hi {
        let t = if a == b { 1.0 } else { (i as f32 - a as f32) / (b as f32 - a as f32) };
        let y = (from[1] + (to[1] - from[1]) * t).round().clamp(0.0, 255.0);
        if let Some(q) = map.get_mut(i)
            && q[1] != y
        {
            q[1] = y;
            changed = true;
        }
    }
    changed
}

/// One pass of Photoshop's Smooth: every level becomes the average of the five around it (fewer
/// at the ends). Returns whether any level changed.
pub fn smooth(map: &mut [[f32; 2]]) -> bool {
    if map.len() != CURVE_MAP_LEN {
        return false;
    }
    let before: Vec<f32> = map.iter().map(|q| q[1]).collect();
    let mut changed = false;
    for (i, q) in map.iter_mut().enumerate() {
        let window = before.get(i.saturating_sub(2)..(i + 3).min(before.len())).unwrap_or(&[]);
        if window.is_empty() {
            continue;
        }
        let y = (window.iter().sum::<f32>() / window.len() as f32).round();
        changed |= q[1] != y;
        q[1] = y;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Vec<[f32; 2]> {
        vec![[0.0, 0.0], [255.0, 255.0]]
    }

    #[test]
    fn a_point_curve_becomes_a_map_and_back() {
        let map = to_map(&line());
        assert!(is_map(&map));
        assert!(map.iter().all(|q| q[0] == q[1]));
        assert_eq!(to_map(&map), map, "a map stays");
        assert_eq!(to_points(&map, 16), line());
        assert!(!is_map(&line()));
    }

    #[test]
    fn drawing_fills_every_level_along_the_stroke() {
        let mut map = to_map(&line());
        assert!(draw(&mut map, None, [10.0, 200.0]));
        assert_eq!(map[10][1], 200.0);
        assert_eq!(map[11][1], 11.0);
        // A fast stroke backwards still fills every level in between.
        assert!(draw(&mut map, Some([100.0, 0.0]), [90.0, 100.0]));
        assert_eq!(map[100][1], 0.0);
        assert_eq!(map[95][1], 50.0);
        assert_eq!(map[90][1], 100.0);
        assert!(!draw(&mut map, None, [90.0, 100.0]), "nothing new");
        assert!(!draw(&mut map, None, [f32::NAN, 1.0]));
        assert!(draw(&mut map, None, [900.0, -5.0]), "clamped to the last level");
        assert_eq!(map[255][1], 0.0);
        assert!(!draw(&mut line(), None, [10.0, 10.0]), "only maps are drawn on");
    }

    #[test]
    fn smoothing_flattens_a_spike() {
        let mut map = to_map(&line());
        draw(&mut map, None, [128.0, 255.0]);
        assert!(smooth(&mut map));
        assert!(map[128][1] < 200.0 && map[126][1] > 126.0);
        assert_eq!(map[0][1], 1.0, "the ends average what they have");
        let mut flat = to_map(&[[0.0, 64.0], [255.0, 64.0]]);
        assert!(!smooth(&mut flat));
    }
}
