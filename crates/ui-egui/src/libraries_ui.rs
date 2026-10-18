//! Window › Libraries (a tab of the Color card): colours, graphics and patterns kept across
//! documents, in the shared preset browser (one folder per library).
//!
//! Clicking a colour sets the foreground (Alt or Ctrl/Cmd: the background). Double-clicking a
//! graphic places it as an embedded smart object, and dragging it onto the canvas places it
//! there; double-clicking a pattern adds it to the document's patterns. The add row saves the
//! foreground colour, the selected layers or the Patterns panel's pattern; Place Linked puts a
//! graphic in as a linked smart object. Every action is a `library.*` engine command.

use egui::{Color32, ColorImage, CornerRadius, Pos2, Rect, TextureHandle, TextureOptions, pos2};
use photocraft_engine::library_cmds::{Item, Kind};
use serde_json::json;

use crate::PhotocraftApp;
use crate::preset_panels::{Ev, GroupView, ItemView, Place, browser, run};
use crate::theme::Tokens;

/// The browser's panel key.
pub const PANEL: &str = "libraries";
/// The Color card tab.
pub const TAB: usize = 4;
const THUMB: u32 = 64;

fn item(app: &PhotocraftApp, key: &str) -> Option<Item> {
    app.session.libraries.item(key.parse().ok()?).cloned()
}

fn c32(rgb: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

/// A graphic's or pattern's preview, decoded once per item content.
fn texture(ctx: &egui::Context, item: &Item) -> Option<TextureHandle> {
    let bytes = match &item.kind {
        Kind::Color { .. } => return None,
        Kind::Graphic { bytes, .. } | Kind::Pattern { bytes, .. } => bytes,
    };
    let id = egui::Id::new(("library-thumb", bytes.as_ptr() as usize, bytes.len()));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<TextureHandle>>(id)) {
        return t;
    }
    let tex = photocraft_engine::library_cmds::preview(item, THUMB).map(|(w, h, px)| {
        ctx.load_texture(format!("library-{}", item.id), ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px), TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// `r` shrunk to the aspect ratio `w`:`h`, centred.
fn fit(r: Rect, w: f32, h: f32) -> Rect {
    let k = (r.width() / w.max(1.0)).min(r.height() / h.max(1.0));
    Rect::from_center_size(r.center(), egui::vec2(w * k, h * k))
}

/// Uses a library item (`center`: where to place a graphic, in document pixels).
pub fn use_item(app: &mut PhotocraftApp, key: &str, background: bool, linked: bool, center: Option<[f64; 2]>) {
    let Some(it) = item(app, key) else { return };
    let mut p = json!({"item": it.id});
    match it.kind {
        Kind::Color { .. } => {
            p["target"] = json!(if background { "background" } else { "foreground" });
            if run(app, "library.use", p).is_some() && !background {
                crate::type_tool::foreground_changed(app);
            }
        }
        Kind::Graphic { .. } => {
            if linked {
                p["linked"] = json!(true);
            }
            if let Some(c) = center {
                p["center"] = json!(c);
            }
            run(app, "library.use", p);
        }
        Kind::Pattern { .. } => {
            if run(app, "library.use", p).is_some() {
                app.ui.status = format!("{} {}", tl!("Added to the document's patterns:"), it.name);
                app.ui.status_error = false;
            }
        }
    }
}

/// Saves something to the library the selection is in (else the first): `what` is
/// `color`, `graphic` or `pattern`.
pub fn add(app: &mut PhotocraftApp, what: &str) {
    let library = app
        .ui
        .presets_ui
        .selected
        .get(PANEL)
        .and_then(|k| k.parse::<u64>().ok())
        .and_then(|id| app.session.libraries.locate(id))
        .and_then(|(li, _)| app.session.libraries.list.get(li))
        .map(|l| l.name.clone());
    let mut p = library.map_or_else(|| json!({}), |l| json!({"library": l}));
    let cmd = match what {
        "graphic" => "library.addGraphic",
        "pattern" => {
            if let Some(sel) = app.ui.presets_ui.selected.get("patterns") {
                p["pattern"] = json!(sel);
            }
            "library.addPattern"
        }
        _ => "library.addColor",
    };
    if let Some(r) = run(app, cmd, p) {
        let id = r["item"]["id"].as_u64().unwrap_or_default().to_string();
        let name = r["item"]["name"].as_str().unwrap_or_default().to_string();
        app.ui.presets_ui.selected.insert(PANEL.into(), id.clone());
        // Named at once, like a new swatch.
        app.ui.presets_ui.renaming = Some((PANEL.into(), id, name));
    }
}

/// The Libraries panel body.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let has_doc = app.session.active().is_some();
    // The add row: what the panel can save.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if crate::widgets::secondary_button(ui, tl!("+ Color"), 0.0).on_hover_text(tl!("Add the foreground color")).clicked() {
            add(app, "color");
        }
        if ui
            .add_enabled_ui(has_doc, |ui| crate::widgets::secondary_button(ui, tl!("+ Graphic"), 0.0))
            .inner
            .on_hover_text(tl!("Add the selected layers"))
            .clicked()
        {
            add(app, "graphic");
        }
        if crate::widgets::secondary_button(ui, tl!("+ Pattern"), 0.0).on_hover_text(tl!("Add the pattern selected in the Patterns panel")).clicked() {
            add(app, "pattern");
        }
    });
    ui.add_space(4.0);
    let libs = &app.session.libraries.list;
    if libs.iter().all(|l| l.items.is_empty()) {
        ui.label(egui::RichText::new(tl!("Save colors, graphics and patterns here to use them in any document.")).color(t.text_faint).size(11.5));
        ui.add_space(4.0);
    }
    let groups: Vec<GroupView> = libs
        .iter()
        .map(|l| GroupView { name: l.name.clone(), items: l.items.iter().map(|i| ItemView { key: i.id.to_string(), name: i.name.clone() }).collect() })
        .collect();
    let items: Vec<Vec<Item>> = libs.iter().map(|l| l.items.clone()).collect();
    let canvas = crate::rulers::content_rect(app, app.last_canvas_rect);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    st.sizes.entry(PANEL.to_string()).or_insert(40.0);
    let max_h = (ui.available_height() - 70.0).clamp(80.0, 320.0);
    let ctx = ui.ctx().clone();
    let events = browser(ui, &mut st, PANEL, &groups, Place { canvas, max_h, new_tip: tl!("Add the foreground color") }, &mut |ui, r, gi, ii| {
        let Some(it) = items.get(gi).and_then(|g| g.get(ii)) else { return };
        match &it.kind {
            Kind::Color { rgb } => {
                ui.painter().rect_filled(r, CornerRadius::same(2), c32(*rgb));
                ui.painter().rect_stroke(r, CornerRadius::same(2), egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            }
            Kind::Graphic { .. } | Kind::Pattern { .. } => {
                if let Some(tex) = texture(&ctx, it) {
                    let s = tex.size_vec2();
                    let inner = if matches!(it.kind, Kind::Pattern { .. }) { r } else { fit(r, s.x, s.y) };
                    crate::widgets::checker(&ui.painter_at(inner), inner, 4.0);
                    ui.painter().image(tex.id(), inner, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    // Bytes that no longer decode: an outlined cell, still renameable and deletable.
                    ui.painter().rect_stroke(r, CornerRadius::same(2), egui::Stroke::new(1.0, t.danger), egui::StrokeKind::Inside);
                }
            }
        }
    });
    app.ui.presets_ui = st;
    let mods = ui.input(|i| i.modifiers);
    let background = mods.alt || mods.command || mods.ctrl;
    for e in events {
        handle(app, e, background);
    }
    // A selected graphic can also go in linked (desktop: it refers to the library's file).
    let selected = app.ui.presets_ui.selected.get(PANEL).cloned();
    if let Some(key) = selected
        && let Some(it) = item(app, &key)
        && matches!(it.kind, Kind::Graphic { .. })
    {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(has_doc, |ui| {
                if crate::widgets::secondary_button(ui, tl!("Place"), 0.0).on_hover_text(tl!("Place as an embedded smart object (a copy)")).clicked() {
                    use_item(app, &key, false, false, None);
                }
                if crate::widgets::secondary_button(ui, tl!("Place Linked"), 0.0)
                    .on_hover_text(tl!("Place as a linked smart object that refers to the library's file"))
                    .clicked()
                {
                    use_item(app, &key, false, true, None);
                }
            });
        });
    }
}

