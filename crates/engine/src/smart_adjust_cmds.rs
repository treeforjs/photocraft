//! Image adjustments whose parameters can be retained and edited on a Smart Object.
//! Other adjustment commands keep their existing pixel/channel behavior.

use photocraft_doc::{ColorMode, LayerContent, SmartFilter};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

pub(crate) fn supported(kind: &str) -> bool {
    matches!(kind, "levels" | "curves" | "hueSaturation" | "shadowsHighlights")
}

/// Reject malformed settings before history or the rendered cache changes. Store normalized
/// values so out-of-range input renders identically after save/reopen.
pub(crate) fn parameters(kind: &str, params: &Value, mode: ColorMode) -> Result<Value> {
    if kind == "shadowsHighlights" {
        return crate::adjust_cmds::checked_shadows_highlights_params(params);
    }
    let adj = crate::adjust_params::from_params(kind, params, None, mode)?;
    Ok(crate::adjust_params::to_params(&adj))
}

/// Channel/range records are edited one control at a time. Replacing a whole record with
/// that patch would silently reset its other controls. Merge one known record level only;
/// arrays and explicit nulls replace the old value and keep the parser's reset semantics.
pub(crate) fn merge_parameters(kind: &str, stored: &Value, patch: &Value, mode: ColorMode) -> Result<Value> {
    let fields = patch
        .as_object()
        .ok_or_else(|| EngineError::BadParams { cmd: "layer.smartFilter.setParams".into(), msg: "adjustment parameters must be an object".into() })?;
    let records: &[&str] = match kind {
        "hueSaturation" => &crate::adjust_params::HUE_RANGES,
        "levels" => &["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"],
        _ => &[],
    };
    let mut merged = stored.as_object().cloned().unwrap_or_default();
    for (key, value) in fields {
        if records.contains(&key.as_str())
            && let Some(old) = merged.get_mut(key).and_then(Value::as_object_mut)
            && let Some(new) = value.as_object()
        {
            old.extend(new.clone());
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    parameters(kind, &Value::Object(merged), mode)
}

pub(crate) fn enabled(s: &Session) -> std::result::Result<(), String> {
    if let Some(st) = s.active()
        && matches!(st.channel_view.target, crate::channel_cmds::ChannelTarget::Color(_))
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|layer| matches!(layer.content, LayerContent::Smart(_)))
    {
        return Err("select the composite color channel before adding a Smart Filter".into());
    }
    crate::filters::has_filterable_layer(s)
}

/// Return None for raster/channel/mask edits; append one nondestructive filter otherwise.
pub(crate) fn apply(s: &mut Session, kind: &str, params: &Value, label: &str) -> Option<Result<Value>> {
    if !supported(kind) || crate::channel_cmds::target_of(params) != crate::channel_cmds::Target::Pixels {
        return None;
    }
    let st = s.active()?;
    let id = st.active_layer?;
    let LayerContent::Smart(sm) = &st.doc.layer(id)?.content else { return None };
    let command = format!("image.adjustments.{kind}");
    if matches!(st.channel_view.target, crate::channel_cmds::ChannelTarget::Color(_)) {
        return Some(Err(EngineError::Other("select the composite color channel before adding a Smart Filter".into())));
    }
    if let Err(error) = crate::smart_cmds::check_renderable_filters(sm) {
        return Some(Err(error));
    }
    if let Err(error) = crate::smart_cmds::authorize_source(s, &command, &st.doc, &sm.source) {
        return Some(Err(error));
    }
    // An unavailable source cannot be edited repeatedly without baking the old filter result.
    if crate::smart_cmds::source_bytes(&st.doc.metadata, &sm.source).is_none() {
        return Some(Err(EngineError::Other("the Smart Object source is unavailable; relink it before editing its filters".into())));
    }
    let normalized = match parameters(kind, params, st.doc.mode) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let authorize = s.authorize;
    Some(s.edit(label, |doc, _| {
        crate::commands::check_pixels_unlocked(doc, id)?;
        let selection = doc.selection.clone();
        let filter = SmartFilter { command, params: normalized, blend: photocraft_doc::BlendMode::Normal, opacity: 1.0, visible: true };
        crate::smart_cmds::add_smart_filter(doc, id, filter, selection.as_ref(), authorize)?;
        Ok(json!({"layer": id.0}))
    }))
}
