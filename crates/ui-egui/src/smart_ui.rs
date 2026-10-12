//! Layers-panel pieces for smart objects: the thumbnail badge and the Smart Filters sub-rows
//! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters).

use egui::{Rect, Stroke, pos2, vec2};
use photocraft_doc::{Layer, LayerContent, SmartObject};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// Smart-object badge in the bottom-right corner of a layer thumbnail.
pub fn thumb_badge(ui: &egui::Ui, l: &Layer, thumb: Rect) {
    if !matches!(l.content, LayerContent::Smart(_)) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let r = Rect::from_min_size(pos2(thumb.right() - 9.0, thumb.bottom() - 9.0), vec2(11.0, 11.0));
    ui.painter().rect_filled(r, 2.0, t.field);
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    icons::paint(ui, r, "app-window", 9.0, t.icon);
}

/// The display name of a smart filter (its command's label without the ellipsis). A Photoshop
/// filter PhotoCraft doesn't implement shows its own name and is marked as kept as is.
pub(crate) fn filter_label(f: &photocraft_doc::SmartFilter) -> String {
    let command = f.command.as_str();
    if command == photocraft_engine::smart_cmds::UNSUPPORTED_FILTER {
        let name = f.params.get("name").and_then(Value::as_str).unwrap_or("Smart filter");
        return format!("{} (kept, not editable)", name.trim_end_matches("...").trim_end_matches('…'));
    }
    photocraft_engine::commands::find(command)
        .map_or_else(|| command.rsplit('.').next().unwrap_or(command).to_string(), |c| tl!(c.label).trim_end_matches('…').to_string())
}

/// Smart Filters header + one row per filter (top filter first, as in Photoshop) under a smart
/// layer. Edits are queued in `actions` as engine commands.
pub fn filter_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    crate::smart_filter_rows::rows(app, ui, l, depth, actions);
}

/// Re-opens a smart filter's dialog with its recorded parameters; OK runs
/// `layer.smartFilter.setParams` instead of adding another filter (see `dialogs::confirm`).
pub fn open_editor(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) {
    let _ = open_editor_dialog(app, ctx, layer, sm, index);
}

/// The dialog id for automation; Camera Raw retains its separate workspace editor.
pub(crate) fn open_editor_dialog(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) -> Option<u64> {
    let f = sm.smart_filters.get(index)?;
    if let Err(error) = authorize_filter_preview(app, photocraft_doc::LayerId(layer)) {
        app.ui.status = error;
        app.ui.status_error = true;
        return None;
    }
    // Camera Raw has its own full-window dialog rather than a generic parameter form.
    if f.command == photocraft_engine::lens_cmds::RAW {
        if let Err(e) = crate::camera_raw_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return None;
    }
    let adjustment = crate::smart_adjust_preview::supported(&f.command);
    let id = if crate::adjust_dialog::has_dialog(&f.command) {
        crate::adjust_dialog::open_for_smart_filter(app, &f.command)?
    } else {
        crate::filter_dialog::open_for_smart_filter(app, &f.command)?
    };
    let binding = app.session.active().map(|st| json!({"layer": layer, "index": index, "document": st.doc.id.0, "revision": st.revision}));
    if let Some(d) = app.ui.dialog_mut(id) {
        // Other filters still use their existing non-preview editor; their proxy semantics
        // (including opaque imported filters) have not been qualified here.
        if !adjustment {
            d.fields.remove("__preview");
        }
        if let Some(binding) = binding {
            d.fields.insert("__smartFilter".into(), binding);
        }
        // Adjustment editors store complete per-channel curves/levels, which are not all
        // present among their default fields. Never silently reset those channels on OK.
        if let Value::Object(p) = &f.params {
            for (k, v) in p {
                if !k.starts_with("__") && (adjustment || d.fields.contains_key(k)) {
                    d.fields.insert(k.clone(), v.clone());
                }
            }
        }
    }
    capture_dialog_preview_authority(app, id);
    Some(id)
}

pub(crate) fn effective_preview_authority(app: &PhotocraftApp) -> crate::state::PreviewAuthority {
    crate::state::PreviewAuthority(app.session.authorize.or_else(|| app.automation_input.then_some(app.services.automation_authorize).flatten()))
}

