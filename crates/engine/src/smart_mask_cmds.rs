//! Painting a Smart Filter mask changes the mask and its cached appearance together. Only
//! Brush, Pencil and Eraser have this prepared edit path; other tools must refuse this target.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, LayerContent, LayerId, LayerMask, SmartObject};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::Value;
use std::sync::Arc;

use crate::{EngineError, Result, Session};

pub const TARGET: &str = "smartFilterMask";

pub fn is_target(params: &Value) -> bool {
    params.get("target").and_then(Value::as_str) == Some(TARGET)
}

pub fn supports(command: &str) -> bool {
    matches!(command, "paint.stroke" | "paint.pencil")
}

fn other(message: &str) -> EngineError {
    EngineError::Other(message.into())
}

fn smart(doc: &Document, layer: LayerId) -> Result<&SmartObject> {
    match &doc.layer(layer).ok_or(EngineError::NoLayer(layer))?.content {
        LayerContent::Smart(smart) if !smart.smart_filters.is_empty() => Ok(smart),
        _ => Err(other("select a Smart Object with Smart Filters to paint its filter mask")),
    }
}

fn layer_param(session: &Session, params: &Value) -> Result<LayerId> {
    if let Some(value) = params.get("layer") {
        return value.as_u64().map(LayerId).ok_or_else(|| other("the Smart Filter mask layer must be an unsigned layer id"));
    }
    session.active().and_then(|state| state.active_layer).ok_or_else(|| other("no active layer"))
}

/// The parameter-aware command precondition. This only inspects document state; selecting a
/// thumbnail or checking menu enablement never opens the Smart Object's source.
pub(crate) fn enabled(session: &Session, command: &str, params: &Value) -> Option<std::result::Result<(), String>> {
    if !is_target(params) {
        return None;
    }
    Some(
        (|| {
            if !supports(command) {
                return Err(other("Smart Filter masks currently support Brush, Pencil and Eraser"));
            }
            let layer = layer_param(session, params)?;
            let state = session.active().ok_or(EngineError::NoDocument)?;
            crate::commands::check_pixels_unlocked(&state.doc, layer)?;
            let smart = smart(&state.doc, layer)?;
            if let Some(message) = crate::hidden_target::refusal(session, command, params) {
                return Err(other(message));
            }
            crate::smart_cmds::check_renderable_filters(smart)?;
            // The existing Smart Filter renderer does not evaluate mask feather. Refuse it rather
            // than silently baking different pixels or doing an unbounded blur on every dab.
            if smart.filter_mask.as_ref().is_some_and(|mask| mask.feather != 0.0) {
                return Err(other("painting feathered Smart Filter masks is not supported yet"));
            }
            Ok(())
        })()
        .map_err(|error: EngineError| error.to_string()),
    )
}

/// Source and filter results authorized and rendered once, before a stroke becomes visible.
/// Subsequent dabs read only these surfaces and update cache tiles intersecting their damage.
pub(crate) struct Prepared {
    layer: LayerId,
    base: Surface,
    filtered: Surface,
    initial_cache: Surface,
    previous_mask: Surface,
    rebase: bool,
    initial_damage: Rect,
    bounds: Rect,
}

