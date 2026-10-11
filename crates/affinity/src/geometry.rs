//! Outlines of curves (`PCvD` node lists), parametric shapes (`Shpe` objects) and compounds, in
//! node space.

use std::collections::HashSet;

use crate::model::{Affine, Path, Point, Reader, SubPath};
use crate::stream::{ObjId, Tag, Value};

/// Nodes per curve object.
const MAX_NODES: usize = 4_000_000;
/// Compound operands are shared objects and may form a cyclic or branching graph.
const MAX_COMPOUND_DEPTH: usize = 128;
const MAX_OUTLINE_NODES: usize = 500_000;
/// Cubic segments and subpath starts materialized across all operands of one outline.
const MAX_OUTLINE_GEOMETRY: usize = 4_000_000;
const GEOMETRY_LIMIT_WARNING: &str = "outlines with more than four million curve segments and subpaths";

struct OutlineBudget {
    active: HashSet<ObjId>,
    remaining: usize,
    geometry_remaining: usize,
}

impl OutlineBudget {
    fn admit_geometry(&mut self, path: &Path) -> bool {
        let remaining =
            path.subpaths.iter().try_fold(self.geometry_remaining, |remaining, subpath| remaining.checked_sub(1)?.checked_sub(subpath.segments.len()));
        match remaining {
            Some(remaining) => {
                self.geometry_remaining = remaining;
                true
            }
            None => {
                // Stop expanding this outline after the first operand that cannot fit.
                self.geometry_remaining = 0;
                false
            }
        }
    }
}

/// The node-space outline of a curve, shape or compound node, and whether it fills even-odd.
/// Instances (master-page or symbol copies) take the geometry of their master.
pub(crate) fn outline(r: &mut Reader, id: ObjId, class: Tag, world: Affine) -> Option<(Path, bool)> {
    let mut budget = OutlineBudget { active: HashSet::new(), remaining: MAX_OUTLINE_NODES, geometry_remaining: MAX_OUTLINE_GEOMETRY };
    outline_bounded(r, id, class, world, &mut budget)
}

fn outline_bounded(r: &mut Reader, id: ObjId, class: Tag, world: Affine, budget: &mut OutlineBudget) -> Option<(Path, bool)> {
    if budget.active.len() >= MAX_COMPOUND_DEPTH {
        r.warn("compound shapes nested deeper than 128 levels");
        return None;
    }
    if budget.remaining == 0 {
        r.warn("compound shapes with more than 500000 operands");
        return None;
    }
    if !budget.active.insert(id) {
        r.warn("a compound shape that contains itself");
        return None;
    }
    budget.remaining -= 1;
    let result = outline_inner(r, id, class, world, budget);
    budget.active.remove(&id);
    // Charge each emitted leaf once. Charging compounds too would count the same
    // geometry again at each ancestor, while deduplicating object IDs would miss copies.
    if class != Tag::of(b"Comp")
        && let Some((path, _)) = &result
        && !budget.admit_geometry(path)
    {
        r.warn(GEOMETRY_LIMIT_WARNING);
        return None;
    }
    result
}

fn outline_inner(r: &mut Reader, id: ObjId, class: Tag, world: Affine, budget: &mut OutlineBudget) -> Option<(Path, bool)> {
    let s = r.s;
    match &class.0.to_be_bytes() {
        b"PCrv" | b"TxtC" => {
            let crvs = s.obj(id, b"Crvs").or_else(|| s.obj(id, b"MCrM").and_then(|m| s.obj(m, b"Crvs")))?;
            let path = curves(r, crvs)?;
            let multi = path.subpaths.len() > 1;
            Some((path, multi))
        }
        b"Comp" => {
            // One path of all the operands' contours in the compound's space, filled even-odd: this
            // matches add and subtract operands in every public sample; a true union of overlapping
            // added operands would differ.
            let mut path = Path::default();
            for c in s.objs(id, b"Chld") {
                if budget.remaining == 0 {
                    r.warn("compound shapes with more than 500000 operands");
                    break;
                }
                if budget.geometry_remaining == 0 {
                    r.warn(GEOMETRY_LIMIT_WARNING);
                    break;
                }
                let local = s.floats::<6>(c, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY);
                let class = s.class(c)?;
                if s.enumeration(c, b"ComO").is_some_and(|(op, _)| op != 0 && op != 2) {
                    r.warn("compound shapes that intersect or exclude (imported as add)");
                }
                if let Some((p, _)) = outline_bounded(r, c, class, local.then(world), budget) {
                    path.subpaths.extend(p.transformed(local).subpaths);
                }
            }
            Some((path, true))
        }
        _ => {
            let (shape, box_owner) = match s.obj(id, b"Shpe") {
                Some(sh) => (sh, id),
                None => {
                    let master = s.obj(id, b"ShpM")?;
                    (s.obj(master, b"Shpe")?, master)
                }
            };
            let b = s.floats::<4>(box_owner, b"ShpB")?;
            crate::shapes::shape(r, shape, b, world)
        }
    }
}

