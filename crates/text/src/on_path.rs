//! Type on a path: the text's baseline follows a path. Each glyph stays rigid and turns with
//! the path at its centre (glyphs past either end of the path are hidden, like overset text).
//!
//! Text space maps onto the path by arc length: `x` is the distance along the path from
//! [`TextPath::start`], `y` the distance from the path on its right-hand side (down, for a path
//! drawn left to right), so the first baseline (`y = 0`) lies on the path. A flipped path runs
//! the other way, which puts the text on the other side.

use photocraft_doc::text::TextPath;

/// Most points a flattened path keeps (a guard against hostile paths).
const MAX_POINTS: usize = 200_000;
/// Flattening steps per cubic segment, by its control-polygon length.
const STEP_PX: f64 = 2.0;
const MAX_STEPS: usize = 256;

/// A path prepared for laying text along it: a polyline with cumulative arc lengths.
#[derive(Clone, Debug, PartialEq)]
pub struct OnPath {
    pts: Vec<(f64, f64)>,
    cum: Vec<f64>,
    start: f64,
}

fn finite(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

impl OnPath {
    /// Flattens the first subpath of `tp.path`. `None` when it has no length or holds
    /// non-finite coordinates.
    pub fn new(tp: &TextPath) -> Option<OnPath> {
        let sp = tp.path.subpaths.first()?;
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for [a, b, c, d] in sp.segments() {
            let q = [(a.x, a.y), (b.x, b.y), (c.x, c.y), (d.x, d.y)];
            if !q.iter().all(|p| finite(*p)) {
                return None;
            }
            let poly = q.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum::<f64>();
            let n = ((poly / STEP_PX).ceil() as usize).clamp(1, MAX_STEPS);
            if pts.is_empty() {
                pts.push(q[0]);
            }
            for i in 1..=n {
                let t = i as f64 / n as f64;
                let u = 1.0 - t;
                let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                pts.push((w0 * q[0].0 + w1 * q[1].0 + w2 * q[2].0 + w3 * q[3].0, w0 * q[0].1 + w1 * q[1].1 + w2 * q[2].1 + w3 * q[3].1));
                if pts.len() > MAX_POINTS {
                    return None;
                }
            }
        }
        pts.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
        if tp.flip {
            pts.reverse();
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        let mut prev: Option<(f64, f64)> = None;
        for p in &pts {
            if let Some(q) = prev {
                total += ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
            }
            cum.push(total);
            prev = Some(*p);
        }
        let start = if tp.start.is_finite() { f64::from(tp.start) } else { 0.0 };
        (total > 0.0 && total.is_finite()).then_some(OnPath { pts, cum, start })
    }

    /// Arc length of the path.
    pub fn len(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }

    /// Point and unit tangent at arc length `s` from the path's start, `None` past either end.
    pub fn at(&self, s: f64) -> Option<((f64, f64), (f64, f64))> {
        if !s.is_finite() || s < 0.0 || s > self.len() {
            return None;
        }
        let i = self.cum.partition_point(|c| *c < s).max(1);
        let (a, b) = (*self.pts.get(i - 1)?, *self.pts.get(i)?);
        let (ca, cb) = (*self.cum.get(i - 1)?, *self.cum.get(i)?);
        let seg = cb - ca;
        if seg <= 0.0 {
            return None;
        }
        let t = ((s - ca) / seg).clamp(0.0, 1.0);
        let tan = ((b.0 - a.0) / seg, (b.1 - a.1) / seg);
        Some(((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t), tan))
    }

    /// Maps a text-space point onto the path (see the module docs).
    pub fn apply(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let s = (self.start + x).clamp(0.0, self.len());
        let ((px, py), (tx, ty)) = self.at(s)?;
        Some((px - ty * y, py + tx * y))
    }

    /// Text space → path for a glyph whose advance centre is at text `x = centre`: a rotation
    /// about that point onto the path's tangent there, as `[a, b, c, d, e, f]` (see
    /// [`crate::raster::Xform`]). `None` when the centre lies past either end of the path.
    pub fn glyph_frame(&self, centre: f64) -> Option<[f64; 6]> {
        let ((px, py), (tx, ty)) = self.at(self.start + centre)?;
        Some([tx, ty, -ty, tx, px - tx * centre, py - ty * centre])
    }

    /// Text-space point nearest to `(x, y)`: (distance along the path minus the start, signed
    /// distance from it, positive on the side `y` grows towards).
    pub fn to_text(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !finite((x, y)) {
            return None;
        }
        let mut best: Option<(f64, f64, f64)> = None;
        for (i, w) in self.pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dy * dy;
            if l2 <= 0.0 {
                continue;
            }
            let t = (((x - a.0) * dx + (y - a.1) * dy) / l2).clamp(0.0, 1.0);
            let (qx, qy) = (a.0 + dx * t, a.1 + dy * t);
            let d2 = (x - qx).powi(2) + (y - qy).powi(2);
            if best.is_none_or(|(bd, _, _)| d2 < bd) {
                let l = l2.sqrt();
                let side = (dx * (y - a.1) - dy * (x - a.0)) / l;
                let s = self.cum.get(i).copied().unwrap_or(0.0) + l * t;
                best = Some((d2, s, side));
            }
        }
        best.map(|(_, s, side)| (s - self.start, side))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::vector::{Path, Subpath};

    fn line(flip: bool, start: f32) -> OnPath {
        let path = Path::new(vec![Subpath::polyline(&[(10.0, 50.0), (110.0, 50.0)])]);
        OnPath::new(&TextPath { path, start, flip }).unwrap()
    }

    #[test]
    fn a_straight_path_is_a_translated_baseline() {
        let p = line(false, 5.0);
        assert!((p.len() - 100.0).abs() < 1e-9);
        let (x, y) = p.apply(20.0, -8.0).unwrap();
        assert!((x - 35.0).abs() < 1e-9 && (y - 42.0).abs() < 1e-9, "{x},{y}");
        assert_eq!(p.to_text(35.0, 42.0).map(|(a, b)| (a.round(), b.round())), Some((20.0, -8.0)));
        // Past the end: no glyph frame.
        assert!(p.glyph_frame(200.0).is_none());
        assert!(p.glyph_frame(-10.0).is_none());
    }

    #[test]
    fn flipping_runs_the_text_the_other_way_on_the_other_side() {
        let p = line(true, 0.0);
        let (x, y) = p.apply(10.0, -8.0).unwrap();
        assert!((x - 100.0).abs() < 1e-9 && (y - 58.0).abs() < 1e-9, "{x},{y}");
    }

    #[test]
    fn glyphs_turn_with_a_curve() {
        // A quarter circle (approximate) from (0, 0) curving down to (100, 100).
        let k = 0.5523 * 100.0;
        let path = Path::new(vec![Subpath {
            closed: false,
            op: Default::default(),
            knots: vec![
                photocraft_doc::vector::Knot::smooth(
                    photocraft_geom::Point::new(0.0, 0.0),
                    photocraft_geom::Point::new(0.0, 0.0),
                    photocraft_geom::Point::new(k, 0.0),
                ),
                photocraft_doc::vector::Knot::smooth(
                    photocraft_geom::Point::new(100.0, 100.0),
                    photocraft_geom::Point::new(100.0, 100.0 - k),
                    photocraft_geom::Point::new(100.0, 100.0),
                ),
            ],
        }]);
        let p = OnPath::new(&TextPath { path, start: 0.0, flip: false }).unwrap();
        assert!((p.len() - 157.0).abs() < 2.0, "{}", p.len());
        let f0 = p.glyph_frame(1.0).unwrap();
        let f1 = p.glyph_frame(p.len() - 1.0).unwrap();
        assert!(f0[0] > 0.99 && f0[1].abs() < 0.05, "starts heading right: {f0:?}");
        assert!(f1[1] > 0.99 && f1[0].abs() < 0.05, "ends heading down: {f1:?}");
    }

    #[test]
    fn hostile_paths_are_refused() {
        let nan = Path::new(vec![Subpath::polyline(&[(f64::NAN, 0.0), (10.0, 0.0)])]);
        assert!(OnPath::new(&TextPath { path: nan, start: 0.0, flip: false }).is_none());
        let dot = Path::new(vec![Subpath::polyline(&[(3.0, 3.0), (3.0, 3.0)])]);
        assert!(OnPath::new(&TextPath { path: dot, start: 0.0, flip: false }).is_none());
        assert!(OnPath::new(&TextPath::default()).is_none());
        let huge = Path::new(vec![Subpath::polyline(&[(-1e300, 0.0), (1e300, 0.0)])]);
        let h = OnPath::new(&TextPath { path: huge, start: f32::NAN, flip: false });
        if let Some(h) = h {
            assert!(h.apply(0.0, 0.0).is_some_and(|(x, y)| x.is_finite() && y.is_finite()));
        }
        let p = line(false, 0.0);
        assert!(p.to_text(f64::NAN, 0.0).is_none());
        assert!(p.at(f64::INFINITY).is_none());
    }
}
