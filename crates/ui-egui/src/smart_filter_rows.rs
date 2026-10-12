//! Smart Filter stack controls. Collapsing is view state; every filter mutation goes through
//! an existing engine command. Drag bindings prevent a delayed drop from moving another filter.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::{BlendMode, DocId, Layer, LayerContent, LayerId, SmartObject};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(crate) const BLENDING: &str = "layer.smartFilter.blendingOptions";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackView {
    pub document: DocId,
    pub layer: LayerId,
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterDrag {
    pub document: DocId,
    pub revision: u64,
    pub layer: LayerId,
    pub index: usize,
}

fn smart(app: &PhotocraftApp, layer: LayerId) -> Result<&SmartObject, String> {
    let state = app.session.active().ok_or("no document open")?;
    match state.doc.layer(layer).map(|layer| &layer.content) {
        Some(LayerContent::Smart(smart)) if !smart.smart_filters.is_empty() => Ok(smart),
        _ => Err("select a Smart Object with Smart Filters".into()),
    }
}

pub(crate) fn collapsed(app: &PhotocraftApp, layer: LayerId) -> bool {
    let Some(state) = app.session.active() else { return false };
    app.ui.smart_filter_stacks.iter().any(|view| view.document == state.doc.id && view.layer == layer && view.collapsed)
}

pub(crate) fn validate_collapse(app: &PhotocraftApp) -> Result<LayerId, String> {
    let layer = app.session.active().and_then(|state| state.active_layer).ok_or("no active layer")?;
    smart(app, layer)?;
    Ok(layer)
}

pub(crate) fn set_collapsed(app: &mut PhotocraftApp, layer: LayerId, collapsed: bool) -> Result<(), String> {
    smart(app, layer)?;
    let document = app.session.active().ok_or("no document open")?.doc.id;
    app.ui.smart_filter_stacks.retain(|view| view.document != document || view.layer != layer);
    if collapsed {
        app.ui.smart_filter_stacks.push(StackView { document, layer, collapsed });
    }
    Ok(())
}

pub(crate) fn sync(app: &mut PhotocraftApp) {
    let documents = app.session.documents();
    app.ui.smart_filter_stacks.retain(|view| {
        documents.iter().any(|state| {
            state.doc.id == view.document
                && state.doc.layer(view.layer).is_some_and(|layer| matches!(&layer.content, LayerContent::Smart(smart) if !smart.smart_filters.is_empty()))
        })
    });
    if app.ui.smart_filter_drag.as_ref().is_some_and(|drag| !valid_drag(app, drag)) {
        app.ui.smart_filter_drag = None;
    }
}

pub(crate) fn finish_pointer_frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|input| input.pointer.any_released()) {
        app.ui.smart_filter_drag = None;
    }
}

fn valid_drag(app: &PhotocraftApp, drag: &FilterDrag) -> bool {
    app.session.active().is_some_and(|state| state.doc.id == drag.document && state.revision == drag.revision)
        && smart(app, drag.layer).is_ok_and(|smart| drag.index < smart.smart_filters.len())
}