/// `PCvD.Data`: positional `[u8, u32 count, then per subpath: bool closed, 18-byte records]`.
/// A record is (f64 x, f64 y, u8 kind, u8 role): role 0 is an anchor, 1 the out-handle of the
/// anchor before it, 2 the in-handle of the anchor after it. A closed subpath repeats its first
/// anchor at the end. `CnrD` live corners are rounded on the stored polyline.
pub(crate) fn curves(r: &mut Reader, crvs: ObjId) -> Option<Path> {
    let s = r.s;
    let data = s.obj(crvs, b"Data")?;
    let mut fields = s.positional(data);
    fields.next()?;
    let count = match fields.next()? {
        Value::UInt(n) => usize::try_from(*n).ok()?,
        _ => return None,
    };
    let corners = s.obj(crvs, b"CnrD").and_then(|c| corner_radii(s.positional(c).collect()));
    let mut path = Path::default();
    let mut seen = 0usize;
    for index in 0..count {
        let closed = match fields.next()? {
            Value::Bool(b) => *b,
            _ => return None,
        };
        let Value::Array(records) = fields.next()? else { return None };
        seen = seen.checked_add(records.len())?;
        if seen > MAX_NODES {
            r.warn("curves with more than four million nodes");
            return None;
        }
        let anchors = anchors(records)?;
        let radii = if index == 0 { corners.as_ref() } else { None };
        if let Some(sp) = subpath(&anchors, closed, radii) {
            path.subpaths.push(sp);
        }
    }
    if corners.is_some() && count > 1 {
        r.warn("rounded corners on curves with several subpaths (only the first is rounded)");
    }
    Some(path)
}

struct Anchor {
    p: Point,
    inh: Option<Point>,
    out: Option<Point>,
    /// Index of the anchor's record (live corners refer to records).
    record: usize,
}

fn record(v: &Value) -> Option<(Point, u8)> {
    let Value::Bytes(b) = v else { return None };
    if b.len() != 18 {
        return None;
    }
    let x = f64::from_le_bytes(b.get(0..8)?.try_into().ok()?);
    let y = f64::from_le_bytes(b.get(8..16)?.try_into().ok()?);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some((Point { x, y }, *b.get(17)?))
}

fn anchors(records: &[Value]) -> Option<Vec<Anchor>> {
    let mut out: Vec<Anchor> = Vec::with_capacity(records.len() / 3 + 1);
    let mut pending_in = None;
    for (i, v) in records.iter().enumerate() {
        let (p, role) = record(v)?;
        match role {
            0 => {
                out.push(Anchor { p, inh: pending_in.take(), out: None, record: i });
            }
            1 => {
                // A leading out-handle without an anchor is edit state only.
                if let Some(a) = out.last_mut() {
                    a.out = Some(p);
                }
            }
            2 => pending_in = Some(p),
            _ => return None,
        }
    }
    Some(out)
}

