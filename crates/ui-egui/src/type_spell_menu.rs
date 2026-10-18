//! Spelling while typing on the canvas (UI-217-8): words the dictionary doesn't know get a wavy
//! underline, and a right-click on one lists suggestions, Add to Dictionary and Check Spelling….
//! Offsets are character indices, like [`crate::state::TextEdit`].

use std::collections::HashSet;

use egui::{Color32, Context, Pos2, Stroke};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_text::spell::Dictionary;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;

/// Suggestions listed in the menu.
const SUGGESTIONS: usize = 6;

/// Texts longer than this (characters) aren't underlined: checking them every frame would cost
/// more than it helps. The right-click menu still works on them.
const UNDERLINE_MAX_CHARS: usize = 20_000;

/// The open spelling menu: the misspelled word under the right-click and what could replace it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpellMenu {
    pub pos: [f32; 2],
    pub layer: u64,
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub suggestions: Vec<String>,
}

fn char_of(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or_else(|| text.chars().count(), |s| s.chars().count())
}

fn edited_text(app: &PhotocraftApp, layer: u64) -> Option<String> {
    match &app.session.active()?.doc.layer(LayerId(layer))?.content {
        LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// The user dictionary (Edit › Check Spelling › Add), lowercased as the checker compares.
fn user_words(app: &PhotocraftApp) -> HashSet<String> {
    app.session.prefs().user_dictionary.iter().map(|w| w.to_lowercase()).collect()
}

/// Misspelled words of `text` as (start, end, word), character offsets.
pub(crate) fn misspelled(app: &PhotocraftApp, text: &str) -> Vec<(usize, usize, String)> {
    Dictionary::english().misspellings(text, &user_words(app)).into_iter().map(|m| (char_of(text, m.start), char_of(text, m.end), m.word)).collect()
}

/// Words to underline in the edited layer, cached per text so a frame doesn't re-check it.
pub(crate) fn underlined(app: &PhotocraftApp, ctx: &Context, text: &str) -> Vec<(usize, usize)> {
    if text.chars().count() > UNDERLINE_MAX_CHARS {
        return Vec::new();
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let mut words: Vec<&String> = app.session.prefs().user_dictionary.iter().collect();
        words.sort();
        words.hash(&mut h);
        egui::Id::new(("type-spell-underline", h.finish()))
    };
    if let Some(v) = ctx.data(|d| d.get_temp::<std::sync::Arc<Vec<(usize, usize)>>>(key)) {
        return v.to_vec();
    }
    let v: Vec<(usize, usize)> = misspelled(app, text).into_iter().map(|(a, b, _)| (a, b)).collect();
    ctx.data_mut(|d| d.insert_temp(key, std::sync::Arc::new(v.clone())));
    v
}

/// A wavy underline from `a` to `b` (screen points), `amp` points high.
pub(crate) fn squiggle(painter: &egui::Painter, a: Pos2, b: Pos2, amp: f32, color: Color32) {
    let d = b - a;
    let len = d.length();
    if !len.is_finite() || len < 1.0 {
        return;
    }
    let (dir, n) = (d / len, egui::vec2(-d.y, d.x) / len);
    let step = 2.0f32;
    let count = ((len / step).ceil() as usize).min(4096);
    let pts: Vec<Pos2> = (0..=count)
        .map(|i| {
            let t = (i as f32 * step).min(len);
            let s = if i % 2 == 0 { amp } else { -amp };
            a + dir * t + n * s
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
}

/// Right-click at document point `doc` (screen `pos`) while editing type: opens the menu when the
/// pointer is on a misspelled word. Returns whether it opened.
pub fn open(app: &mut PhotocraftApp, doc: [f64; 2], pos: [f32; 2]) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    if !pos.iter().all(|v| v.is_finite()) || !doc.iter().all(|v| v.is_finite()) {
        return false;
    }
    let Some(text) = edited_text(app, ed.layer) else { return false };
    let off = crate::type_tool::hit_offset(app, LayerId(ed.layer), doc[0], doc[1]);
    let Some((start, end, word)) = misspelled(app, &text).into_iter().find(|(a, b, _)| *a <= off && off <= *b) else { return false };
    let suggestions = Dictionary::english().suggest(&word, SUGGESTIONS);
    app.ui.canvas_tool_menu = None;
    app.ui.layer_menu = None;
    app.ui.spell_menu = Some(SpellMenu { pos, layer: ed.layer, start, end, word, suggestions });
    true
}

/// Replaces the menu's word with `with`, inside the editing session (one history step with the
/// typing). Refuses when the text changed under the menu.
pub fn replace(app: &mut PhotocraftApp, menu: &SpellMenu, with: &str) -> Result<(), String> {
    let text = edited_text(app, menu.layer).ok_or("the type layer is gone")?;
    let now: String = text.chars().skip(menu.start).take(menu.end.saturating_sub(menu.start)).collect();
    if now != menu.word {
        return Err(tl!("The text changed; right-click the word again.").to_string());
    }
    let ed = app.ui.text_edit.as_mut().filter(|e| e.layer == menu.layer).ok_or("not editing that type layer")?;
    ed.anchor = menu.start;
    ed.caret = menu.end;
    crate::type_tool::insert(app, with);
    Ok(())
}

enum Pick {
    Replace(String),
    Add,
    Dialog,
}

fn choose(app: &mut PhotocraftApp, menu: &SpellMenu, pick: Pick) {
    app.ui.spell_menu = None;
    let r = match pick {
        Pick::Replace(w) => replace(app, menu, &w),
        Pick::Add => app.run("edit.checkSpelling", json!({"action": "addToDictionary", "word": menu.word})).map(|_| ()),
        Pick::Dialog => app.run("edit.checkSpelling", json!({"allLayers": false, "layer": menu.layer})).map(|_| ()),
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Draws the open menu and applies the picked row.
pub fn show(app: &mut PhotocraftApp, ctx: &Context) {
    let Some(menu) = app.ui.spell_menu.clone() else { return };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || app.ui.text_edit.as_ref().is_none_or(|e| e.layer != menu.layer) {
        app.ui.spell_menu = None;
        return;
    }
    let id = egui::Id::new("canvas-spelling-menu");
    let screen = ctx.content_rect();
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(210.0, 160.0), |r| r.size());
    let pos = egui::pos2(menu.pos[0].min(screen.right() - size.x).max(screen.left()), menu.pos[1].min(screen.bottom() - size.y).max(screen.top()));
    let mut pick = None;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            let t = crate::theme::Tokens::get(ui.ctx());
            let v = &mut ui.style_mut().visuals;
            v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.hovered.weak_bg_fill = t.accent;
            v.widgets.hovered.bg_fill = t.accent;
            v.widgets.hovered.bg_stroke = Stroke::NONE;
            v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
            v.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(200.0);
            let row = |ui: &mut egui::Ui, label: &str, enabled: bool, strong: bool| {
                let text = if strong { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
                ui.add_enabled(enabled, egui::Button::new(text).frame_when_inactive(false).min_size(egui::vec2(200.0, 20.0))).clicked()
            };
            if menu.suggestions.is_empty() {
                row(ui, tl!("No Suggestions"), false, false);
            }
            for s in &menu.suggestions {
                if row(ui, s, true, true) {
                    pick = Some(Pick::Replace(s.clone()));
                }
            }
            ui.separator();
            if row(ui, tl!("Add to Dictionary"), true, false) {
                pick = Some(Pick::Add);
            }
            if row(ui, tl!("Check Spelling…"), true, false) {
                pick = Some(Pick::Dialog);
            }
        });
    });
    if let Some(p) = pick {
        choose(app, &menu, p);
    } else if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p))) {
        app.ui.spell_menu = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TextEdit, Tool};

    fn editing(text: &str) -> (PhotocraftApp, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 100})).unwrap();
        let id = app.run("type.create", json!({"text": text, "x": 10, "y": 60, "size": 24})).unwrap()["layer"].as_u64().unwrap();
        app.ui.tool = Tool::Type;
        app.ui.text_edit =
            Some(TextEdit { layer: id, caret: 0, anchor: 0, session: "type-test".into(), created: false, dragging: false, resize: None, preedit: None });
        (app, id)
    }

    #[test]
    fn misspelled_words_are_found_by_character_offset() {
        let (app, _) = editing("Café teh cat");
        assert_eq!(misspelled(&app, "Café teh cat"), vec![(5, 8, "teh".to_string())]);
    }

    #[test]
    fn a_suggestion_replaces_the_word_in_the_editing_session() {
        let (mut app, id) = editing("the teh cat");
        let menu = SpellMenu { pos: [0.0, 0.0], layer: id, start: 4, end: 7, word: "teh".into(), suggestions: Dictionary::english().suggest("teh", 6) };
        assert!(menu.suggestions.iter().any(|s| s == "the"));
        replace(&mut app, &menu, "the").unwrap();
        assert_eq!(edited_text(&app, id).as_deref(), Some("the the cat"));
        let ed = app.ui.text_edit.as_ref().unwrap();
        assert_eq!((ed.caret, ed.anchor), (7, 7));
        // A stale menu (the word changed since) refuses rather than replacing other text.
        assert!(replace(&mut app, &menu, "ten").is_err());
        assert_eq!(edited_text(&app, id).as_deref(), Some("the the cat"));
    }

    #[test]
    fn added_words_stop_being_underlined() {
        let (mut app, id) = editing("zorblat");
        let ctx = Context::default();
        assert_eq!(underlined(&app, &ctx, "zorblat"), vec![(0, 7)]);
        let menu = SpellMenu { pos: [0.0, 0.0], layer: id, start: 0, end: 7, word: "zorblat".into(), suggestions: Vec::new() };
        choose(&mut app, &menu, Pick::Add);
        assert!(underlined(&app, &ctx, "zorblat").is_empty());
        app.run("edit.checkSpelling", json!({"action": "removeFromDictionary", "word": "zorblat"})).unwrap();
    }

    #[test]
    fn right_click_opens_only_on_a_misspelled_word() {
        let (mut app, id) = editing("teh");
        // Far from the text: the hit lands at an end of the text, which touches the word, so
        // use a correct word to check the negative case.
        assert!(open(&mut app, [12.0, 55.0], [5.0, 5.0]));
        let m = app.ui.spell_menu.clone().unwrap();
        assert_eq!((m.layer, m.start, m.end, m.word.as_str()), (id, 0, 3, "teh"));
        app.ui.spell_menu = None;
        app.run("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 3, "text": "the"}})).unwrap();
        assert!(!open(&mut app, [12.0, 55.0], [5.0, 5.0]));
        assert!(!open(&mut app, [f64::NAN, 0.0], [5.0, 5.0]));
    }
}