fn handle(app: &mut PhotocraftApp, e: Ev, background: bool) {
    let sel = |app: &mut PhotocraftApp, k: &str| {
        app.ui.presets_ui.selected.insert(PANEL.into(), k.to_string());
    };
    let library = |app: &PhotocraftApp, g: &str| app.session.libraries.list.iter().find(|l| l.name == g).map(|l| l.name.clone());
    let id = |k: &str| k.parse::<u64>().ok();
    match e {
        Ev::Select(k) => {
            sel(app, &k);
            if item(app, &k).is_some_and(|i| matches!(i.kind, Kind::Color { .. })) {
                use_item(app, &k, background, false, None);
            }
        }
        Ev::Activate(k) => {
            sel(app, &k);
            if item(app, &k).is_some_and(|i| matches!(i.kind, Kind::Color { .. })) {
                // A colour applies on click; double-click renames it, like a swatch.
                let name = item(app, &k).map(|i| i.name).unwrap_or_default();
                app.ui.presets_ui.renaming = Some((PANEL.into(), k, name));
            } else {
                use_item(app, &k, false, false, None);
            }
        }
        Ev::Drop(k, pos) => {
            sel(app, &k);
            let center = crate::canvas::doc_point_at(app, pos);
            use_item(app, &k, background, false, center);
        }
        Ev::Rename(k, n) => {
            if let Some(i) = id(&k) {
                run(app, "library.renameItem", json!({"item": i, "name": n}));
            }
        }
        Ev::Delete(k) => {
            if let Some(i) = id(&k)
                && run(app, "library.deleteItem", json!({"item": i})).is_some()
            {
                app.ui.presets_ui.selected.remove(PANEL);
            }
        }
        Ev::Move(k, g) => {
            if let (Some(i), Some(l)) = (id(&k), library(app, &g)) {
                run(app, "library.moveItem", json!({"item": i, "library": l}));
            }
        }
        Ev::RenameGroup(g, n) => drop(run(app, "library.rename", json!({"library": g, "name": n}))),
        Ev::DeleteGroup(g) => drop(run(app, "library.delete", json!({"library": g}))),
        Ev::NewGroup => {
            if let Some(r) = run(app, "library.new", json!({})) {
                let g = r["library"].as_str().unwrap_or_default().to_string();
                app.ui.presets_ui.renaming = Some((PANEL.into(), format!("group:{g}"), g));
            }
        }
        Ev::New => add(app, "color"),
    }
}

#[cfg(test)]
mod tests;
