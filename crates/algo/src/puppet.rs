//! Edit › Puppet Warp: pins on a triangle mesh over the layer's opaque region, deformed
//! as-rigidly-as-possible.
//!
//! * **Mesh.** The layer's coverage (alpha above a threshold, grown or shrunk by `expansion`) is
//!   rasterized onto a coarse block grid. The mesh follows the outline: points along the
//!   coverage boundary (every corner, straight runs every half cell) and on an interior grid at
//!   the density's spacing are Delaunay-triangulated and clipped to the coverage, so the mesh
//!   hugs the shape instead of stepping around it in whole cells. If that triangulation fails
//!   its checks (or wouldn't cover every covered block), a square grid keeping every cell that
//!   touches covered blocks, split into two triangles, is used instead.
//! * **Deformation.** The per-triangle ARAP energy of Liu et al. 2008 (the triangle form of
//!   Sorkine & Alexa 2007), `E = Σ_t Σ_(i,j)∈t w_ij |(v_i − v_j) − R_t (x_i − x_j)|²` with
//!   cotangent weights, minimized by alternating a local step (best rotation per triangle from
//!   the 2×2 covariance: `atan2(S10 − S01, S00 + S11)`) and a global step (one sparse SPD solve
//!   per axis, Jacobi-preconditioned conjugate gradients, warm-started). Pins are stiff soft
//!   constraints on the barycentric point under them. The first global step uses `R = I`
//!   (a harmonic deformation), which is the classic initialisation. Modes change the local
//!   step: Rigid fits rotations, Distort fits similarities (rotation × scale, i.e. ASAP), and
//!   Normal fits rotation × √scale, in between. A pin's rotation fixes `R` on the triangles
//!   around it. Mesh components without a pin stay where they are.
//! * **Rendering.** Triangles are rasterized with [`crate::warp::warp_triangles`] (tile-parallel,
//!   premultiplied, bilinear/bicubic), back to front by the depth of their nearest pin.

use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::transform::Interp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PuppetMode {
    Rigid,
    #[default]
    Normal,
    Distort,
}

impl PuppetMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "rigid" => Some(PuppetMode::Rigid),
            "normal" => Some(PuppetMode::Normal),
            "distort" => Some(PuppetMode::Distort),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PuppetDensity {
    Fewer,
    #[default]
    Normal,
    More,
}

impl PuppetDensity {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fewer" | "fewerpoints" => Some(PuppetDensity::Fewer),
            "normal" => Some(PuppetDensity::Normal),
            "more" | "morepoints" => Some(PuppetDensity::More),
            _ => None,
        }
    }

    fn cells(self) -> f64 {
        match self {
            PuppetDensity::Fewer => 12.0,
            PuppetDensity::Normal => 20.0,
            PuppetDensity::More => 32.0,
        }
    }
}

/// A pin: where it was placed (`src`) and where it is now (`dst`), document pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PuppetPin {
    pub src: [f64; 2],
    pub dst: [f64; 2],
    /// Fixed rotation of the mesh around the pin (degrees); `None` = automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<f64>,
    /// Pin depth: where the mesh overlaps itself, triangles nearest a higher pin draw on top.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

fn d_expansion() -> f64 {
    2.0
}

/// A complete puppet warp (the command's params and a smart filter's stored data).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PuppetWarp {
    #[serde(default)]
    pub pins: Vec<PuppetPin>,
    #[serde(default)]
    pub mode: PuppetMode,
    #[serde(default)]
    pub density: PuppetDensity,
    /// Grows (or with a negative value shrinks) the mesh beyond the opaque region, px.
    #[serde(default = "d_expansion")]
    pub expansion: f64,
}

impl PuppetWarp {
    pub fn is_identity(&self) -> bool {
        self.pins.iter().all(|p| p.src == p.dst && p.rotate.is_none_or(|r| r.rem_euclid(360.0) == 0.0))
    }
}

/// A triangle mesh in document pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PuppetMesh {
    pub verts: Vec<[f64; 2]>,
    pub tris: Vec<[usize; 3]>,
    /// Grid spacing (px).
    pub spacing: f64,
}

/// Coverage threshold: alpha above this counts as part of the puppet.
const ALPHA_MIN: f32 = 0.05;

