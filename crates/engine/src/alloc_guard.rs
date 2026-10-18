//! Memory preflight for the commands that write whole layers or documents (fill, clear, cut,
//! copy, paste, transform, resize, crop, rotate, save and export).
//!
//! Before such a command runs, [`preflight`] estimates the memory it will allocate and asks the
//! allocator for that much up front with `try_reserve`, freeing it straight away. When the
//! allocator refuses, or the estimate exceeds [`Session::alloc_limit`], the command fails with a
//! plain "not enough memory" error before it touches anything, so the document stays as it was.
//! The limit doubles as the injection point for tests and for embedders that want to cap one
//! command's footprint.
//!
//! Surfaces are sparse tiles shared between history states, so the estimate counts what is
//! really allocated: the tiles a command reads (each costs a stored copy for the result plus an
//! `f32` working copy) or, for fills and saves, the area it writes. Duplicating shares tiles and
//! costs nothing, so it isn't guarded.

use photocraft_doc::{Document, Layer};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::Surface;
use serde_json::Value;

use crate::{EngineError, Result, Session};

const TILE_PX: u64 = (TILE_SIZE as u64) * (TILE_SIZE as u64);

fn tiles(s: &Surface) -> u64 {
    s.tile_count() as u64
}

fn layer_px(l: &Layer) -> u64 {
    let t = l.surface().map_or(0, tiles).saturating_add(l.mask.as_ref().map_or(0, |m| tiles(&m.surface)));
    t.saturating_mul(TILE_PX)
}

fn doc_px(d: &Document) -> u64 {
    d.walk().iter().fold(0u64, |a, (_, _, l)| a.saturating_add(layer_px(l)))
}

fn canvas_px(d: &Document) -> u64 {
    u64::from(d.size.width).saturating_mul(u64::from(d.size.height))
}

/// Pixels a fill, clear or copy writes: the selection's tiles, or the canvas without one.
fn selected_px(d: &Document) -> u64 {
    let full = canvas_px(d);
    match &d.selection {
        Some(sel) if sel.default_pixel().iter().all(|v| *v == 0.0) => tiles(sel).saturating_mul(TILE_PX).min(full),
        _ => full,
    }
}

fn requested(p: &Value, key: &str) -> Option<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0)
}

/// Bytes `id` is expected to allocate on the active document, or `None` when it isn't guarded.
pub fn estimate(s: &Session, id: &str, p: &Value) -> Option<u64> {
    let d = s.active().map(|st| &st.doc);
    let fmt = d.map_or(photocraft_color::PixelFormat::RGBA8, |d| d.pixel_format());
    // One stored copy plus one f32 working copy per pixel.
    let cost = (fmt.bytes_per_pixel() as u64).saturating_add(fmt.channels() as u64 * 4);
    let active = || {
        let st = s.active()?;
        st.active_layer.and_then(|id| st.doc.layer(id))
    };
    let px = match id {
        // Content-Aware Fill budgets its own search.
        "edit.fill" if p.get("contents").and_then(Value::as_str) == Some("contentAware") => return None,
        "edit.fill" | "edit.fillForeground" | "edit.fillBackground" | "edit.clear" | "edit.cut" | "edit.copy" | "edit.copyMerged" => selected_px(d?),
        i if i.starts_with("edit.paste") => s.clipboard.as_ref().map_or(0, |c| tiles(&c.surface).saturating_mul(TILE_PX)),
        i if i.starts_with("edit.transform") => active().map_or(0, layer_px).saturating_mul(2),
        i if i.starts_with("image.imageRotation.") || i == "image.rotation.arbitrary" => doc_px(d?).saturating_mul(2),
        "image.crop" | "image.trim" | "image.canvasSize" => doc_px(d?),
        "image.imageSize" => {
            let d = d?;
            if p.get("resample").and_then(Value::as_str) == Some("none") {
                return None;
            }
            let (ow, oh) = (f64::from(d.size.width.max(1)), f64::from(d.size.height.max(1)));
            let (w, h) = match (requested(p, "width"), requested(p, "height")) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, w * oh / ow),
                (None, Some(h)) => (h * ow / oh, h),
                (None, None) => (ow, oh),
            };
            let new = (w.ceil() * h.ceil()).min(u64::MAX as f64) as u64;
            // A raster past Image Size's own budget is refused there, with its own advice.
            if new.saturating_mul(fmt.bytes_per_pixel() as u64) > crate::image_cmds::MAX_RESAMPLE_BYTES {
                return None;
            }
            let ratio = (new as f64 / (ow * oh)).max(1.0);
            doc_px(d).saturating_add((doc_px(d) as f64 * ratio).min(u64::MAX as f64) as u64)
        }
        // Saving flattens the whole canvas.
        "file.saveACopy" => canvas_px(d?),
        i if i.starts_with("file.export.") => canvas_px(d?),
        _ => return None,
    };
    Some(px.saturating_mul(cost))
}

fn megabytes(bytes: u64) -> u64 {
    bytes.div_ceil(1 << 20)
}

/// Fails `id` with a "not enough memory" error when its working memory isn't available.
pub(crate) fn preflight(s: &Session, id: &str, p: &Value) -> Result<()> {
    let Some(need) = estimate(s, id, p) else { return Ok(()) };
    let refuse = || {
        EngineError::Other(format!(
            "not enough memory for `{id}`: it needs about {} MB; close other documents or apps, or reduce the image size, and try again (nothing was changed)",
            megabytes(need)
        ))
    };
    if s.alloc_limit.is_some_and(|limit| need > limit) {
        return Err(refuse());
    }
    // Small jobs always fit; probing them would only cost time.
    if need < (64 << 20) {
        return Ok(());
    }
    let n = usize::try_from(need).map_err(|_| refuse())?;
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(n).map_err(|_| refuse())?;
    Ok(())
}

#[cfg(test)]
#[path = "alloc_guard_tests.rs"]
mod tests;