/// Once automation has touched a dialog, later preview frames retain its request authority.
/// A subsequent manual frame must not clear the captured callback.
pub(crate) fn capture_dialog_preview_authority(app: &mut PhotocraftApp, id: u64) {
    let authority = effective_preview_authority(app);
    if authority.0.is_some()
        && let Some(dialog) = app.ui.dialog_mut(id)
    {
        dialog.preview_authorize = authority;
    }
}

/// Keyboard and pointer automation opens dialogs after the control request has returned.
/// Install its effective gate only for this check, preserving an existing caller's authority.
fn with_preview_authority<T>(app: &mut PhotocraftApp, check: impl FnOnce(&PhotocraftApp) -> Result<T, String>) -> Result<T, String> {
    let previous = app.session.authorize;
    if previous.is_none() && app.automation_input {
        app.session.authorize = app.services.automation_authorize;
    }
    let result = check(app);
    app.session.authorize = previous;
    result
}

/// All engine drivers that append a Smart Filter and may therefore resolve its source.
pub(crate) fn creates_smart_filter(command: &str) -> bool {
    use photocraft_engine::{distort_cmds, lens_cmds};
    channel_filter(command)
        || matches!(
            command,
            distort_cmds::LIQUIFY
                | distort_cmds::PUPPET
                | distort_cmds::PUPPET_SMART
                | distort_cmds::PERSPECTIVE
                | distort_cmds::PERSPECTIVE_SMART
                | lens_cmds::LENS
                | lens_cmds::WIDE
                | lens_cmds::RAW
        )
}

/// Only these drivers divert mask/alpha targets before entering their Smart Object branch.
fn channel_filter(command: &str) -> bool {
    photocraft_engine::filters::params_for(command, &Value::Null).is_some()
        || crate::smart_adjust_preview::supported(command)
        || command == photocraft_engine::plugin_cmds::RUN
}

/// Re-editors can also be opened directly by synthetic native menu/keyboard input.
pub(crate) fn authorize_filter_preview(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<(), String> {
    with_preview_authority(app, |app| photocraft_engine::smart_cmds::check_filter_edit(&app.session, layer).map_err(|error| error.to_string()))
}

/// Shared native and control entry point for creation-dialog builders.
pub(crate) fn prepare_creation_dialog(app: &mut PhotocraftApp, command: &str) -> bool {
    match with_preview_authority(app, |app| authorize_creation_preview(app, command, &json!({}))) {
        Ok(()) => true,
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            false
        }
    }
}

/// A saved index must never address a different stack after an automated document edit/switch.
pub fn validate_editor(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    if let Some(binding) = fields.get("__smartCreation") {
        let valid = app.session.active().is_some_and(|st| {
            binding.get("document").and_then(Value::as_u64) == Some(st.doc.id.0)
                && binding.get("revision").and_then(Value::as_u64) == Some(st.revision)
                && binding.get("layer").and_then(Value::as_u64) == st.active_layer.map(|layer| layer.0)
                && binding.get("channel") == Some(&json!(st.channel_view.target))
                && binding.get("maskTarget").and_then(Value::as_bool) == Some(app.ui.mask_target)
                && binding.get("smartFilterMaskTarget") == Some(&json!(app.ui.smart_filter_mask_target))
        });
        if !valid {
            return Err("the document or target changed; reopen the adjustment dialog".into());
        }
    }
    let Some(binding) = fields.get("__smartFilter") else { return Ok(()) };
    let valid = app.session.active().is_some_and(|st| {
        binding.get("document").and_then(Value::as_u64) == Some(st.doc.id.0) && binding.get("revision").and_then(Value::as_u64) == Some(st.revision)
    });
    if valid { Ok(()) } else { Err("the document changed; reopen the Smart Filter to edit it".into()) }
}