/// Builds the mesh over the covered part of `src` inside `bounds`.
pub fn build_mesh(src: &Surface, bounds: Rect, density: PuppetDensity, expansion: f64) -> PuppetMesh {
    if bounds.is_empty() {
        return PuppetMesh::default();
    }
    let long = f64::from(bounds.width().max(bounds.height()));
    let spacing = (long / density.cells()).max(2.0);
    let e = expansion.clamp(-200.0, 200.0);
    // Coverage on a block grid (block = a quarter cell or less).
    let g = ((spacing / 4.0).floor() as i32).max(1);
    let pad = e.max(0.0).ceil() as i32 + g;
    let area = bounds.inflate(pad);
    let bw = (area.width() as i32 + g - 1) / g;
    let bh = (area.height() as i32 + g - 1) / g;
    let (bw, bh) = (bw as usize, bh as usize);
    let mut cov = vec![false; bw * bh];
    let fmt = src.format();
    let n = fmt.channels();
    if !fmt.alpha {
        for by in 0..bh {
            for bx in 0..bw {
                let r = Rect::new(area.x0 + bx as i32 * g, area.y0 + by as i32 * g, area.x0 + (bx as i32 + 1) * g, area.y0 + (by as i32 + 1) * g);
                cov[by * bw + bx] = !r.intersect(&bounds).is_empty();
            }
        }
    } else {
        // One block row per task (rows are independent).
        let scan = |by: usize, out: &mut [bool]| {
            let r = Rect::new(area.x0, area.y0 + by as i32 * g, area.x0 + bw as i32 * g, area.y0 + (by as i32 + 1) * g).intersect(&bounds);
            if r.is_empty() || !src.has_tiles_in(r) {
                return;
            }
            let row = src.read_region(r);
            let rw = r.width() as usize;
            for (i, p) in row.chunks_exact(n).enumerate() {
                if p[n - 1] > ALPHA_MIN {
                    let x = r.x0 + (i % rw) as i32;
                    out[((x - area.x0) / g) as usize] = true;
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            cov.par_chunks_mut(bw).enumerate().for_each(|(by, out)| scan(by, out));
        }
        #[cfg(target_arch = "wasm32")]
        cov.chunks_mut(bw).enumerate().for_each(|(by, out)| scan(by, out));
    }
    // Grow / shrink by the expansion (round stamp in block units).
    let eb = (e.abs() / f64::from(g)).round() as i32;
    if eb > 0 {
        let src_cov = cov.clone();
        let grow = e > 0.0;
        for by in 0..bh as i32 {
            for bx in 0..bw as i32 {
                let mut hit = !grow;
                'stamp: for dy in -eb..=eb {
                    for dx in -eb..=eb {
                        if dx * dx + dy * dy > eb * eb {
                            continue;
                        }
                        let (x, y) = (bx + dx, by + dy);
                        let v = x >= 0 && y >= 0 && x < bw as i32 && y < bh as i32 && src_cov[y as usize * bw + x as usize];
                        if grow && v {
                            hit = true;
                            break 'stamp;
                        }
                        if !grow && !v {
                            hit = false;
                            break 'stamp;
                        }
                    }
                }
                cov[by as usize * bw + bx as usize] = hit;
            }
        }
    }
    let c = Coverage { cov, bw, bh, g, ox: f64::from(area.x0), oy: f64::from(area.y0) };
    outline_mesh(&c, spacing).unwrap_or_else(|| grid_mesh(&c, spacing))
}

/// The layer's coverage on the block grid (row-major, `bw × bh` blocks of `g` px from `(ox, oy)`).
struct Coverage {
    cov: Vec<bool>,
    bw: usize,
    bh: usize,
    g: i32,
    ox: f64,
    oy: f64,
}

impl Coverage {
    /// Whether the block at `(x, y)` (block units, may be outside the grid) is set in `m`.
    fn at(&self, m: &[bool], x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= self.bw as i64 || y >= self.bh as i64 {
            return false;
        }
        m.get(y as usize * self.bw + x as usize).copied().unwrap_or(false)
    }

    /// The block holding document point `p`.
    fn block_of(&self, p: [f64; 2]) -> (i64, i64) {
        let g = f64::from(self.g);
        (((p[0] - self.ox) / g).floor() as i64, ((p[1] - self.oy) / g).floor() as i64)
    }
}

/// Most points the outline triangulation takes on (beyond, the grid mesh is used).
const MAX_OUTLINE_POINTS: usize = 8000;

/// A mesh that follows the outline: the coverage grown by one block is sampled along its
/// boundary (every corner, straight runs every half cell) and on an interior grid at the
/// cell spacing, Delaunay-triangulated, and clipped to the grown coverage. `None` when the
/// triangulation fails or wouldn't cover every covered block (the grid mesh is used then).
fn outline_mesh(c: &Coverage, spacing: f64) -> Option<PuppetMesh> {
    let (bw, bh) = (c.bw, c.bh);
    let g = f64::from(c.g);
    // The coverage grown by one block: its boundary runs half a block outside the covered
    // blocks, so a polygon through boundary block centres contains them.
    let mut outer = vec![false; bw * bh];
    for y in 0..bh as i64 {
        for x in 0..bw as i64 {
            if (-1..=1).any(|dy| (-1..=1).any(|dx| c.at(&c.cov, x + dx, y + dy))) {
                outer[y as usize * bw + x as usize] = true;
            }
        }
    }
    let inside = |x: i64, y: i64| c.at(&outer, x, y);
    let centre = |x: i64, y: i64| [c.ox + (x as f64 + 0.5) * g, c.oy + (y as f64 + 0.5) * g];
    // Samples, with a hash grid (cells of half the spacing) for distance queries.
    let r = spacing / 2.0;
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut hash: std::collections::HashMap<(i64, i64), Vec<[f64; 2]>> = std::collections::HashMap::new();
    let key = |p: [f64; 2]| ((p[0] / r).floor() as i64, (p[1] / r).floor() as i64);
    let near = |hash: &std::collections::HashMap<(i64, i64), Vec<[f64; 2]>>, p: [f64; 2], d: f64| {
        let (kx, ky) = key(p);
        (-1..=1).any(|dy| (-1..=1).any(|dx| hash.get(&(kx + dx, ky + dy)).is_some_and(|v| v.iter().any(|q| dist2(*q, p) < d * d))))
    };
    let add = |pts: &mut Vec<[f64; 2]>, hash: &mut std::collections::HashMap<(i64, i64), Vec<[f64; 2]>>, p: [f64; 2]| {
        pts.push(p);
        hash.entry(key(p)).or_default().push(p);
        pts.len() <= MAX_OUTLINE_POINTS
    };
    // Boundary blocks: corners first (where the outline turns), then straight runs.
    let mut runs = Vec::new();
    for y in 0..bh as i64 {
        for x in 0..bw as i64 {
            if !inside(x, y) {
                continue;
            }
            let (l, rt, u, d) = (inside(x - 1, y), inside(x + 1, y), inside(x, y - 1), inside(x, y + 1));
            let diag = inside(x - 1, y - 1) && inside(x + 1, y - 1) && inside(x - 1, y + 1) && inside(x + 1, y + 1);
            let convex = (!l || !rt) && (!u || !d);
            let concave = l && rt && u && d && !diag;
            if convex || concave {
                if !add(&mut pts, &mut hash, centre(x, y)) {
                    return None;
                }
            } else if !(l && rt && u && d) {
                runs.push(centre(x, y));
            }
        }
    }
    for p in runs {
        if !near(&hash, p, r) && !add(&mut pts, &mut hash, p) {
            return None;
        }
    }
    let boundary = pts.len();
    // Interior points on the cell grid, well inside and clear of the boundary samples.
    let cw = (bw as f64 * g / spacing).ceil() as i64;
    let ch = (bh as f64 * g / spacing).ceil() as i64;
    let h = 0.35 * spacing;
    for j in 0..=ch {
        for i in 0..=cw {
            let p = [c.ox + i as f64 * spacing, c.oy + j as f64 * spacing];
            let (x0, y0) = c.block_of([p[0] - h, p[1] - h]);
            let (x1, y1) = c.block_of([p[0] + h, p[1] + h]);
            if !(y0..=y1).all(|y| (x0..=x1).all(|x| inside(x, y))) || near(&hash, p, 0.45 * spacing) {
                continue;
            }
            if !add(&mut pts, &mut hash, p) {
                return None;
            }
        }
    }
    if boundary < 3 {
        return None;
    }
    // Row-major order keeps the solver's band narrow.
    pts.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
    let tris = delaunay(&pts)?;
    // Clip: a triangle stays when its centroid and edge midpoints lie in the grown coverage,
    // and it isn't a sliver (its cotangent weights would blow up).
    let in_outer = |p: [f64; 2]| {
        let (x, y) = c.block_of(p);
        inside(x, y)
    };
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let mut kept = Vec::new();
    for t in tris {
        let [a, b, d] = t.map(|i| pts.get(i).copied().unwrap_or_default());
        let cen = [(a[0] + b[0] + d[0]) / 3.0, (a[1] + b[1] + d[1]) / 3.0];
        if !(in_outer(cen) && in_outer(mid(a, b)) && in_outer(mid(b, d)) && in_outer(mid(d, a))) {
            continue;
        }
        let area = ((b[0] - a[0]) * (d[1] - a[1]) - (d[0] - a[0]) * (b[1] - a[1])).abs() / 2.0;
        let long = dist2(a, b).max(dist2(b, d)).max(dist2(d, a));
        if area < 0.01 * long {
            continue;
        }
        kept.push(t);
    }
    // Every covered block must lie in the mesh (its four corners in some triangle).
    let lw = bw + 1;
    let mut hit = vec![false; lw * (bh + 1)];
    for t in &kept {
        let [a, b, d] = t.map(|i| pts.get(i).copied().unwrap_or_default());
        let det = (b[0] - a[0]) * (d[1] - a[1]) - (d[0] - a[0]) * (b[1] - a[1]);
        if det.abs() < 1e-12 {
            continue;
        }
        let lx0 = ((a[0].min(b[0]).min(d[0]) - c.ox) / g).floor().max(0.0) as usize;
        let ly0 = ((a[1].min(b[1]).min(d[1]) - c.oy) / g).floor().max(0.0) as usize;
        let lx1 = (((a[0].max(b[0]).max(d[0]) - c.ox) / g).ceil().max(0.0) as usize).min(bw);
        let ly1 = (((a[1].max(b[1]).max(d[1]) - c.oy) / g).ceil().max(0.0) as usize).min(bh);
        for ly in ly0..=ly1 {
            for lx in lx0..=lx1 {
                let p = [c.ox + lx as f64 * g, c.oy + ly as f64 * g];
                let l1 = ((p[0] - a[0]) * (d[1] - a[1]) - (d[0] - a[0]) * (p[1] - a[1])) / det;
                let l2 = ((b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1])) / det;
                if l1 >= -1e-9
                    && l2 >= -1e-9
                    && 1.0 - l1 - l2 >= -1e-9
                    && let Some(h) = hit.get_mut(ly * lw + lx)
                {
                    *h = true;
                }
            }
        }
    }
    let corner = |x: usize, y: usize| hit.get(y * lw + x).copied().unwrap_or(false);
    for y in 0..bh {
        for x in 0..bw {
            if c.cov.get(y * bw + x).copied().unwrap_or(false) && !(corner(x, y) && corner(x + 1, y) && corner(x, y + 1) && corner(x + 1, y + 1)) {
                return None;
            }
        }
    }
    // Drop unused points (order preserved).
    let mut index = vec![usize::MAX; pts.len()];
    let mut mesh = PuppetMesh { spacing, ..Default::default() };
    for t in &mut kept {
        for v in t.iter_mut() {
            let slot = index.get_mut(*v)?;
            if *slot == usize::MAX {
                *slot = usize::MAX - 1;
            }
        }
    }
    for (i, slot) in index.iter_mut().enumerate() {
        if *slot != usize::MAX {
            *slot = mesh.verts.len();
            mesh.verts.push(*pts.get(i)?);
        }
    }
    for t in kept {
        let mut out = [0; 3];
        for (o, v) in out.iter_mut().zip(t) {
            *o = *index.get(v)?;
        }
        mesh.tris.push(out);
    }
    Some(mesh)
}