fn subpath(anchors: &[Anchor], closed: bool, radii: Option<&Vec<(usize, f64)>>) -> Option<SubPath> {
    let first = anchors.first()?;
    if let Some(radii) = radii.filter(|r| !r.is_empty()) {
        let corner = |a: &Anchor| radii.iter().filter(|(rec, _)| *rec == a.record).map(|(_, r)| *r).fold(0.0, f64::max);
        let polyline = anchors.iter().all(|a| a.inh.is_none_or(|h| h == a.p) && a.out.is_none_or(|h| h == a.p));
        if polyline {
            let mut pts: Vec<(Point, f64)> = anchors.iter().map(|a| (a.p, corner(a))).collect();
            // A closed subpath repeats its first anchor; its radius may be on either copy.
            if closed
                && pts.len() > 1
                && pts.first().map(|p| p.0) == pts.last().map(|p| p.0)
                && let Some((_, r)) = pts.pop()
                && let Some(f) = pts.first_mut()
            {
                f.1 = f.1.max(r);
            }
            return Some(crate::shapes::fillet(&pts, closed));
        }
    }
    let segments = anchors
        .windows(2)
        .filter_map(|w| {
            let [a, b] = w else { return None };
            Some([a.out.unwrap_or(a.p), b.inh.unwrap_or(b.p), b.p])
        })
        .collect();
    Some(SubPath { start: first.p, segments, closed })
}

/// `CnrD`: positional `[u8, i32[] record indices, f64[] radii, u8[] types]`.
fn corner_radii(fields: Vec<&Value>) -> Option<Vec<(usize, f64)>> {
    let (Some(Value::Array(idx)), Some(Value::Array(rad))) = (fields.get(1), fields.get(2)) else { return None };
    Some(
        idx.iter()
            .zip(rad)
            .filter_map(|(i, r)| match (i, r) {
                (Value::Int(i), Value::Float(r)) if r.is_finite() && *r > 0.0 => Some((usize::try_from(*i).ok()?, *r)),
                _ => None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{self, F, Method, tag};
    use crate::{Archive, Limits, stream};

    fn line() -> F {
        let records = [0.0f64, 1.0]
            .into_iter()
            .map(|x| {
                let mut record = x.to_le_bytes().to_vec();
                record.extend(0.0f64.to_le_bytes());
                record.extend([0, 0]);
                record
            })
            .collect();
        let data = F::Pos(vec![F::U8(0), F::U32(1), F::Bool(false), F::Records(18, records)]);
        F::Def(1, vec![tag(b"PCrv")], vec![(tag(b"Crvs"), F::Obj(tag(b"PCvD"), vec![(tag(b"Data"), data)]))])
    }

    fn repeated_outline(operand: F, limit: usize) -> Path {
        let inner = F::Def(2, vec![tag(b"Comp")], vec![(tag(b"Chld"), F::Shared(vec![operand, F::Ref(1)]))]);
        let outer = F::Def(3, vec![tag(b"Comp")], vec![(tag(b"Chld"), F::Shared(vec![inner, F::Ref(2)]))]);
        let bytes = synth::stream(&[(tag(b"Node"), outer)]);
        let file = synth::container(&[("doc.dat", &bytes, Method::Stored)], None);
        let mut archive = Archive::open(&file, Limits::default()).unwrap();
        let stream = stream::parse(&bytes).unwrap();
        let id = stream.obj(stream.root, b"Node").unwrap();
        let mut reader = Reader::new(&stream, &mut archive);
        let mut budget = OutlineBudget { active: HashSet::new(), remaining: MAX_OUTLINE_NODES, geometry_remaining: limit };
        outline_bounded(&mut reader, id, tag(b"Comp"), Affine::IDENTITY, &mut budget).unwrap().0
    }

    #[test]
    fn shared_curve_geometry_is_charged_per_emitted_copy() {
        // Four references to the same line, each one start plus one segment. Compound
        // ancestors must not charge their children's geometry again as it is propagated.
        for limit in 0..=9 {
            let path = repeated_outline(line(), limit);
            assert_eq!(path.subpaths.len(), (limit / 2).min(4), "budget {limit}");
            assert!(path.subpaths.iter().all(|p| p.segments.len() == 1));
        }
    }

    #[test]
    fn parametric_geometry_uses_the_same_aggregate_budget() {
        let rectangle =
            F::Def(1, vec![tag(b"ShpN")], vec![(tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 10.0, 10.0])), (tag(b"Shpe"), F::Obj(tag(b"ShNR"), Vec::new()))]);
        // One start and three cubic segments per rectangle. The third copy exceeds the
        // budget; the first two complete operands are retained.
        let path = repeated_outline(rectangle, 9);
        assert_eq!(path.subpaths.len(), 2);
        assert!(path.subpaths.iter().all(|p| p.segments.len() == 3));
    }
}