impl Prepared {
    pub(crate) fn begin(session: &Session, command: &str, params: &Value, authorize: Option<fn(&str, &Value) -> Result<()>>) -> Result<Option<Self>> {
        let Some(enabled) = enabled(session, command, params) else { return Ok(None) };
        enabled.map_err(EngineError::Other)?;
        let authorize = session.authorize.or(authorize);
        if let Some(gate) = authorize {
            gate(command, params)?;
        }
        let layer = layer_param(session, params)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        let smart = smart(&state.doc, layer)?;
        let (base, filtered) = crate::smart_cmds::prepare_mask_pixels(session, layer, authorize)?;
        let bounds = base.content_bounds().union(&filtered.content_bounds());
        let initial_cache = match &smart.filter_mask {
            Some(mask) if mask.enabled => crate::smart_cmds::mask_mix(&base, &filtered, mask),
            _ => filtered.clone(),
        };
        // An imported cache may reflect another renderer, or a linked source may have changed.
        // Rebase once before painting so the result never mixes two source/render versions.
        // Exact caches keep their existing COW tiles, including every untouched tile.
        let rebase = smart.cache.as_ref() != Some(&initial_cache);
        let initial_damage =
            if rebase { state.doc.bounds().union(&bounds).union(&smart.cache.as_ref().map_or(Rect::EMPTY, Surface::content_bounds)) } else { Rect::EMPTY };
        let previous_mask = smart
            .filter_mask
            .as_ref()
            .map(|mask| mask.surface.clone())
            .unwrap_or_else(|| Surface::with_default(PixelFormat::new(ColorMode::Grayscale, state.doc.pixel_format().sample, false), &[1.0]));
        Ok(Some(Self { layer, base, filtered, initial_cache, previous_mask, rebase, initial_damage, bounds }))
    }

    /// The only mutable-surface adapter for this target. It runs after preparation, on either
    /// a transaction's document copy or the live stroke's private copy.
    pub(crate) fn surface<'a>(&self, doc: &'a mut Document) -> Result<(&'a mut Surface, bool)> {
        crate::commands::check_pixels_unlocked(doc, self.layer)?;
        let format = PixelFormat::new(ColorMode::Grayscale, doc.pixel_format().sample, false);
        let layer = doc.layer_mut(self.layer).ok_or(EngineError::NoLayer(self.layer))?;
        let LayerContent::Smart(smart) = &mut layer.content else { return Err(other("the mask target is no longer a Smart Object")) };
        let mask = smart.filter_mask.get_or_insert_with(|| LayerMask { surface: Surface::with_default(format, &[1.0]), ..LayerMask::reveal_all() });
        Ok((&mut mask.surface, false))
    }

    pub(crate) fn initial_damage(&self) -> Rect {
        self.initial_damage
    }

    pub(crate) fn refresh(&mut self, doc: &mut Document, damage: Rect) -> Result<Rect> {
        let layer = doc.layer_mut(self.layer).ok_or(EngineError::NoLayer(self.layer))?;
        let LayerContent::Smart(smart) = &mut layer.content else { return Err(other("the mask target is no longer a Smart Object")) };
        let rebase = std::mem::take(&mut self.rebase);
        if rebase {
            smart.cache = Some(self.initial_cache.clone());
        }
        let cache = smart.cache.get_or_insert_with(|| self.initial_cache.clone());
        if let Some(mask) = smart.filter_mask.as_ref().filter(|mask| mask.enabled) {
            let area = damage.intersect(&self.bounds);
            for coordinate in area.tiles() {
                let current = mask.surface.tile(coordinate);
                let previous = self.previous_mask.tile(coordinate);
                let unchanged = match (current, previous) {
                    (Some(current), Some(previous)) => Arc::ptr_eq(current, previous),
                    (None, None) => true,
                    _ => false,
                };
                if unchanged {
                    continue;
                }
                crate::smart_cmds::mask_mix_region(&self.base, &self.filtered, mask, cache, coordinate.rect().intersect(&area));
                if let Some(tile) = current {
                    self.previous_mask.put_tiles([(coordinate, tile.clone())]);
                }
            }
        }
        Ok(if rebase { damage.union(&self.initial_damage) } else { damage })
    }
}

pub(crate) fn stroke_surface<'a>(
    prepared: Option<&Prepared>,
    doc: &'a mut Document,
    layer: Option<LayerId>,
    params: &Value,
) -> Result<(&'a mut Surface, bool)> {
    match prepared {
        Some(prepared) => prepared.surface(doc),
        None => crate::channel_cmds::target_surface(doc, layer, params),
    }
}