/// Bowyer–Watson Delaunay triangulation (counter-clockwise triangles over `p`'s indices).
/// Points are normalised and nudged by a tiny deterministic jitter so co-circular grids
/// don't tie. `None` if the result fails its consistency checks.
fn delaunay(p: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
    let n = p.len();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for q in p {
        x0 = x0.min(q[0]);
        y0 = y0.min(q[1]);
        x1 = x1.max(q[0]);
        y1 = y1.max(q[1]);
    }
    let ext = (x1 - x0).max(y1 - y0);
    if n < 3 || !ext.is_finite() || ext <= 0.0 {
        return None;
    }
    let mut q: Vec<[f64; 2]> = p
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let hsh = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let jx = ((hsh >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 1e-6;
            let jy = (((hsh.rotate_left(29)) >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 1e-6;
            [(v[0] - x0) / ext + jx, (v[1] - y0) / ext + jy]
        })
        .collect();
    // A super triangle far around the unit square.
    q.extend([[-100.0, -100.0], [300.0, -100.0], [-100.0, 300.0]]);
    let orient = |q: &[[f64; 2]], t: [usize; 3]| -> Option<f64> {
        let [a, b, c] = [*q.get(t[0])?, *q.get(t[1])?, *q.get(t[2])?];
        Some((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
    };
    let circ = |q: &[[f64; 2]], t: [usize; 3]| -> Option<[f64; 3]> {
        let [a, b, c] = [*q.get(t[0])?, *q.get(t[1])?, *q.get(t[2])?];
        let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
        if d.abs() < 1e-300 {
            return None;
        }
        let (a2, b2, c2) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1], c[0] * c[0] + c[1] * c[1]);
        let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
        let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
        Some([ux, uy, (a[0] - ux).powi(2) + (a[1] - uy).powi(2)])
    };
    let mut tris: Vec<([usize; 3], [f64; 3])> = vec![([n, n + 1, n + 2], circ(&q, [n, n + 1, n + 2])?)];
    let mut bad = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        let v = *q.get(i)?;
        bad.clear();
        for (k, (_, cc)) in tris.iter().enumerate() {
            if (v[0] - cc[0]).powi(2) + (v[1] - cc[1]).powi(2) < cc[2] {
                bad.push(k);
            }
        }
        if bad.is_empty() {
            return None;
        }
        edges.clear();
        for &k in &bad {
            let t = tris.get(k)?.0;
            edges.extend([(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]);
        }
        // Remove the cavity (highest index first so swap_remove keeps the rest valid).
        for &k in bad.iter().rev() {
            tris.swap_remove(k);
        }
        for &(a, b) in &edges {
            if edges.contains(&(b, a)) {
                continue;
            }
            let t = [a, b, i];
            if orient(&q, t)? <= 0.0 {
                return None;
            }
            tris.push((t, circ(&q, t)?));
        }
    }
    // A triangulation of n points inside the super triangle has 2n + 1 triangles.
    if tris.len() != 2 * n + 1 {
        return None;
    }
    Some(tris.into_iter().map(|(t, _)| t).filter(|t| t.iter().all(|&v| v < n)).collect())
}

/// The square grid mesh: every cell that touches a covered block, split into two triangles.
fn grid_mesh(c: &Coverage, spacing: f64) -> PuppetMesh {
    let (cov, bw, bh, g, ox, oy) = (&c.cov, c.bw, c.bh, c.g, c.ox, c.oy);
    let cw = (f64::from(bw as i32 * g) / spacing).ceil() as usize;
    let ch = (f64::from(bh as i32 * g) / spacing).ceil() as usize;
    let mut index = vec![usize::MAX; (cw + 1) * (ch + 1)];
    let mut mesh = PuppetMesh { spacing, ..Default::default() };
    for cj in 0..ch {
        for ci in 0..cw {
            let bx0 = ((ci as f64 * spacing) / f64::from(g)).floor() as usize;
            let by0 = ((cj as f64 * spacing) / f64::from(g)).floor() as usize;
            let bx1 = (((ci + 1) as f64 * spacing) / f64::from(g)).ceil().min(bw as f64) as usize;
            let by1 = (((cj + 1) as f64 * spacing) / f64::from(g)).ceil().min(bh as f64) as usize;
            let any = (by0..by1).any(|y| (bx0..bx1).any(|x| cov[y * bw + x]));
            if !any {
                continue;
            }
            let mut v = |i: usize, j: usize| {
                let k = j * (cw + 1) + i;
                if index[k] == usize::MAX {
                    index[k] = mesh.verts.len();
                    mesh.verts.push([ox + i as f64 * spacing, oy + j as f64 * spacing]);
                }
                index[k]
            };
            let (a, b, c, d) = (v(ci, cj), v(ci + 1, cj), v(ci + 1, cj + 1), v(ci, cj + 1));
            if (ci + cj) % 2 == 0 {
                mesh.tris.push([a, b, c]);
                mesh.tris.push([a, c, d]);
            } else {
                mesh.tris.push([a, b, d]);
                mesh.tris.push([b, c, d]);
            }
        }
    }
    mesh
}

/// A compressed sparse row matrix (symmetric, used by CG).
struct Csr {
    rows: Vec<usize>,
    cols: Vec<usize>,
    vals: Vec<f64>,
    diag: Vec<f64>,
}

impl Csr {
    fn from_triplets(n: usize, mut t: Vec<(usize, usize, f64)>) -> Csr {
        t.sort_by_key(|a| (a.0, a.1));
        let mut rows = vec![0usize; n + 1];
        let mut cols = Vec::with_capacity(t.len());
        let mut vals: Vec<f64> = Vec::with_capacity(t.len());
        let mut last = (usize::MAX, usize::MAX);
        for (i, j, v) in t {
            if (i, j) == last {
                *vals.last_mut().unwrap_or(&mut 0.0) += v;
                continue;
            }
            last = (i, j);
            cols.push(j);
            vals.push(v);
            rows[i + 1] = cols.len();
        }
        for i in 0..n {
            rows[i + 1] = rows[i + 1].max(rows[i]);
        }
        let mut diag = vec![1.0; n];
        for (i, d) in diag.iter_mut().enumerate() {
            for k in rows[i]..rows[i + 1] {
                if cols[k] == i && vals[k] > 0.0 {
                    *d = vals[k];
                }
            }
        }
        Csr { rows, cols, vals, diag }
    }

    fn mul(&self, x: &[f64], out: &mut [f64]) {
        for (i, o) in out.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in self.rows[i]..self.rows[i + 1] {
                s += self.vals[k] * x[self.cols[k]];
            }
            *o = s;
        }
    }

    /// Jacobi-preconditioned conjugate gradients, warm-started from `x`.
    fn solve(&self, b: &[f64], x: &mut [f64]) {
        let n = b.len();
        let mut r = vec![0.0; n];
        self.mul(x, &mut r);
        for i in 0..n {
            r[i] = b[i] - r[i];
        }
        let bn: f64 = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
        let mut z: Vec<f64> = r.iter().zip(&self.diag).map(|(r, d)| r / d).collect();
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let mut ap = vec![0.0; n];
        for _ in 0..(4 * n).clamp(50, 2000) {
            if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= 1e-12 * bn {
                break;
            }
            self.mul(&p, &mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
            if pap.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
            }
            for i in 0..n {
                z[i] = r[i] / self.diag[i];
            }
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz2 / rz;
            rz = rz2;
            for i in 0..n {
                p[i] = z[i] + beta * p[i];
            }
        }
    }
}

/// Banded Cholesky factor `A = L Lᵀ` (grid meshes number their vertices row by row, so the
/// system's bandwidth is about two mesh rows): factored once per pin set, then each global step
/// is two triangular solves.
struct BandChol {
    n: usize,
    p: usize,
    /// Row `i` holds `L[i][i - p ..= i]`.
    l: Vec<f64>,
}

impl BandChol {
    /// `None` when the band would be too large (or the matrix isn't positive definite).
    fn new(a: &Csr) -> Option<BandChol> {
        let n = a.diag.len();
        let mut p = 0;
        for i in 0..n {
            for k in a.rows[i]..a.rows[i + 1] {
                p = p.max(i.abs_diff(a.cols[k]));
            }
        }
        if n.saturating_mul(p + 1) > 40_000_000 {
            return None;
        }
        let w = p + 1;
        let mut l = vec![0.0f64; n * w];
        for i in 0..n {
            for k in a.rows[i]..a.rows[i + 1] {
                let j = a.cols[k];
                if j <= i {
                    l[i * w + (j + p - i)] = a.vals[k];
                }
            }
        }
        for i in 0..n {
            let lo = i.saturating_sub(p);
            for j in lo..=i {
                let jlo = j.saturating_sub(p).max(lo);
                let mut s = l[i * w + (j + p - i)];
                for k in jlo..j {
                    s -= l[i * w + (k + p - i)] * l[j * w + (k + p - j)];
                }
                if i == j {
                    if s <= 0.0 || !s.is_finite() {
                        return None;
                    }
                    l[i * w + p] = s.sqrt();
                } else {
                    l[i * w + (j + p - i)] = s / l[j * w + p];
                }
            }
        }
        Some(BandChol { n, p, l })
    }

    #[allow(clippy::needless_range_loop)] // band index arithmetic reads clearer with indices
    fn solve(&self, b: &[f64], x: &mut [f64]) {
        let (n, p, w) = (self.n, self.p, self.p + 1);
        let mut y = b.to_vec();
        for i in 0..n {
            let mut s = y[i];
            for k in i.saturating_sub(p)..i {
                s -= self.l[i * w + (k + p - i)] * y[k];
            }
            y[i] = s / self.l[i * w + p];
        }
        for i in (0..n).rev() {
            let mut s = y[i];
            for k in i + 1..(i + p + 1).min(n) {
                s -= self.l[k * w + (i + p - k)] * x[k];
            }
            x[i] = s / self.l[i * w + p];
        }
    }
}

/// A pin bound to the mesh: barycentric weights on a triangle's vertices.
#[derive(Clone, Debug)]
struct Bound {
    verts: [usize; 3],
    bary: [f64; 3],
}

/// Prepared ARAP solver for one mesh and a set of pin source positions.
pub struct PuppetSolver {
    pub mesh: PuppetMesh,
    /// Cotangent weight of each triangle's edges: `w[t][k]` for the edge opposite corner `k`.
    w: Vec<[f64; 3]>,
    bound: Vec<Bound>,
    /// Triangles whose rotation a pin fixes: (pin index, triangles).
    pin_tris: Vec<Vec<usize>>,
    /// Vertices of components without pins (held at rest).
    anchored: Vec<bool>,
    matrix: Csr,
    chol: Option<BandChol>,
    pin_weight: f64,
}

fn cot(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    // Cotangent of the angle at `a`.
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
    let cross = (u[0] * v[1] - u[1] * v[0]).abs();
    if cross < 1e-12 { 0.0 } else { (u[0] * v[0] + u[1] * v[1]) / cross }
}

fn locate(mesh: &PuppetMesh, p: [f64; 2]) -> Bound {
    for t in &mesh.tris {
        let [a, b, c] = t.map(|i| mesh.verts[i]);
        let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        if det.abs() < 1e-12 {
            continue;
        }
        let l1 = ((p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1])) / det;
        let l2 = ((b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1])) / det;
        let l0 = 1.0 - l1 - l2;
        if l0 >= -1e-9 && l1 >= -1e-9 && l2 >= -1e-9 {
            return Bound { verts: *t, bary: [l0, l1, l2] };
        }
    }
    // Outside the mesh: the nearest vertex.
    let i = (0..mesh.verts.len()).min_by(|&a, &b| dist2(mesh.verts[a], p).total_cmp(&dist2(mesh.verts[b], p))).unwrap_or(0);
    Bound { verts: [i, i, i], bary: [1.0, 0.0, 0.0] }
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

