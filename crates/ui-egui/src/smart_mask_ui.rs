//! The Smart Filters header's mask thumbnail and its document/layer-bound painting target.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2};
use photocraft_doc::{DocId, LayerContent, LayerId, SmartObject};
use photocraft_engine::channel_cmds::ChannelTarget;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub(crate) const THUMB: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub document: DocId,
    pub layer: LayerId,
}

/// A stroke keeps the target and authority it began with until release. Switching document,
/// layer, channel or mask during a gesture cancels it instead of painting a different surface.
#[derive(Clone, Debug)]
pub(crate) struct StrokeBinding {
    document: DocId,
    revision: u64,
    layer: Option<LayerId>,
    channel: ChannelTarget,
    target: Value,
    pub authority: crate::state::PreviewAuthority,
}

impl StrokeBinding {
    pub(crate) fn capture(app: &PhotocraftApp) -> Option<Self> {
        let state = app.session.active()?;
        Some(Self {
            document: state.doc.id,
            revision: state.revision,
            layer: state.active_layer,
            channel: state.channel_view.target,
            target: crate::canvas::paint_target(app),
            authority: crate::smart_ui::effective_preview_authority(app),
        })
    }

    pub(crate) fn valid(&self, app: &PhotocraftApp) -> bool {
        app.session.active().is_some_and(|state| {
            state.doc.id == self.document
                && state.revision == self.revision
                && state.active_layer == self.layer
                && state.channel_view.target == self.channel
                && crate::canvas::paint_target(app) == self.target
        })
    }

    pub(crate) fn apply(&self, params: &mut Value) {
        params["target"] = self.target.clone();
        if let Some(layer) = self.layer {
            params["layer"] = json!(layer.0);
        }
    }

    pub(crate) fn is_smart_mask(&self) -> bool {
        self.target.as_str() == Some(photocraft_engine::smart_mask_cmds::TARGET)
    }
}

pub(crate) fn cancel_stroke(app: &mut PhotocraftApp) -> bool {
    if !app.drag.as_ref().and_then(|drag| drag.paint_binding.as_ref()).is_some_and(StrokeBinding::is_smart_mask) {
        return false;
    }
    app.drag = None;
    app.live_stroke = None;
    true
}

/// Pure validation for `ui.set`: malformed requests fail before changing any UI fields.
pub(crate) fn requested(app: &PhotocraftApp, enabled: bool) -> Result<Option<Target>, String> {
    if !enabled {
        return Ok(None);
    }
    let state = app.session.active().ok_or("no document open")?;
    let layer = state.active_layer.ok_or("no active layer")?;
    let valid = state.doc.layer(layer).is_some_and(|layer| matches!(&layer.content, LayerContent::Smart(smart) if !smart.smart_filters.is_empty()));
    if !valid {
        return Err("select a Smart Object with Smart Filters to target its filter mask".into());
    }
    Ok(Some(Target { document: state.doc.id, layer }))
}

pub(crate) fn set(app: &mut PhotocraftApp, target: Option<Target>) {
    app.ui.smart_filter_mask_target = target;
    if target.is_some() {
        app.ui.mask_target = false;
        app.ui.vector_mask_target = false;
        if let Some(state) = app.session.active_mut() {
            state.channel_view.layer_mask = None;
        }
    }
}

pub(crate) fn sync(app: &mut PhotocraftApp) {
    if app.ui.smart_filter_mask_target.is_some_and(|target| requested(app, true).ok().flatten() != Some(target)) {
        app.ui.smart_filter_mask_target = None;
    }
}

/// Alpha channels and Quick Mask retain their established priority over layer targets.
pub(crate) fn current(app: &PhotocraftApp) -> Option<Target> {
    let target = app.ui.smart_filter_mask_target?;
    let state = app.session.active()?;
    (requested(app, true).ok().flatten() == Some(target)
        && state.channel_view.target == ChannelTarget::Composite
        && state.doc.quick_mask.is_none()
        && photocraft_engine::mask_view_cmds::current(state).is_none())
    .then_some(target)
}

pub(crate) fn with_target(app: &PhotocraftApp, command: &str, mut params: Value) -> Value {
    if params.get("target").is_some() || !photocraft_engine::channel_cmds::follows_target(command) {
        return params;
    }
    let Some(target) = current(app) else { return params };
    if params.is_null() {
        params = json!({});
    } else if !params.is_object() {
        return params;
    }
    params["target"] = json!(photocraft_engine::smart_mask_cmds::TARGET);
    if params.get("layer").is_none() {
        params["layer"] = json!(target.layer.0);
    }
    params
}

/// Only mask pixels are read here. Selecting or drawing a thumbnail never resolves a source.
pub(crate) fn thumbnail(app: &mut PhotocraftApp, ui: &egui::Ui, layer: LayerId, smart: &SmartObject, rect: Rect, actions: &mut Vec<(String, Value)>) {
    let Some(doc) = app.session.active().map(|state| state.doc.clone()) else { return };
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let key = (layer, THUMB);
    let revision = smart.filter_mask.as_ref().map_or(0, |mask| crate::surface_fingerprint(&mask.surface))
        ^ (doc.size.width as u64) << 40
        ^ (doc.size.height as u64).rotate_left(20);
    let texture = match app.thumbs.get(&key).filter(|(old, _)| *old == revision) {
        Some((_, texture)) => texture.id(),
        None => {
            let image = crate::thumb_image(&doc, 64, |x, y| {
                let value = smart.filter_mask.as_ref().map_or(1.0, |mask| mask.surface.sample_channel(x, y, 0));
                [value, value, value, 1.0]
            });
            app.store_thumb(ui.ctx(), key, revision, image)
        }
    };
    ui.painter().image(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);
    if smart.filter_mask.as_ref().is_some_and(|mask| !mask.enabled) {
        let stroke = Stroke::new(1.5, tokens.danger);
        ui.painter().line_segment([rect.left_top(), rect.right_bottom()], stroke);
        ui.painter().line_segment([rect.right_top(), rect.left_bottom()], stroke);
    }
    if current(app).is_some_and(|target| target.layer == layer) {
        crate::mask_thumbs_ui::paint_brackets(ui.painter(), rect, tokens.text);
    }
    let response = ui.interact(rect.expand(2.0), ui.id().with(("smart-filter-mask", layer.0)), Sense::click());
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("smart-filter-mask-rect", layer.0)), rect));
    let response = response.on_hover_text("Smart Filter mask: click to paint with Brush, Pencil or Eraser; Shift-click to enable or disable");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Smart Filter mask"));
    if response.clicked() {
        if ui.input(|input| input.modifiers.shift) && smart.filter_mask.is_some() {
            actions.push(("layer.smartFilter.disableFilterMask".into(), json!({"layer": layer.0})));
        } else {
            actions.push(("ui.smartFilterMaskTarget".into(), json!({"layer": layer.0})));
        }
    }
}

pub(crate) fn select(app: &mut PhotocraftApp, layer: LayerId) -> Result<(), String> {
    app.session.select_layer(layer).map_err(|error| error.to_string())?;
    app.run("channel.target.composite", json!({}))?;
    let target = requested(app, true)?;
    set(app, target);
    Ok(())
}