pub(crate) fn begin_drag(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<(), String> {
    if index >= smart(app, layer)?.smart_filters.len() {
        return Err("no Smart Filter at that index".into());
    }
    let state = app.session.active().ok_or("no document open")?;
    app.ui.smart_filter_drag = Some(FilterDrag { document: state.doc.id, revision: state.revision, layer, index });
    Ok(())
}

/// The visual rows run top-to-bottom while engine indices run bottom-to-top. Convert an
/// above/below insertion gap before removing the source; adjacent no-op drops do not edit.
pub(crate) fn drop_command(app: &PhotocraftApp, drag: &FilterDrag, layer: LayerId, index: usize, above: bool) -> Result<Option<Value>, String> {
    if !valid_drag(app, drag) {
        return Err("the document or filter stack changed; drag the Smart Filter again".into());
    }
    if layer != drag.layer {
        return Err("Smart Filters can be reordered within the same layer only".into());
    }
    let count = smart(app, layer)?.smart_filters.len();
    if index >= count {
        return Err("no Smart Filter at that drop target".into());
    }
    let gap = index.saturating_add(usize::from(above));
    let to = gap.saturating_sub(usize::from(drag.index < gap));
    if to == drag.index {
        return Ok(None);
    }
    Ok(Some(json!({"layer": layer.0, "index": drag.index, "to": to})))
}

pub(crate) fn open_blending(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<u64, String> {
    smart(app, layer)?.smart_filters.get(index).ok_or("no Smart Filter at that index")?;
    crate::smart_ui::authorize_filter_preview(app, layer)?;
    let state = app.session.active().ok_or("no document open")?;
    let filter = smart(app, layer)?.smart_filters.get(index).ok_or("no Smart Filter at that index")?;
    let fields = Map::from_iter([
        ("__command".into(), json!(BLENDING)),
        ("__label".into(), json!("Smart Filter Blending Options")),
        ("__filter".into(), json!(true)),
        ("__preview".into(), json!(true)),
        ("__smartFilter".into(), json!({"document": state.doc.id.0, "revision": state.revision, "layer": layer.0, "index": index})),
        ("blend".into(), json!(format!("{:?}", filter.blend))),
        ("opacity".into(), json!(filter.opacity)),
    ]);
    let dialog = app.ui.open_dialog(crate::state::DialogKind::Command, fields);
    crate::smart_ui::capture_dialog_preview_authority(app, dialog);
    Ok(dialog)
}

pub(crate) fn blending_owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(BLENDING) && fields.contains_key("__smartFilter")
}

pub(crate) fn blending_body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut mode = fields.get("blend").and_then(Value::as_str).and_then(photocraft_engine::commands::blend_from_str).unwrap_or(BlendMode::Normal);
    let prior = mode;
    ui.horizontal(|ui| {
        ui.label("Mode");
        egui::ComboBox::from_id_salt("smart-filter-blend-mode").selected_text(mode.label()).show_ui(ui, |ui| {
            for choice in BlendMode::layer_modes() {
                ui.selectable_value(&mut mode, choice, choice.label());
            }
        });
    });
    if mode != prior {
        fields.insert("blend".into(), json!(format!("{mode:?}")));
    }
    if mode == BlendMode::Dissolve {
        ui.colored_label(crate::theme::Tokens::get(ui.ctx()).warning, "Dissolve currently blends smoothly; a speckled Dissolve pattern is not supported yet.");
    }
    let mut percent = fields.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) * 100.0;
    let changed = ui
        .horizontal(|ui| {
            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%")).changed()
        })
        .inner;
    if changed {
        fields.insert("opacity".into(), json!(percent / 100.0));
    }
    let mut preview = fields.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    if ui.checkbox(&mut preview, "Preview").changed() {
        fields.insert("__preview".into(), json!(preview));
    }
}

/// UI actions are processed after drawing the Layers panel. A queued drag retains its binding
/// until here, so an earlier queued mutation cannot silently change what its index addresses.
pub(crate) fn action(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str, params: &Value) -> Option<Result<(), String>> {
    if !matches!(command, "ui.smartFilterBlend" | "ui.smartFilterEdit" | "ui.smartFilterMove" | "ui.smartFilterCollapse") {
        return None;
    }
    Some((|| {
        let layer = params.get("layer").and_then(Value::as_u64).map(LayerId).ok_or("need a Smart Object layer")?;
        if command == "ui.smartFilterCollapse" {
            return set_collapsed(app, layer, params.get("collapsed").and_then(Value::as_bool).ok_or("need a collapsed boolean")?);
        }
        let index = params.get("index").and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()).ok_or("need a Smart Filter index")?;
        match command {
            "ui.smartFilterBlend" => {
                open_blending(app, layer, index)?;
            }
            "ui.smartFilterEdit" => {
                let object = smart(app, layer)?.clone();
                if object.smart_filters.get(index).is_some_and(|filter| filter.command == photocraft_engine::lens_cmds::RAW) {
                    crate::camera_raw_ui::open_smart_filter(app, ctx, layer, index)?;
                } else {
                    crate::smart_ui::open_editor_dialog(app, ctx, layer.0, &object, index).ok_or("this Smart Filter has no parameter dialog")?;
                }
            }
            "ui.smartFilterMove" => {
                app.ui.smart_filter_drag = None;
                let drag: FilterDrag =
                    serde_json::from_value(params.get("drag").cloned().ok_or("need the original drag binding")?).map_err(|error| error.to_string())?;
                let above = params.get("above").and_then(Value::as_bool).ok_or("need an insertion side")?;
                if let Some(params) = drop_command(app, &drag, layer, index, above)? {
                    app.run("layer.smartFilter.move", params)?;
                }
            }
            _ => {}
        }
        Ok(())
    })())
}