impl PuppetSolver {
    /// Prepares the solver for pins placed at `pin_src` (rotation-fixed pins listed in
    /// `fixed_rotation`).
    pub fn new(mesh: PuppetMesh, pin_src: &[[f64; 2]], fixed_rotation: &[bool]) -> Self {
        let nv = mesh.verts.len();
        let w: Vec<[f64; 3]> = mesh
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.verts[i]);
                [0.5 * cot(a, b, c), 0.5 * cot(b, c, a), 0.5 * cot(c, a, b)]
            })
            .collect();
        // Without vertices there is nothing to bind a pin to (`locate` would fall back to a vertex 0
        // that doesn't exist, #951); every pin loop zips `bound`, so the pins are then ignored.
        let bound: Vec<Bound> = if nv == 0 { Vec::new() } else { pin_src.iter().map(|p| locate(&mesh, *p)).collect() };
        // Connected components (union-find over triangle edges).
        let mut parent: Vec<usize> = (0..nv).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for t in &mesh.tris {
            for k in 1..3 {
                let (a, b) = (find(&mut parent, t[0]), find(&mut parent, t[k]));
                if a != b {
                    parent[a] = b;
                }
            }
        }
        let mut pinned_root = vec![false; nv];
        for b in &bound {
            for &v in &b.verts {
                let r = find(&mut parent, v);
                pinned_root[r] = true;
            }
        }
        let anchored: Vec<bool> = (0..nv).map(|v| !pinned_root[find(&mut parent, v)]).collect();
        // Rotation-fixed pins: the triangles around the pin's nearest vertex.
        let pin_tris: Vec<Vec<usize>> = bound
            .iter()
            .zip(pin_src)
            .enumerate()
            .map(|(k, (b, p))| {
                if !fixed_rotation.get(k).copied().unwrap_or(false) {
                    return Vec::new();
                }
                let v = *b.verts.iter().min_by(|&&a, &&c| dist2(mesh.verts[a], *p).total_cmp(&dist2(mesh.verts[c], *p))).unwrap_or(&0);
                mesh.tris.iter().enumerate().filter(|(_, t)| t.contains(&v)).map(|(i, _)| i).collect()
            })
            .collect();
        let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(mesh.tris.len() * 12 + nv);
        let mut diag_sum = 0.0;
        for (t, wt) in mesh.tris.iter().zip(&w) {
            for k in 0..3 {
                let (i, j) = (t[(k + 1) % 3], t[(k + 2) % 3]);
                let wk = wt[k];
                trip.push((i, i, wk));
                trip.push((j, j, wk));
                trip.push((i, j, -wk));
                trip.push((j, i, -wk));
                diag_sum += 2.0 * wk;
            }
        }
        let pin_weight = 1e4 * (diag_sum / nv.max(1) as f64).max(1.0);
        for b in &bound {
            for a in 0..3 {
                for c in 0..3 {
                    trip.push((b.verts[a], b.verts[c], pin_weight * b.bary[a] * b.bary[c]));
                }
            }
        }
        for (v, &an) in anchored.iter().enumerate() {
            if an {
                trip.push((v, v, pin_weight));
            }
        }
        let matrix = Csr::from_triplets(nv, trip);
        let chol = BandChol::new(&matrix);
        PuppetSolver { mesh, w, bound, pin_tris, anchored, matrix, chol, pin_weight }
    }

    /// The ARAP energy of `v` for the given per-triangle linear maps (plus the pin terms).
    fn energy_with(&self, v: &[[f64; 2]], rots: &[[f64; 4]], pin_dst: &[[f64; 2]]) -> f64 {
        let x = &self.mesh.verts;
        let mut e = 0.0;
        for ((t, wt), r) in self.mesh.tris.iter().zip(&self.w).zip(rots) {
            for k in 0..3 {
                let (i, j) = (t[(k + 1) % 3], t[(k + 2) % 3]);
                let d = [v[i][0] - v[j][0], v[i][1] - v[j][1]];
                let ex = [x[i][0] - x[j][0], x[i][1] - x[j][1]];
                let re = [r[0] * ex[0] + r[1] * ex[1], r[2] * ex[0] + r[3] * ex[1]];
                e += wt[k] * ((d[0] - re[0]).powi(2) + (d[1] - re[1]).powi(2));
            }
        }
        for (b, d) in self.bound.iter().zip(pin_dst) {
            let p = (0..3).fold([0.0, 0.0], |a, k| [a[0] + b.bary[k] * v[b.verts[k]][0], a[1] + b.bary[k] * v[b.verts[k]][1]]);
            e += self.pin_weight * dist2(p, *d);
        }
        e
    }

    fn local(&self, v: &[[f64; 2]], mode: PuppetMode, fixed: &[Option<f64>]) -> Vec<[f64; 4]> {
        let x = &self.mesh.verts;
        let mut rots: Vec<[f64; 4]> = self
            .mesh
            .tris
            .iter()
            .zip(&self.w)
            .map(|(t, wt)| {
                let mut s = [0.0f64; 4];
                let mut norm = 0.0;
                for k in 0..3 {
                    let (i, j) = (t[(k + 1) % 3], t[(k + 2) % 3]);
                    let d = [v[i][0] - v[j][0], v[i][1] - v[j][1]];
                    let e = [x[i][0] - x[j][0], x[i][1] - x[j][1]];
                    s[0] += wt[k] * d[0] * e[0];
                    s[1] += wt[k] * d[0] * e[1];
                    s[2] += wt[k] * d[1] * e[0];
                    s[3] += wt[k] * d[1] * e[1];
                    norm += wt[k] * (e[0] * e[0] + e[1] * e[1]);
                }
                let (c, sn) = (s[0] + s[3], s[2] - s[1]);
                let th = sn.atan2(c);
                let scale = match mode {
                    PuppetMode::Rigid => 1.0,
                    PuppetMode::Normal => (c.hypot(sn) / norm.max(1e-12)).sqrt(),
                    PuppetMode::Distort => c.hypot(sn) / norm.max(1e-12),
                };
                let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
                let (sth, cth) = th.sin_cos();
                [scale * cth, -scale * sth, scale * sth, scale * cth]
            })
            .collect();
        for (k, tris) in self.pin_tris.iter().enumerate() {
            if let Some(Some(deg)) = fixed.get(k) {
                let (s, c) = deg.to_radians().sin_cos();
                for &t in tris {
                    rots[t] = [c, -s, s, c];
                }
            }
        }
        rots
    }

    fn global(&self, rots: &[[f64; 4]], pin_dst: &[[f64; 2]], v: &mut [[f64; 2]]) {
        let nv = v.len();
        let x = &self.mesh.verts;
        let mut bx = vec![0.0; nv];
        let mut by = vec![0.0; nv];
        for ((t, wt), r) in self.mesh.tris.iter().zip(&self.w).zip(rots) {
            for k in 0..3 {
                let (i, j) = (t[(k + 1) % 3], t[(k + 2) % 3]);
                let e = [x[i][0] - x[j][0], x[i][1] - x[j][1]];
                let re = [r[0] * e[0] + r[1] * e[1], r[2] * e[0] + r[3] * e[1]];
                bx[i] += wt[k] * re[0];
                by[i] += wt[k] * re[1];
                bx[j] -= wt[k] * re[0];
                by[j] -= wt[k] * re[1];
            }
        }
        for (b, d) in self.bound.iter().zip(pin_dst) {
            for k in 0..3 {
                bx[b.verts[k]] += self.pin_weight * b.bary[k] * d[0];
                by[b.verts[k]] += self.pin_weight * b.bary[k] * d[1];
            }
        }
        for (i, &an) in self.anchored.iter().enumerate() {
            if an {
                bx[i] += self.pin_weight * x[i][0];
                by[i] += self.pin_weight * x[i][1];
            }
        }
        let mut sx: Vec<f64> = v.iter().map(|p| p[0]).collect();
        let mut sy: Vec<f64> = v.iter().map(|p| p[1]).collect();
        match &self.chol {
            Some(c) => {
                c.solve(&bx, &mut sx);
                c.solve(&by, &mut sy);
            }
            None => {
                self.matrix.solve(&bx, &mut sx);
                self.matrix.solve(&by, &mut sy);
            }
        }
        for (i, p) in v.iter_mut().enumerate() {
            *p = [sx[i], sy[i]];
        }
    }

    /// Deforms the mesh: `pin_dst` (one per pin given to [`Self::new`]), optional fixed pin
    /// rotations (degrees), `iterations` local/global rounds, warm-started from `init` when
    /// given. Returns the deformed vertices and the rigid ARAP energy after each round.
    pub fn solve(
        &self,
        pin_dst: &[[f64; 2]],
        rotations: &[Option<f64>],
        mode: PuppetMode,
        iterations: usize,
        init: Option<&[[f64; 2]]>,
    ) -> (Vec<[f64; 2]>, Vec<f64>) {
        let nv = self.mesh.verts.len();
        let mut v: Vec<[f64; 2]> = match init {
            Some(i) if i.len() == nv => i.to_vec(),
            _ => self.mesh.verts.clone(),
        };
        if nv == 0 {
            return (v, Vec::new());
        }
        if init.is_none_or(|i| i.len() != nv) {
            // Start from R = I, or with rotated pins, each triangle turned by the inverse-distance
            // weighted angle of the rotated pins (ARAP alone propagates a turn only slowly).
            let rotated: Vec<([f64; 2], f64)> = self
                .bound
                .iter()
                .zip(rotations)
                .filter_map(|(b, r)| {
                    r.map(|deg| {
                        (
                            (0..3).fold([0.0, 0.0], |a, k| {
                                [a[0] + b.bary[k] * self.mesh.verts[b.verts[k]][0], a[1] + b.bary[k] * self.mesh.verts[b.verts[k]][1]]
                            }),
                            deg.to_radians(),
                        )
                    })
                })
                .collect();
            let r: Vec<[f64; 4]> = self
                .mesh
                .tris
                .iter()
                .map(|t| {
                    if rotated.is_empty() {
                        return [1.0, 0.0, 0.0, 1.0];
                    }
                    let c = t.iter().fold([0.0, 0.0], |a, &i| [a[0] + self.mesh.verts[i][0] / 3.0, a[1] + self.mesh.verts[i][1] / 3.0]);
                    let (mut sw, mut sa) = (0.0, 0.0);
                    for (p, a) in &rotated {
                        let w = 1.0 / (dist2(*p, c) + 1.0);
                        sw += w;
                        sa += w * a;
                    }
                    let (s, co) = (sa / sw).sin_cos();
                    [co, -s, s, co]
                })
                .collect();
            self.global(&r, pin_dst, &mut v);
        }
        let mut energies = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let rots = self.local(&v, mode, rotations);
            self.global(&rots, pin_dst, &mut v);
            let rigid = self.local(&v, PuppetMode::Rigid, rotations);
            energies.push(self.energy_with(&v, &rigid, pin_dst));
        }
        (v, energies)
    }

    /// Energy of the current state with the rigid best fit (for monitoring).
    pub fn energy(&self, v: &[[f64; 2]], pin_dst: &[[f64; 2]], rotations: &[Option<f64>]) -> f64 {
        let rots = self.local(v, PuppetMode::Rigid, rotations);
        self.energy_with(v, &rots, pin_dst)
    }

    /// Triangle draw order (back to front) for pins with depths.
    pub fn draw_order(&self, depths: &[i32], pin_src: &[[f64; 2]]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.mesh.tris.len()).collect();
        if depths.iter().all(|d| *d == 0) || pin_src.is_empty() {
            return order;
        }
        let depth_of = |t: usize| {
            let c = self.mesh.tris[t].iter().fold([0.0, 0.0], |a, &i| [a[0] + self.mesh.verts[i][0] / 3.0, a[1] + self.mesh.verts[i][1] / 3.0]);
            let k = (0..pin_src.len()).min_by(|&a, &b| dist2(pin_src[a], c).total_cmp(&dist2(pin_src[b], c))).unwrap_or(0);
            depths.get(k).copied().unwrap_or(0)
        };
        let d: Vec<i32> = order.iter().map(|&t| depth_of(t)).collect();
        order.sort_by_key(|&t| d[t]);
        order
    }
}

