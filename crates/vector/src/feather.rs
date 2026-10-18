//! Shape feather (Properties › Feather on a shape layer): the shape's rendering seen through a
//! Gaussian-softened edge, as Photoshop applies a feathered vector mask to the shape.

use photocraft_geom::Rect;

/// Photoshop's feather tops out at 1000 px.
pub const MAX_FEATHER: f32 = 1000.0;

/// Largest area (pixels) rendered feathered; a larger one renders with a hard edge rather than
/// allocating an unbounded buffer.
pub(crate) const MAX_PIXELS: usize = 1 << 26;

/// Gaussian sigma of a feather of `px` pixels (sigma = radius, as the mask compositor fits it on
/// Photoshop files); 0 for non-finite or non-positive values.
pub fn sigma(px: f32) -> f32 {
    if px.is_finite() { px.clamp(0.0, MAX_FEATHER) } else { 0.0 }
}

/// `r` grown by the reach of a Gaussian of `sigma`.
pub(crate) fn grown(r: Rect, sigma: f32) -> Rect {
    let m = (sigma * 3.0).ceil() as i32 + 2;
    if r.is_empty() { r } else { Rect::new(r.x0.saturating_sub(m), r.y0.saturating_sub(m), r.x1.saturating_add(m), r.y1.saturating_add(m)) }
}

/// Radius of the three box passes whose variance matches `sigma`² (Wells 1986 / Kovesi).
fn box_radius(sigma: f32) -> usize {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    (((ideal.floor() as usize) | 1).max(1) - 1) / 2
}

/// One box pass over `line` (edges clamp).
fn box_pass(line: &mut [[f32; 4]], tmp: &mut Vec<[f32; 4]>, r: usize) {
    let len = line.len();
    if len == 0 {
        return;
    }
    tmp.clear();
    tmp.extend_from_slice(line);
    let last = len - 1;
    let at = |t: &[[f32; 4]], i: isize| t.get(i.clamp(0, last as isize) as usize).copied().unwrap_or_default();
    let r = r as isize;
    let n = (2 * r + 1) as f32;
    let mut acc = [0.0f32; 4];
    for i in -r..=r {
        let p = at(tmp, i);
        for (a, v) in acc.iter_mut().zip(p) {
            *a += v;
        }
    }
    for (i, o) in line.iter_mut().enumerate() {
        let i = i as isize;
        for (o, a) in o.iter_mut().zip(acc) {
            *o = a / n;
        }
        let (add, sub) = (at(tmp, i + r + 1), at(tmp, i - r));
        for ((a, x), y) in acc.iter_mut().zip(add).zip(sub) {
            *a += x - y;
        }
    }
}

/// Gaussian blur (three box passes per axis) of a `w`×`h` premultiplied RGBA plane in place.
pub(crate) fn blur(px: &mut [[f32; 4]], w: usize, h: usize, sigma: f32) {
    let r = box_radius(sigma);
    if r == 0 || w == 0 || h == 0 || px.len() != w.saturating_mul(h) {
        return;
    }
    let mut tmp = Vec::new();
    for row in px.chunks_exact_mut(w) {
        for _ in 0..3 {
            box_pass(row, &mut tmp, r);
        }
    }
    let mut col = Vec::with_capacity(h);
    for x in 0..w {
        col.clear();
        col.extend((0..h).map(|y| px.get(y * w + x).copied().unwrap_or_default()));
        for _ in 0..3 {
            box_pass(&mut col, &mut tmp, r);
        }
        for (y, v) in col.iter().enumerate() {
            if let Some(p) = px.get_mut(y * w + x) {
                *p = *v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_total_coverage_and_softens_the_edge() {
        let (w, h) = (40, 1);
        let mut px: Vec<[f32; 4]> = (0..w).map(|x| if x < 20 { [1.0; 4] } else { [0.0; 4] }).collect();
        blur(&mut px, w, h, 4.0);
        assert!((px[19][3] - 0.5).abs() < 0.15, "edge {}", px[19][3]);
        assert!(px[10][3] > 0.95 && px[30][3] < 0.05);
    }

    #[test]
    fn hostile_feathers_are_harmless() {
        assert_eq!(sigma(f32::NAN), 0.0);
        assert_eq!(sigma(-3.0), 0.0);
        assert_eq!(sigma(f32::INFINITY), 0.0);
        assert_eq!(sigma(1e9), MAX_FEATHER);
        let mut px = vec![[1.0; 4]; 3];
        blur(&mut px, 5, 5, 3.0); // wrong size: untouched
        assert_eq!(px, vec![[1.0; 4]; 3]);
        let g = grown(Rect::new(i32::MAX - 1, 0, i32::MAX, 1), MAX_FEATHER);
        assert_eq!(g.x1, i32::MAX);
    }
}