fn menu_action(ui: &mut egui::Ui, label: &str, enabled: bool, command: &str, params: Value, actions: &mut Vec<(String, Value)>) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        actions.push((command.into(), params));
        ui.close();
    }
}

fn context(ui: &mut egui::Ui, layer: LayerId, smart: &SmartObject, index: Option<usize>, collapsed: bool, actions: &mut Vec<(String, Value)>) {
    let layer_params = json!({"layer": layer.0});
    if let Some(index) = index {
        let params = json!({"layer": layer.0, "index": index});
        let filter = smart.smart_filters.get(index);
        let editable = filter.is_some_and(|filter| filter.command != photocraft_engine::smart_cmds::UNSUPPORTED_FILTER);
        menu_action(ui, "Edit Smart Filter…", editable, "ui.smartFilterEdit", params.clone(), actions);
        menu_action(ui, "Blending Options…", editable, "ui.smartFilterBlend", params.clone(), actions);
        menu_action(
            ui,
            if filter.is_some_and(|filter| filter.visible) { "Hide Smart Filter" } else { "Show Smart Filter" },
            true,
            "layer.smartFilter.setVisible",
            params.clone(),
            actions,
        );
        ui.separator();
        menu_action(
            ui,
            "Move Up",
            index.saturating_add(1) < smart.smart_filters.len(),
            "layer.smartFilter.move",
            json!({"layer":layer.0,"index":index,"to":index.saturating_add(1)}),
            actions,
        );
        menu_action(ui, "Move Down", index > 0, "layer.smartFilter.move", json!({"layer":layer.0,"index":index,"to":index.saturating_sub(1)}), actions);
        menu_action(ui, "Delete Smart Filter", true, "layer.smartFilter.delete", params, actions);
    } else {
        menu_action(
            ui,
            if collapsed { "Expand Smart Filters" } else { "Collapse Smart Filters" },
            true,
            "ui.smartFilterCollapse",
            json!({"layer": layer.0, "collapsed": !collapsed}),
            actions,
        );
        menu_action(
            ui,
            if smart.filters_enabled { "Disable Smart Filters" } else { "Enable Smart Filters" },
            true,
            "layer.smartFilter.disableSmartFilters",
            layer_params.clone(),
            actions,
        );
        menu_action(ui, "Clear Smart Filters", true, "layer.smartFilter.clearSmartFilters", layer_params.clone(), actions);
        ui.separator();
        menu_action(ui, "Select Filter Mask", true, "ui.smartFilterMaskTarget", layer_params.clone(), actions);
        if let Some(mask) = &smart.filter_mask {
            menu_action(
                ui,
                if mask.enabled { "Disable Filter Mask" } else { "Enable Filter Mask" },
                true,
                "layer.smartFilter.disableFilterMask",
                layer_params.clone(),
                actions,
            );
            menu_action(ui, "Delete Filter Mask", true, "layer.smartFilter.deleteFilterMask", layer_params, actions);
        }
    }
}