/// Number of local/global rounds used by the command.
pub const ITERATIONS: usize = 40;

/// The deformed mesh of a puppet warp: (solver, deformed vertices, draw order).
pub fn deform(src: &Surface, bounds: Rect, w: &PuppetWarp, iterations: usize) -> (PuppetSolver, Vec<[f64; 2]>, Vec<usize>) {
    let mesh = build_mesh(src, bounds, w.density, w.expansion);
    let pin_src: Vec<[f64; 2]> = w.pins.iter().map(|p| p.src).collect();
    let fixed: Vec<bool> = w.pins.iter().map(|p| p.rotate.is_some()).collect();
    let solver = PuppetSolver::new(mesh, &pin_src, &fixed);
    let dst: Vec<[f64; 2]> = w.pins.iter().map(|p| p.dst).collect();
    let rot: Vec<Option<f64>> = w.pins.iter().map(|p| p.rotate).collect();
    let (v, _) = solver.solve(&dst, &rot, w.mode, iterations, None);
    let depths: Vec<i32> = w.pins.iter().map(|p| p.depth).collect();
    let order = solver.draw_order(&depths, &pin_src);
    (solver, v, order)
}

/// Rasterizes the mesh deformation of `src` (inside `bounds`): the deformed triangles only, so
/// pixels outside the mesh are dropped.
pub fn render(src: &Surface, bounds: Rect, mesh: &PuppetMesh, deformed: &[[f64; 2]], order: &[usize], interp: Interp) -> Surface {
    let verts: Vec<([f64; 2], [f64; 2])> = deformed.iter().zip(&mesh.verts).map(|(d, s)| (*d, *s)).collect();
    let tris: Vec<[usize; 3]> = order.iter().map(|&t| mesh.tris[t]).collect();
    crate::warp::warp_triangles(src, bounds, &verts, &tris, interp)
}