/// A creation preview must keep the target whose source the opening request authorized.
/// Bind raster and channel dialogs too: selecting a Smart Object later must not acquire a
/// fresh linked-source read after the temporary request gate has been restored.
pub(crate) fn bind_creation_preview(app: &mut PhotocraftApp, command: &str, dialog: u64) {
    capture_dialog_preview_authority(app, dialog);
    if !creates_smart_filter(command) {
        return;
    }
    let active = app.session.active();
    let binding = json!({
        "document": active.map(|st| st.doc.id.0),
        "revision": active.map(|st| st.revision),
        "layer": active.and_then(|st| st.active_layer).map(|layer| layer.0),
        "channel": active.map(|st| st.channel_view.target),
        "maskTarget": app.ui.mask_target,
        "smartFilterMaskTarget": app.ui.smart_filter_mask_target,
    });
    if let Some(dialog) = app.ui.dialog_mut(dialog) {
        dialog.fields.insert("__smartCreation".into(), binding);
    }
}

/// Control requests temporarily install their authorization gate. Check the source while that
/// gate is present, before a creation dialog can schedule an independent preview session.
pub(crate) fn authorize_creation_preview(app: &PhotocraftApp, command: &str, params: &Value) -> Result<(), String> {
    if !creates_smart_filter(command) {
        return Ok(());
    }
    let params = photocraft_engine::channel_cmds::inject_target(&app.session, command, app.with_mask_target(command, params.clone()));
    if photocraft_engine::smart_mask_cmds::is_target(&params) {
        return Err("Smart Filter masks currently support Brush, Pencil and Eraser".into());
    }
    let channel_target = match params.get("target") {
        Some(Value::String(target)) => matches!(target.as_str(), "mask" | "quickMask"),
        Some(Value::Object(target)) => target.get("channel").and_then(Value::as_u64).is_some(),
        _ => false,
    };
    if channel_filter(command) && channel_target {
        return Ok(());
    }
    let Some(st) = app.session.active() else { return Ok(()) };
    // All Smart Filter drivers except these adjustments accept an explicit layer. Match
    // their routing before checking the source; adjustments use the active layer instead.
    let explicit_layer =
        (!crate::smart_adjust_preview::supported(command)).then(|| params.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId)).flatten();
    let Some(layer) = explicit_layer.or(st.active_layer).and_then(|id| st.doc.layer(id)) else { return Ok(()) };
    if !matches!(layer.content, LayerContent::Smart(_)) {
        return Ok(());
    }
    if crate::smart_adjust_preview::supported(command) && matches!(st.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_)) {
        return Err("select the composite color channel before adding a Smart Filter".into());
    }
    photocraft_engine::smart_cmds::check_filter_edit(&app.session, layer.id).map_err(|error| error.to_string())
}

/// User fields such as the channel target remain editable over control. Validate the
/// prospective target under the request gate before replacing any live fields.
pub(crate) fn authorize_dialog_preview(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    validate_editor(app, fields)?;
    let Some(command) = fields.get("__command").and_then(Value::as_str) else { return Ok(()) };
    let (command, params) = confirm_command(fields, command.to_string(), crate::filter_dialog::params_of(fields));
    if let Some(authorize) = app.session.authorize {
        authorize(&command, &params).map_err(|error| error.to_string())?;
    }
    if matches!(command.as_str(), "layer.smartFilter.setParams" | crate::smart_filter_rows::BLENDING) {
        let layer = match params.get("layer") {
            Some(value) => value.as_u64().map(photocraft_doc::LayerId),
            None => app.session.active().and_then(|st| st.active_layer),
        }
        .ok_or("need a valid Smart Object layer")?;
        photocraft_engine::smart_cmds::check_filter_edit(&app.session, layer).map_err(|error| error.to_string())
    } else {
        authorize_creation_preview(app, &command, &params)
    }
}

/// The command a confirmed filter dialog runs: smart-filter edits update the recorded filter.
pub fn confirm_command(fields: &serde_json::Map<String, Value>, command: String, params: Value) -> (String, Value) {
    match fields.get("__smartFilter") {
        Some(sf) if command == crate::smart_filter_rows::BLENDING => {
            let mut params = params.as_object().cloned().unwrap_or_default();
            params.insert("layer".into(), sf["layer"].clone());
            params.insert("index".into(), sf["index"].clone());
            (command, Value::Object(params))
        }
        Some(sf) => ("layer.smartFilter.setParams".into(), json!({"layer": sf["layer"], "index": sf["index"], "params": params})),
        None => (command, params),
    }
}