pub(crate) fn rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let LayerContent::Smart(smart) = &layer.content else { return };
    if smart.smart_filters.is_empty() {
        return;
    }
    let collapsed = collapsed(app, layer.id);
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let indent = 30.0 + depth as f32 * 14.0 + 44.0;
    let indices = std::iter::once(None).chain((0..smart.smart_filters.len()).rev().filter(|_| !collapsed).map(Some));
    for index in indices {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, tokens.hover.gamma_multiply(0.35));
        }
        let (name, shown) = match index.and_then(|index| smart.smart_filters.get(index)) {
            Some(filter) => (crate::smart_ui::filter_label(filter), filter.visible),
            None => ("Smart Filters".into(), smart.filters_enabled),
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, &name));
        let eye = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(18.0, 18.0));
        let eye_response = ui.interact(eye, ui.id().with(("smart-eye", layer.id.0, index)), Sense::click());
        if shown {
            crate::icons::paint(ui, eye, "eye", 12.0, tokens.icon);
        }
        if eye_response.clicked() {
            actions.push(match index {
                Some(index) => ("layer.smartFilter.setVisible".into(), json!({"layer":layer.id.0,"index":index})),
                None => ("layer.smartFilter.disableSmartFilters".into(), json!({"layer":layer.id.0})),
            });
        }
        let x = rect.left() + indent;
        if let Some(index) = index {
            let blend_rect = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
            let blend = ui.interact(blend_rect, ui.id().with(("smart-blend", layer.id.0, index)), Sense::click());
            crate::icons::paint(ui, blend_rect, "sliders-horizontal", 13.0, tokens.icon);
            blend.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Blending Options: {name}")));
            if blend.clicked() {
                actions.push(("ui.smartFilterBlend".into(), json!({"layer":layer.id.0,"index":index})));
            }
            if response.double_clicked() {
                actions.push(("ui.smartFilterEdit".into(), json!({"layer":layer.id.0,"index":index})));
            }
            if response.drag_started() {
                let _ = begin_drag(app, layer.id, index);
            }
            if let Some(drag) = app.ui.smart_filter_drag.as_ref().filter(|drag| drag.layer == layer.id && valid_drag(app, drag))
                && let Some(pointer) = ui.input(|input| input.pointer.latest_pos()).filter(|pointer| rect.contains(*pointer))
            {
                let above = pointer.y < rect.center().y;
                if drop_command(app, drag, layer.id, index, above).ok().flatten().is_some() {
                    let y = if above { rect.top() } else { rect.bottom() };
                    ui.painter().line_segment([pos2(x - 8.0, y), pos2(rect.right(), y)], Stroke::new(2.0, tokens.text));
                }
                if ui.input(|input| input.pointer.any_released()) {
                    actions.push(("ui.smartFilterMove".into(), json!({"layer":layer.id.0,"index":index,"above":above,"drag":drag})));
                }
            }
        } else {
            let arrow_rect = Rect::from_center_size(pos2(x - 33.0, rect.center().y), vec2(16.0, 18.0));
            let arrow = ui.interact(arrow_rect, ui.id().with(("smart-collapse", layer.id.0)), Sense::click());
            crate::icons::paint(ui, arrow_rect, if collapsed { "chevron-right" } else { "chevron-down" }, 12.0, tokens.icon);
            arrow.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, if collapsed { "Expand Smart Filters" } else { "Collapse Smart Filters" })
            });
            if arrow.clicked() {
                actions.push(("ui.smartFilterCollapse".into(), json!({"layer":layer.id.0,"collapsed":!collapsed})));
            }
            let mask_rect = Rect::from_center_size(pos2(x - 12.0, rect.center().y), vec2(18.0, 14.0));
            crate::smart_mask_ui::thumbnail(app, ui, layer.id, smart, mask_rect, actions);
        }
        ui.painter().with_clip_rect(rect.intersect(Rect::from_min_max(pos2(x, rect.top()), pos2(rect.right() - 26.0, rect.bottom())))).text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(11.5),
            if shown && smart.filters_enabled { tokens.text_dim } else { tokens.text_faint },
        );
        response.context_menu(|ui| context(ui, layer.id, smart, index, collapsed, actions));
    }
}