/// The largest area (px) a puppet warp's result may span: 2³⁰, about 16000 tiles. A pin dragged
/// absurdly far stretches the mesh over far more, and the renderer allocates bookkeeping for
/// every tile of it before drawing (#1019).
pub const MAX_WARP_PIXELS: f64 = (1u64 << 30) as f64;

/// Applies a puppet warp to `src` (content inside `bounds`). `None` when the deformed mesh would
/// span more than [`MAX_WARP_PIXELS`].
pub fn puppet_warp(src: &Surface, bounds: Rect, w: &PuppetWarp, interp: Interp) -> Option<Surface> {
    let (solver, v, order) = deform(src, bounds, w, ITERATIONS);
    // No triangles (an expansion that shrinks every pixel away): nothing to warp. Rendering the
    // empty mesh would drop every pixel of the layer.
    if solver.mesh.tris.is_empty() {
        return Some(src.clone());
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &v {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let area = (x1 - x0 + 1.0) * (y1 - y0 + 1.0);
    if area.is_nan() || area > MAX_WARP_PIXELS {
        return None;
    }
    Some(render(src, bounds, &solver.mesh, &v, &order, interp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    fn blob(st: SampleType) -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, st, true));
        // An L shape: a vertical bar and a foot.
        for y in 10..70 {
            for x in 20..36 {
                s.write_pixel(x, y, &[((x + y) % 7) as f32 / 6.0, 0.5, (y as f32) / 80.0, 1.0]);
            }
        }
        for y in 56..70 {
            for x in 36..70 {
                s.write_pixel(x, y, &[0.2, ((x * 3) % 5) as f32 / 4.0, 0.8, 1.0]);
            }
        }
        s
    }

    #[test]
    fn mesh_covers_the_shape_only() {
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let m = build_mesh(&s, b, PuppetDensity::Normal, 2.0);
        assert!(m.tris.len() > 20);
        let inside = |p: [f64; 2]| {
            m.tris.iter().any(|t| {
                let xs = t.map(|i| m.verts[i]);
                let (x0, x1) = (xs.iter().map(|v| v[0]).fold(f64::MAX, f64::min), xs.iter().map(|v| v[0]).fold(f64::MIN, f64::max));
                let (y0, y1) = (xs.iter().map(|v| v[1]).fold(f64::MAX, f64::min), xs.iter().map(|v| v[1]).fold(f64::MIN, f64::max));
                p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1
            })
        };
        assert!(inside([28.0, 30.0]) && inside([60.0, 62.0]));
        assert!(!inside([60.0, 20.0]), "the empty corner of the L has no mesh");
        let more = build_mesh(&s, b, PuppetDensity::More, 2.0);
        let fewer = build_mesh(&s, b, PuppetDensity::Fewer, 2.0);
        assert!(more.tris.len() > m.tris.len() && m.tris.len() > fewer.tris.len());
    }

    fn alpha(s: &Surface, x: i32, y: i32) -> f32 {
        let mut px = [0.0f32; 4];
        s.read_pixel(x, y, &mut px);
        px[3]
    }

    fn coverage_of(s: &Surface, density: PuppetDensity) -> (Coverage, f64) {
        // Mirrors `build_mesh` without expansion, for checking the outline path directly.
        let b = s.content_bounds();
        let spacing = (f64::from(b.width().max(b.height())) / density.cells()).max(2.0);
        let g = ((spacing / 4.0).floor() as i32).max(1);
        let area = b.inflate(g);
        let (bw, bh) = (((area.width() as i32 + g - 1) / g) as usize, ((area.height() as i32 + g - 1) / g) as usize);
        let mut cov = vec![false; bw * bh];
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if alpha(s, x, y) > ALPHA_MIN {
                    cov[((y - area.y0) / g) as usize * bw + ((x - area.x0) / g) as usize] = true;
                }
            }
        }
        (Coverage { cov, bw, bh, g, ox: f64::from(area.x0), oy: f64::from(area.y0) }, spacing)
    }

    fn mesh_area(m: &PuppetMesh) -> f64 {
        m.tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.verts[i]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    fn disc() -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        for y in 0..120 {
            for x in 0..120 {
                if (x as f64 - 60.0).powi(2) + (y as f64 - 60.0).powi(2) <= 50.0f64.powi(2) {
                    s.write_pixel(x, y, &[0.5, 0.5, 0.5, 1.0]);
                }
            }
        }
        s
    }

    #[test]
    fn the_mesh_follows_the_outline() {
        // TOOL-214-6: a disc and an L both get the outline triangulation (not the grid
        // fallback), it hugs the shape far tighter than whole grid cells, and every covered
        // pixel stays inside it.
        for (s, name) in [(disc(), "disc"), (blob(SampleType::U8), "L")] {
            for d in [PuppetDensity::Fewer, PuppetDensity::Normal, PuppetDensity::More] {
                let (c, spacing) = coverage_of(&s, d);
                let m = outline_mesh(&c, spacing).unwrap_or_else(|| panic!("{name} {d:?}: fell back to the grid"));
                let grid = grid_mesh(&c, spacing);
                let covered = c.cov.iter().filter(|&&v| v).count() as f64 * f64::from(c.g * c.g);
                let (a, ga) = (mesh_area(&m), mesh_area(&grid));
                assert!(a >= covered * 0.98 && a < ga, "{name} {d:?}: mesh {a}, grid {ga}, covered {covered}");
                assert!(a - covered < (ga - covered) * 0.75, "{name} {d:?}: mesh {a}, grid {ga}, covered {covered}");
                // Boundary vertices sit off the cell lattice (on the outline).
                let off = m.verts.iter().filter(|v| ((v[0] - c.ox) / spacing).fract().abs() > 1e-6).count();
                assert!(off > 8, "{name} {d:?}");
                // `build_mesh` (with an expansion) takes the same path.
                let built = build_mesh(&s, s.content_bounds(), d, 2.0);
                assert!(built.verts.iter().any(|v| ((v[0] - c.ox) / spacing).fract().abs() > 1e-6), "{name} {d:?}");
                // Every covered pixel is in some triangle.
                let b = s.content_bounds();
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        if alpha(&s, x, y) <= ALPHA_MIN {
                            continue;
                        }
                        let p = [x as f64 + 0.5, y as f64 + 0.5];
                        let hit = m.tris.iter().any(|t| {
                            let [a, b, c] = t.map(|i| m.verts[i]);
                            let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
                            let l1 = ((p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1])) / det;
                            let l2 = ((b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1])) / det;
                            l1 >= -1e-9 && l2 >= -1e-9 && 1.0 - l1 - l2 >= -1e-9
                        });
                        assert!(hit, "{name} {d:?}: pixel {x},{y} outside the mesh");
                    }
                }
            }
        }
    }

    #[test]
    fn noisy_alpha_builds_a_mesh_quickly() {
        // Speckled coverage has a boundary everywhere: the outline path must give up (point
        // cap) or finish fast, and a mesh still comes out.
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        for y in 0..600 {
            for x in 0..600 {
                if (x * 7 + y * 13) % 5 < 2 || (x / 9 + y / 7) % 3 == 0 {
                    s.write_pixel(x, y, &[0.5, 0.5, 0.5, 1.0]);
                }
            }
        }
        let t = std::time::Instant::now();
        let m = build_mesh(&s, s.content_bounds(), PuppetDensity::More, 0.0);
        assert!(!m.tris.is_empty());
        assert!(t.elapsed().as_secs_f64() < 20.0, "{:?}", t.elapsed());
    }

    #[test]
    fn delaunay_handles_degenerate_input() {
        assert!(delaunay(&[]).is_none());
        assert!(delaunay(&[[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]]).is_none());
        assert!(delaunay(&[[f64::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]]).is_none());
        // Collinear points: no triangles (or a refusal), never a panic.
        let line: Vec<[f64; 2]> = (0..10).map(|i| [i as f64, 0.0]).collect();
        let _ = delaunay(&line);
        // A co-circular square grid triangulates fully: (n−1)² cells × 2.
        let grid: Vec<[f64; 2]> = (0..64).map(|i| [(i % 8) as f64, (i / 8) as f64]).collect();
        assert_eq!(delaunay(&grid).map(|t| t.len()), Some(98));
    }

    #[test]
    fn no_pins_or_unmoved_pins_are_identity() {
        for st in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let s = blob(st);
            let b = s.content_bounds();
            for pins in [
                vec![],
                vec![
                    PuppetPin { src: [28.0, 20.0], dst: [28.0, 20.0], rotate: None, depth: 0 },
                    PuppetPin { src: [60.0, 62.0], dst: [60.0, 62.0], rotate: None, depth: 0 },
                ],
            ] {
                let w = PuppetWarp { pins, mode: PuppetMode::Normal, density: PuppetDensity::Normal, expansion: 2.0 };
                let out = puppet_warp(&s, b, &w, Interp::Bicubic).unwrap();
                let worst = s.read_region(b).iter().zip(out.read_region(b)).map(|(a, c)| (a - c).abs()).fold(0.0f32, f32::max);
                assert!(worst <= 1.0 / 255.0 + 1e-6, "{st:?}: {worst}");
            }
        }
    }

    #[test]
    fn an_expansion_that_empties_the_mesh_warps_nothing() {
        // #951: with pins and no vertices, `locate` fell back to vertex 0 and the solver indexed
        // its empty union-find.
        let solver = PuppetSolver::new(PuppetMesh::default(), &[[10.0, 10.0]], &[true]);
        assert!(solver.mesh.verts.is_empty());
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let w = PuppetWarp {
            pins: vec![PuppetPin { src: [28.0, 30.0], dst: [38.0, 25.0], rotate: None, depth: 0 }],
            mode: PuppetMode::Normal,
            density: PuppetDensity::Normal,
            expansion: -200.0,
        };
        assert!(build_mesh(&s, b, w.density, w.expansion).tris.is_empty());
        // Nothing to warp: the pixels stay, rather than being dropped outside an empty mesh.
        let out = puppet_warp(&s, b, &w, Interp::Bilinear).unwrap();
        assert_eq!(out.read_region(b), s.read_region(b));
    }

    #[test]
    fn a_pin_dragged_absurdly_far_is_refused_before_rendering() {
        // #1019: the renderer sized its tile bookkeeping by the deformed mesh, so one far pin
        // allocated hundreds of MB before drawing a pixel.
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let pin = |dst: [f64; 2]| PuppetWarp {
            pins: vec![
                PuppetPin { src: [28.0, 20.0], dst: [28.0, 20.0], rotate: None, depth: 0 },
                PuppetPin { src: [60.0, 62.0], dst, rotate: None, depth: 0 },
            ],
            mode: PuppetMode::Normal,
            density: PuppetDensity::Normal,
            expansion: 2.0,
        };
        assert!(puppet_warp(&s, b, &pin([1048576.0, 1048576.0]), Interp::Bilinear).is_none());
        assert!(puppet_warp(&s, b, &pin([-1e7, 62.0]), Interp::Bilinear).is_none());
        let near = puppet_warp(&s, b, &pin([70.0, 66.0]), Interp::Bilinear).expect("a near pin warps");
        assert!(!near.content_bounds().is_empty());
    }

    #[test]
    fn a_single_pin_translates_rigidly() {
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let w = PuppetWarp {
            pins: vec![PuppetPin { src: [28.0, 30.0], dst: [38.0, 25.0], rotate: None, depth: 0 }],
            mode: PuppetMode::Rigid,
            density: PuppetDensity::Normal,
            expansion: 2.0,
        };
        let (solver, v, _) = deform(&s, b, &w, ITERATIONS);
        for (d, x) in v.iter().zip(&solver.mesh.verts) {
            assert!((d[0] - x[0] - 10.0).abs() < 1e-3 && (d[1] - x[1] + 5.0).abs() < 1e-3, "{d:?} {x:?}");
        }
        let out = puppet_warp(&s, b, &w, Interp::Bilinear).unwrap();
        assert_eq!(out.content_bounds(), b.translate(10, -5));
        assert_eq!(out.pixel(30, 20), s.pixel(20, 25));
        // A fixed rotation turns the whole mesh about the pin.
        let w = PuppetWarp { pins: vec![PuppetPin { src: [28.0, 30.0], dst: [28.0, 30.0], rotate: Some(90.0), depth: 0 }], ..w };
        let (solver, v, _) = deform(&s, b, &w, ITERATIONS);
        let i = (0..v.len()).max_by(|&a, &c| solver.mesh.verts[a][0].total_cmp(&solver.mesh.verts[c][0])).unwrap();
        let (x, d) = (solver.mesh.verts[i], v[i]);
        // (dx, dy) → (−dy, dx) for +90° with y down.
        let (dx, dy) = (x[0] - 28.0, x[1] - 30.0);
        assert!((d[0] - (28.0 - dy)).abs() < 1.0 && (d[1] - (30.0 + dx)).abs() < 1.0, "{x:?} → {d:?}");
    }

    #[test]
    fn arap_energy_decreases_and_modes_differ() {
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let mesh = build_mesh(&s, b, PuppetDensity::Normal, 2.0);
        let src = [[28.0, 14.0], [64.0, 62.0], [28.0, 62.0]];
        let dst = [[10.0, 30.0], [70.0, 50.0], [28.0, 62.0]];
        let solver = PuppetSolver::new(mesh, &src, &[false; 3]);
        let rot = [None; 3];
        let (v, e) = solver.solve(&dst, &rot, PuppetMode::Rigid, 30, None);
        assert!(e.len() == 30);
        let start = solver.energy(&solver.solve(&dst, &rot, PuppetMode::Rigid, 0, None).0, &dst, &rot);
        assert!(e[0] <= start * (1.0 + 1e-9), "{} vs {start}", e[0]);
        for w in e.windows(2) {
            assert!(w[1] <= w[0] * (1.0 + 1e-6) + 1e-9, "{w:?}");
        }
        assert!(e[29] < start);
        // Pins are honoured closely.
        for (b, d) in solver.bound.iter().zip(&dst) {
            let p = (0..3).fold([0.0, 0.0], |a, k| [a[0] + b.bary[k] * v[b.verts[k]][0], a[1] + b.bary[k] * v[b.verts[k]][1]]);
            assert!(dist2(p, *d) < 0.25, "{p:?} vs {d:?}");
        }
        let (vd, _) = solver.solve(&dst, &rot, PuppetMode::Distort, 30, None);
        assert_ne!(v, vd);
    }

    #[test]
    fn depth_orders_triangles() {
        let s = blob(SampleType::U8);
        let b = s.content_bounds();
        let mesh = build_mesh(&s, b, PuppetDensity::Fewer, 2.0);
        let pins = [[28.0, 14.0], [64.0, 62.0]];
        let solver = PuppetSolver::new(mesh, &pins, &[false; 2]);
        let order = solver.draw_order(&[1, 0], &pins);
        let near0 = |t: usize| {
            let c = solver.mesh.tris[t].iter().fold([0.0, 0.0], |a, &i| [a[0] + solver.mesh.verts[i][0] / 3.0, a[1] + solver.mesh.verts[i][1] / 3.0]);
            dist2(c, pins[0]) < dist2(c, pins[1])
        };
        // Triangles nearest the raised pin come last, all others first.
        let k = order.iter().position(|&t| near0(t)).unwrap();
        assert!(order[..k].iter().all(|&t| !near0(t)) && order[k..].iter().all(|&t| near0(t)));
    }
}
