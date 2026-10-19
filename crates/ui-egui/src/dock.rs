//! Right-dock layout (#88): panel groups with fixed heights, like Photoshop's dock columns.
//!
//! A group's height never follows its content: content taller than the group scrolls inside
//! it. The last expanded group (Layers by default) fills what the others leave. Drag the gap
//! between two groups to resize them, double-click a tab (or use the panel menu) to collapse a
//! group to its tab strip, and drag a tab strip to move the group up or down the column
//! (unless Window › Workspace › Lock Workspace is on). Drag a tab strip out of the column (or
//! pick Float Panel Group in its menu) to float the group over the workspace; drag it back onto
//! the column (or pick Dock Panel Group) to dock it again.
//!
//! The layout is [`DockLayout`] in `UiState::dock` (serialisable, drivable with `ui.set`), saved
//! with Window › Workspace › New Workspace…, reset by Reset Workspace, and remembered across
//! launches in the preferences (`panelLayout`) while Remember Workspace Changes is on.

use std::collections::BTreeMap;

use egui::{Pos2, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{DockTabs, Panels};
use crate::theme::Tokens;
use crate::widgets;

/// A dock panel group (one tab strip).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    /// Color | Swatches | Gradients | Patterns | Libraries.
    Color,
    /// Properties | Adjustments.
    Properties,
    /// Character | Paragraph (not in the default workspace; Window › Character opens it, #150).
    Character,
    /// Navigator | Histogram | Info.
    Navigator,
    /// History | Actions | Layer Comps.
    History,
    /// Layers | Channels | Paths.
    Layers,
}

impl Group {
    /// Photoshop Essentials order, top to bottom.
    pub const ALL: [Group; 6] = [Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History, Group::Layers];

    pub fn key(self) -> &'static str {
        match self {
            Group::Color => "color",
            Group::Properties => "properties",
            Group::Character => "character",
            Group::Navigator => "navigator",
            Group::History => "history",
            Group::Layers => "layers",
        }
    }

    /// Height (points, tab strip included) a group gets until the user resizes it, when the
    /// column has room (see [`DockLayout::heights_for`]).
    pub fn default_height(self) -> f32 {
        match self {
            Group::Color => 190.0,
            Group::Properties => 250.0,
            Group::Character => 270.0,
            Group::Navigator => 210.0,
            Group::History => 200.0,
            Group::Layers => 320.0,
        }
    }

    /// What a group left at its default height gives way down to so the filler (Layers) keeps
    /// [`Group::preferred_fill`] in a short column (#147). Content taller than this scrolls.
    pub fn compact_height(self) -> f32 {
        match self {
            Group::Color => 130.0,
            Group::Properties => 160.0,
            Group::Character => 160.0,
            Group::Navigator => 140.0,
            Group::History => 130.0,
            Group::Layers => 200.0,
        }
    }

    /// The height the filling group asks for before default-sized groups above it get their
    /// full defaults: Layers wants room for about ten rows at 900 pt (#147).
    pub fn preferred_fill(self) -> f32 {
        match self {
            Group::Layers => 500.0,
            g => g.min_height(),
        }
    }

    /// The smallest height an expanded group can be dragged or squeezed to.
    pub fn min_height(self) -> f32 {
        match self {
            Group::Layers => 140.0,
            _ => 80.0,
        }
    }

    pub fn tabs(self, pro: bool) -> &'static [&'static str] {
        match self {
            // Photoshop Essentials: Color | Swatches | Gradients | Patterns, plus Libraries.
            Group::Color if pro => &["Color", "Swatches", "Gradients", "Patterns", "Libraries"],
            Group::Color => &["Swatches", "Color", "Gradients", "Patterns", "Libraries"],
            Group::Properties => &["Properties", "Adjustments"],
            Group::Character => &["Character", "Paragraph"],
            Group::Navigator => &["Navigator", "Histogram", "Info"],
            Group::History => &["History", "Actions", "Layer Comps"],
            Group::Layers => &["Layers", "Channels", "Paths"],
        }
    }

    /// Tabs that lay out their own scrolling list and footer (they fill the group).
    pub fn scrolls_itself(self, tab: usize, pro: bool) -> bool {
        matches!((self, tab), (Group::Layers, 0) | (Group::History, 0)) || (pro && matches!((self, tab), (Group::Color, 0)))
    }

    fn tab_mut(self, tabs: &mut DockTabs) -> &mut usize {
        match self {
            Group::Color => &mut tabs.color,
            Group::Properties => &mut tabs.properties,
            Group::Character => &mut tabs.character,
            Group::Navigator => &mut tabs.navigator,
            Group::History => &mut tabs.history,
            Group::Layers => &mut tabs.layers,
        }
    }

    /// The group for a `panels` / `dockTabs` key ("color", "properties", …).
    pub fn from_key(key: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.key() == key)
    }

    pub fn shown(self, panels: &Panels) -> bool {
        match self {
            Group::Color => panels.color,
            Group::Properties => panels.properties,
            Group::Character => panels.character,
            Group::Navigator => panels.navigator,
            Group::History => panels.history,
            Group::Layers => panels.layers,
        }
    }

    fn shown_mut(self, panels: &mut Panels) -> &mut bool {
        match self {
            Group::Color => &mut panels.color,
            Group::Properties => &mut panels.properties,
            Group::Character => &mut panels.character,
            Group::Navigator => &mut panels.navigator,
            Group::History => &mut panels.history,
            Group::Layers => &mut panels.layers,
        }
    }
}

/// Order, heights, collapsed state and visibility of individual dock tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockLayout {
    /// Top-to-bottom order; a group missing here (say one added after the layout was saved)
    /// goes just below the nearest group that precedes it by default (or last).
    pub order: Vec<Group>,
    /// Heights the user dragged groups to (points, tab strip included). Unset = default.
    pub heights: BTreeMap<Group, f32>,
    /// Groups collapsed to their tab strip.
    pub collapsed: Vec<Group>,
    /// Names of tabs hidden by Close (not positions: Color/Swatches swap with the theme).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden_tabs: BTreeMap<Group, Vec<String>>,
    /// Floating groups and their frames `[x, y, width, height]` (screen points), sanitised on
    /// use by [`DockLayout::float_rect`].
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub floating: BTreeMap<Group, [f32; 4]>,
}

/// Gap between groups; it is also the splitter's grab area.
pub const GAP: f32 = 6.0;
/// Upper bound on a stored height (guards against absurd values from `ui.set`).
const MAX_HEIGHT: f32 = 4000.0;
/// Width range of a floating group.
pub const FLOAT_WIDTH: std::ops::RangeInclusive<f32> = 220.0..=900.0;
/// How far left of the column a tab strip must be dropped to float its group.
const FLOAT_OUT: f32 = 48.0;
/// Part of a floating group's tab strip kept on screen, so it can always be dragged back.
const FLOAT_GRIP: f32 = 60.0;

impl DockLayout {
    /// Visible tabs, retaining their original indices for commands and panel bodies.
    pub fn visible_tabs(&self, group: Group, pro: bool) -> Vec<(usize, &'static str)> {
        let hidden = self.hidden_tabs.get(&group);
        group.tabs(pro).iter().copied().enumerate().filter(|(_, name)| !hidden.is_some_and(|xs| xs.iter().any(|x| x.as_str() == *name))).collect()
    }

    /// Hide one tab; it can be reopened through Window › Panel.
    pub fn hide_tab(&mut self, group: Group, tab: usize, pro: bool) {
        let Some(&name) = group.tabs(pro).get(tab) else { return };
        let hidden = self.hidden_tabs.entry(group).or_default();
        if !hidden.iter().any(|x| x.as_str() == name) {
            hidden.push(name.to_owned());
        }
    }

    /// Restore the tab requested by the Window menu or icon rail.
    pub fn show_tab(&mut self, group: Group, tab: usize, pro: bool) {
        let Some(&name) = group.tabs(pro).get(tab) else { return };
        if let Some(hidden) = self.hidden_tabs.get_mut(&group) {
            hidden.retain(|n| n != name);
            if hidden.is_empty() {
                self.hidden_tabs.remove(&group);
            }
        }
    }

    /// Every group once, in display order.
    pub fn order(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::with_capacity(Group::ALL.len());
        for g in &self.order {
            if !out.contains(g) {
                out.push(*g);
            }
        }
        // Missing groups slot in below their default predecessor, so a group new to an old
        // saved layout (Character) never lands below Layers and takes over as the filler.
        for (i, g) in Group::ALL.iter().enumerate() {
            if out.contains(g) {
                continue;
            }
            let prev = Group::ALL.iter().take(i).rev().find_map(|p| out.iter().position(|x| x == p));
            match prev {
                Some(at) => out.insert(at + 1, *g),
                None => out.push(*g),
            }
        }
        out
    }

    pub fn is_collapsed(&self, g: Group) -> bool {
        self.collapsed.contains(&g)
    }

    pub fn set_collapsed(&mut self, g: Group, on: bool) {
        self.collapsed.retain(|c| *c != g);
        if on {
            self.collapsed.push(g);
        }
    }

    /// The stored (or default) height, sanitised.
    pub fn height(&self, g: Group) -> f32 {
        match self.heights.get(&g) {
            Some(h) if h.is_finite() => h.clamp(g.min_height(), MAX_HEIGHT),
            _ => g.default_height(),
        }
    }

    pub fn is_floating(&self, g: Group) -> bool {
        self.floating.contains_key(&g)
    }

    /// Float `g` in `frame` (screen points). Non-finite frames are ignored.
    pub fn float(&mut self, g: Group, frame: Rect) {
        let v = [frame.left(), frame.top(), frame.width(), frame.height()];
        if v.iter().all(|x| x.is_finite()) {
            self.floating.insert(g, v);
        }
    }

    /// Dock `g` back into the column (its place in [`Self::order`] is kept).
    pub fn dock(&mut self, g: Group) {
        self.floating.remove(&g);
    }

    /// A floating group's frame on `screen`: size clamped to sane bounds, position kept so the
    /// tab strip stays reachable. `None` when `g` is docked.
    pub fn float_rect(&self, g: Group, screen: Rect) -> Option<Rect> {
        let [x, y, w, h] = *self.floating.get(&g)?;
        let w = if w.is_finite() { w.clamp(*FLOAT_WIDTH.start(), *FLOAT_WIDTH.end()) } else { 300.0 };
        let h = if h.is_finite() { h.clamp(g.min_height(), MAX_HEIGHT) } else { g.default_height() };
        let x = if x.is_finite() { x } else { screen.center().x - w / 2.0 };
        let y = if y.is_finite() { y } else { screen.top() };
        let (x_lo, y_lo) = (screen.left() + FLOAT_GRIP - w, screen.top());
        let x = x.max(x_lo).min((screen.right() - FLOAT_GRIP).max(x_lo));
        let y = y.max(y_lo).min((screen.bottom() - 40.0).max(y_lo));
        Some(Rect::from_min_size(pos2(x, y), vec2(w, h)))
    }

    /// Move `g` so it is drawn just before `before` (or last when `None`).
    pub fn move_group(&mut self, g: Group, before: Option<Group>) {
        if before == Some(g) {
            return;
        }
        let mut order = self.order();
        order.retain(|x| *x != g);
        let at = before.and_then(|b| order.iter().position(|x| *x == b)).unwrap_or(order.len());
        order.insert(at, g);
        self.order = order;
    }

    /// Lay out the `shown` groups (in display order) in a column `avail` points tall with
    /// `strip`-high tab strips. Returns each group's height. The last expanded group fills the
    /// rest. Groups the user never resized give way first, down to their compact heights, so
    /// the filler gets its preferred height (Layers: ~10 rows, #147); when the column is still
    /// too short every group gives way down to its minimum height.
    pub fn heights_for(&self, shown: &[Group], avail: f32, strip: f32) -> Vec<(Group, f32)> {
        let avail = if avail.is_finite() { avail.max(0.0) } else { 0.0 };
        let filler = shown.iter().rposition(|g| !self.is_collapsed(*g));
        let mut hs: Vec<f32> = shown
            .iter()
            .enumerate()
            .map(|(i, g)| {
                if self.is_collapsed(*g) {
                    strip
                } else if Some(i) == filler {
                    0.0
                } else {
                    self.height(*g)
                }
            })
            .collect();
        if let Some(f) = filler {
            let gaps = GAP * shown.len().saturating_sub(1) as f32;
            let min_fill = shown.get(f).map_or(0.0, |g| g.min_height());
            let pref_fill = shown.get(f).map_or(0.0, |g| g.preferred_fill());
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + pref_fill - avail).max(0.0);
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(g) = shown.get(i) else { continue };
                if self.is_collapsed(*g) || self.heights.contains_key(g) {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - g.compact_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + min_fill - avail).max(0.0);
            // Squeeze the expanded groups nearest the filler first.
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(g) = shown.get(i) else { continue };
                if self.is_collapsed(*g) {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - g.min_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let rest = avail - hs.iter().sum::<f32>() - gaps;
            if let Some(h) = hs.get_mut(f) {
                *h = rest.max(min_fill);
            }
        }
        shown.iter().copied().zip(hs).collect()
    }
}

/// Show `g` and expand it (Window › <panel>, the icon rail): a panel asked for is always
/// brought back, whatever state it was left in (#129).
pub fn reveal(app: &mut PhotocraftApp, g: Group) {
    *g.shown_mut(&mut app.ui.panels) = true;
    let selected = *g.tab_mut(&mut app.ui.dock_tabs);
    let pro = crate::theme::Tokens::for_kind(app.ui.theme).pro;
    app.ui.dock.show_tab(g, selected, pro);
    app.ui.dock.set_collapsed(g, false);
}

/// Icon rail click: a hidden group is shown, a collapsed one expanded and an expanded one
/// collapsed to its tab strip. A docked group is never hidden from the rail (it used to
/// toggle visibility, so one stray click made a panel vanish: #129); `docked` is false for
/// Studio's floating Properties card, which the rail shows and hides.
pub fn rail_click(app: &mut PhotocraftApp, g: Group, docked: bool) {
    if !g.shown(&app.ui.panels) {
        reveal(app, g);
    } else if !docked {
        *g.shown_mut(&mut app.ui.panels) = false;
    } else {
        let collapse = !app.ui.dock.is_collapsed(g);
        app.ui.dock.set_collapsed(g, collapse);
        if !collapse && !app.ui.dock.is_floating(g) {
            // Opened from its icon: Auto-Collapse Iconic Panels may fold it back.
            app.ui.rail_opened.retain(|x| *x != g);
            app.ui.rail_opened.push(g);
        }
    }
}

/// Preferences › Workspace › Auto-Collapse Iconic Panels: a group expanded from its rail icon
/// collapses again at the next click anywhere outside the dock column, its floating groups and
/// the rail (on the canvas, say), as Photoshop does. Groups the person expanded any other way stay
/// open. Call before the dock is drawn.
pub fn auto_collapse(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.rail_opened.is_empty() {
        return;
    }
    if !app.session.prefs().workspace.auto_collapse_icon_panels {
        app.ui.rail_opened.clear();
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten()) else { return };
    let inside = column(ctx).into_iter().chain(rail(ctx)).chain(last_floating(ctx).into_iter().map(|(_, r)| r)).any(|r| r.contains(p));
    if inside {
        return;
    }
    for g in std::mem::take(&mut app.ui.rail_opened) {
        if !app.ui.dock.is_floating(g) {
            app.ui.dock.set_collapsed(g, true);
        }
    }
}

fn rail_id() -> egui::Id {
    egui::Id::new("dock-rail-rect")
}

/// Record where the icon rail is drawn this frame (screen points).
pub fn set_rail(ctx: &egui::Context, r: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(rail_id(), (frame, r)));
}

fn rail(ctx: &egui::Context) -> Option<Rect> {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Rect)>(rail_id())).filter(|(f, _)| now.saturating_sub(*f) <= 1).map(|(_, r)| r)
}

/// Per-group interactions collected while drawing, applied afterwards.
enum Action {
    ToggleCollapse(Group),
    Close(Group),
    CloseTab(Group, usize),
    Move(Group, Option<Group>),
    /// Float the group in this frame (screen points).
    Float(Group, Rect),
    /// Dock a floating group just before another (or last).
    Dock(Group, Option<Group>),
}

/// Rects of the groups drawn last frame (screen points), for tests and automation.
pub fn last_rects(ctx: &egui::Context) -> Vec<(Group, Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(Group, Rect)>>(rects_id())).unwrap_or_default()
}

fn rects_id() -> egui::Id {
    egui::Id::new("dock-group-rects")
}

/// A group's tab strip as drawn last frame (screen points), for tests and automation.
#[derive(Clone, Debug, PartialEq)]
pub struct StripRects {
    pub group: Group,
    /// `(tab index, rect)` of the tabs on the strip (the others are in the chevron menu).
    pub tabs: Vec<(usize, Rect)>,
    /// The panel menu button.
    pub menu: Rect,
    /// The » overflow button, when some tabs didn't fit.
    pub chevron: Option<Rect>,
}

/// The tab strips drawn last frame.
pub fn last_strips(ctx: &egui::Context) -> Vec<StripRects> {
    let mut v = ctx.data(|d| d.get_temp::<Vec<StripRects>>(strips_id())).unwrap_or_default();
    v.extend(ctx.data(|d| d.get_temp::<Vec<StripRects>>(float_strips_id())).unwrap_or_default());
    v
}

fn strips_id() -> egui::Id {
    egui::Id::new("dock-strip-rects")
}

/// Draw the `shown` groups (any order; the layout decides) filling `ui`. `body` draws one
/// group's tab content.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, shown: &[Group], mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui, Group, usize)) {
    let t = Tokens::get(ui.ctx());
    let strip = if t.pro { 28.0 } else { 40.0 };
    let order: Vec<Group> = app.ui.dock.order().into_iter().filter(|g| shown.contains(g)).collect();
    let area = ui.available_rect_before_wrap();
    let heights = app.ui.dock.heights_for(&order, area.height(), strip);
    let locked = app.session.prefs().workspace_locked;
    let rects = rects_after_layout(&heights, area);
    let mut actions: Vec<Action> = Vec::new();
    let mut dragging: Option<Group> = None;
    let mut strips: Vec<StripRects> = Vec::with_capacity(rects.len());
    for (i, (g, rect)) in rects.iter().copied().enumerate() {
        let collapsed = app.ui.dock.is_collapsed(g);
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("dock-group", g.key())).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = if t.pro { 0.0 } else { 6.0 };
        let cx = GroupCtx { order: &order, locked, floating: None };
        let Some(strip_resp) = draw_group(app, &mut child, g, &cx, &mut actions, &mut strips, &mut body) else { continue };
        let out = |p: Pos2| p.x < area.left() - FLOAT_OUT;
        if !locked && strip_resp.dragged() {
            dragging = Some(g);
        }
        if !locked
            && strip_resp.drag_stopped()
            && let Some(p) = ui.ctx().pointer_interact_pos()
        {
            // Dropped left of the column: the group floats where it was let go.
            if out(p) {
                let size = vec2(area.width(), if collapsed { g.default_height() } else { rect.height() });
                actions.push(Action::Float(g, Rect::from_min_size(p - vec2(size.x * 0.5, 14.0), size)));
            } else {
                actions.push(Action::Move(g, drop_before(&order, &rects, g, p.y)));
            }
        }
        if dragging == Some(g) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // Splitter in the gap below this group: resizes it against the next expanded group.
        if i + 1 < rects.len() && !collapsed && rects.iter().skip(i + 1).any(|(n, _)| !app.ui.dock.is_collapsed(*n)) {
            let gap = Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(rect.width(), GAP)).expand2(vec2(0.0, 2.0));
            let sresp = ui.interact(gap, ui.id().with(("dock-splitter", g.key())), Sense::drag());
            if sresp.hovered() || sresp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                ui.painter().line_segment([gap.left_center(), gap.right_center()], Stroke::new(2.0, t.accent.gamma_multiply(0.7)));
            }
            if sresp.dragged() {
                resize(&mut app.ui.dock, &heights, i, sresp.drag_delta().y);
            }
        }
    }
    // Drop indicator while a group is dragged by its tab strip.
    if let (Some(g), Some(p)) = (dragging, ui.ctx().pointer_interact_pos())
        && p.x < area.left() - FLOAT_OUT
    {
        // Outside the column: preview the floating frame.
        let h = rects.iter().find(|(x, _)| *x == g).map_or(g.default_height(), |(_, r)| r.height());
        let preview = Rect::from_min_size(p - vec2(area.width() * 0.5, 14.0), vec2(area.width(), h));
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dock-float-preview")));
        painter.rect(preview, t.radius_lg, t.accent_soft.gamma_multiply(0.5), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    } else if let (Some(g), Some(p)) = (dragging, ui.ctx().pointer_interact_pos()) {
        let before = drop_before(&order, &rects, g, p.y);
        let line_y = match before.and_then(|b| rects.iter().find(|(x, _)| *x == b)) {
            Some((_, r)) => r.top() - GAP / 2.0,
            None => rects.last().map_or(area.top(), |(_, r)| r.bottom() + GAP / 2.0),
        };
        ui.painter().line_segment([pos2(area.left(), line_y), pos2(area.right(), line_y)], Stroke::new(3.0, t.accent));
    }
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|d| {
        d.insert_temp(rects_id(), rects);
        d.insert_temp(strips_id(), strips);
        d.insert_temp(column_id(), (frame, area));
    });
    ui.advance_cursor_after_rect(area);
    apply_actions(app, actions, t.pro);
}

/// Applies the interactions collected while drawing groups (docked or floating).
fn apply_actions(app: &mut PhotocraftApp, actions: Vec<Action>, pro: bool) {
    let t = DockTheme { pro };
    for a in actions {
        match a {
            Action::ToggleCollapse(g) => {
                let on = !app.ui.dock.is_collapsed(g);
                app.ui.dock.set_collapsed(g, on);
            }
            Action::Close(g) => *g.shown_mut(&mut app.ui.panels) = false,
            Action::CloseTab(g, tab) => {
                app.ui.dock.hide_tab(g, tab, t.pro);
                let remaining = app.ui.dock.visible_tabs(g, t.pro);
                if remaining.is_empty() {
                    *g.shown_mut(&mut app.ui.panels) = false;
                } else if *g.tab_mut(&mut app.ui.dock_tabs) == tab {
                    // Prefer the next tab, or the previous one when closing the last.
                    let next = remaining.iter().find(|(i, _)| *i > tab).or_else(|| remaining.last());
                    if let Some(&(next, _)) = next {
                        *g.tab_mut(&mut app.ui.dock_tabs) = next;
                    }
                }
            }
            Action::Move(g, before) => {
                if before != Some(g) {
                    app.ui.dock.move_group(g, before);
                }
            }
            Action::Float(g, frame) => app.ui.dock.float(g, frame),
            Action::Dock(g, before) => {
                app.ui.dock.dock(g);
                if before != Some(g) {
                    app.ui.dock.move_group(g, before);
                }
            }
        }
    }
}

/// What [`apply_actions`] needs of the theme.
struct DockTheme {
    pro: bool,
}

fn column_id() -> egui::Id {
    egui::Id::new("dock-column-rect")
}

/// The dock column drawn this frame or the last one (screen points), where a floating group
/// docks when dropped.
fn column(ctx: &egui::Context) -> Option<Rect> {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Rect)>(column_id())).filter(|(f, _)| now.saturating_sub(*f) <= 1).map(|(_, r)| r)
}

/// Where a group is drawn: the column's ordering, Lock Workspace, and the frame of a floating
/// group (`None` = docked).
struct GroupCtx<'a> {
    order: &'a [Group],
    locked: bool,
    floating: Option<Rect>,
}

/// Draws group `g`'s card (tab strip, tab body and panel menu) filling `ui`, collecting its
/// interactions. Returns the tab strip's response for moving the group, `None` when it has no
/// visible tabs.
fn draw_group(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    g: Group,
    cx: &GroupCtx,
    actions: &mut Vec<Action>,
    strips: &mut Vec<StripRects>,
    body: &mut dyn FnMut(&mut PhotocraftApp, &mut egui::Ui, Group, usize),
) -> Option<egui::Response> {
    let t = Tokens::get(ui.ctx());
    let (order, locked) = (cx.order, cx.locked);
    let collapsed = app.ui.dock.is_collapsed(g);
    let before = *g.tab_mut(&mut app.ui.dock_tabs);
    let visible = app.ui.dock.visible_tabs(g, t.pro);
    if visible.is_empty() {
        return None;
    }
    let indices: Vec<usize> = visible.iter().map(|(i, _)| *i).collect();
    let tabs: Vec<&str> = visible.iter().map(|(_, name)| *name).collect();
    let mut sel = indices.iter().position(|i| *i == before).unwrap_or(0);
    let resp = widgets::card_ex(ui, g.key(), &tabs, &mut sel, collapsed, |ui, shown_tab| {
        let Some(&tab) = indices.get(shown_tab) else { return };
        let inner = ui.available_height().max(0.0);
        if g.scrolls_itself(tab, t.pro) {
            ui.set_min_height(inner);
            body(app, ui, g, tab);
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("dock-scroll", g.key(), tab))
                .max_height(inner)
                .auto_shrink([false, false])
                .show(ui, |ui| body(app, ui, g, tab));
        }
    });
    strips.push(StripRects {
        group: g,
        tabs: resp.tabs.iter().filter_map(|(i, r)| indices.get(*i).map(|original| (*original, *r))).collect(),
        menu: resp.menu.rect,
        chevron: resp.chevron,
    });
    // The strip uses visible indices; dockTabs and the panel bodies use original indices.
    if let Some(&picked) = indices.get(sel)
        && picked != before
    {
        *g.tab_mut(&mut app.ui.dock_tabs) = picked;
    }
    if let Some(context) = resp.tab_context {
        match context {
            crate::tab_strip::TabContextAction::Close(i) => {
                if let Some(&tab) = indices.get(i) {
                    actions.push(Action::CloseTab(g, tab));
                }
            }
            crate::tab_strip::TabContextAction::CloseGroup => actions.push(Action::Close(g)),
        }
    }
    if resp.strip.double_clicked() || resp.tab_double_clicked || (collapsed && resp.tab_clicked) {
        actions.push(Action::ToggleCollapse(g));
    }
    egui::Popup::menu(&resp.menu).show(|ui| {
        ui.set_min_width(170.0);
        if tabs.get(sel) == Some(&"Layers") {
            crate::layer_row_ui::panel_menu(app, ui);
            ui.separator();
        }
        if tabs.get(sel) == Some(&"Swatches") {
            crate::swatches_ui::panel_menu(app, ui);
            ui.separator();
        }
        if ui.button(if collapsed { tl!("Expand Panel Group") } else { tl!("Collapse Panel Group") }).clicked() {
            actions.push(Action::ToggleCollapse(g));
            ui.close();
        }
        let pos = order.iter().position(|x| *x == g).unwrap_or(0);
        if ui.add_enabled(!locked && pos > 0, egui::Button::new(tl!("Move Group Up"))).clicked() {
            actions.push(Action::Move(g, order.get(pos.saturating_sub(1)).copied()));
            ui.close();
        }
        if ui.add_enabled(!locked && pos + 1 < order.len(), egui::Button::new(tl!("Move Group Down"))).clicked() {
            actions.push(Action::Move(g, order.get(pos + 2).copied()));
            ui.close();
        }
        match cx.floating {
            Some(frame) => {
                if ui.add_enabled(!locked, egui::Button::new(tl!("Dock Panel Group"))).clicked() {
                    actions.push(Action::Dock(g, None));
                    ui.close();
                }
                let _ = frame;
            }
            None => {
                if ui.add_enabled(!locked, egui::Button::new(tl!("Float Panel Group"))).clicked() {
                    let r = ui.ctx().data(|d| d.get_temp::<Vec<(Group, Rect)>>(rects_id())).and_then(|v| v.into_iter().find(|(x, _)| *x == g)).map(|(_, r)| r);
                    let r = r.unwrap_or_else(|| Rect::from_min_size(pos2(200.0, 120.0), vec2(300.0, g.default_height())));
                    // Just left of where it was docked.
                    actions.push(Action::Float(g, r.translate(vec2(-(r.width() + 24.0), 0.0))));
                    ui.close();
                }
            }
        }
        ui.separator();
        if ui.button(tl!("Close Tab Group")).clicked() {
            actions.push(Action::Close(g));
            ui.close();
        }
    });
    Some(resp.strip)
}

fn floats_id() -> egui::Id {
    egui::Id::new("dock-floating-rects")
}

/// Frames of the floating groups drawn last frame (screen points), for tests and automation.
pub fn last_floating(ctx: &egui::Context) -> Vec<(Group, Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(Group, Rect)>>(floats_id())).unwrap_or_default()
}

/// Draw the floating groups among `shown` over the workspace. Drag a tab strip to move one
/// (onto the dock column to dock it), the bottom-right corner to resize it.
pub fn show_floating(app: &mut PhotocraftApp, ctx: &egui::Context, shown: &[Group], mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui, Group, usize)) {
    let groups: Vec<Group> = app.ui.dock.floating.keys().copied().filter(|g| shown.contains(g)).collect();
    if groups.is_empty() {
        ctx.data_mut(|d| d.remove_temp::<Vec<(Group, Rect)>>(floats_id()));
        return;
    }
    let t = Tokens::get(ctx);
    let strip = if t.pro { 28.0 } else { 40.0 };
    let margin: i8 = if t.pro { 2 } else { 8 };
    let screen = ctx.content_rect();
    let locked = app.session.prefs().workspace_locked;
    let docked: Vec<Group> = app.ui.dock.order().into_iter().filter(|g| !app.ui.dock.is_floating(*g)).collect();
    let docked_rects = last_rects(ctx);
    let column = column(ctx);
    let mut actions = Vec::new();
    let mut strips = Vec::new();
    let mut frames = Vec::new();
    for g in groups {
        let Some(frame) = app.ui.dock.float_rect(g, screen) else { continue };
        let collapsed = app.ui.dock.is_collapsed(g);
        let inner = vec2(frame.width(), if collapsed { strip } else { frame.height() }) - vec2(2.0, 2.0) * f32::from(margin);
        let id = egui::Id::new(("dock-float", g.key()));
        let mut strip_resp = None;
        let mut grip_resp = None;
        let shown_area = egui::Area::new(id).order(egui::Order::Middle).movable(false).constrain(false).fixed_pos(frame.min).show(ctx, |ui| {
            let fr = egui::Frame::NONE
                .fill(t.dock)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(egui::CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(margin))
                .show(ui, |ui| {
                    let (r, _) = ui.allocate_exact_size(inner.max(vec2(0.0, 0.0)), Sense::hover());
                    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("dock-float-group", g.key())).max_rect(r));
                    child.set_clip_rect(r);
                    child.spacing_mut().item_spacing.y = if t.pro { 0.0 } else { 6.0 };
                    let cx = GroupCtx { order: &docked, locked, floating: Some(frame) };
                    strip_resp = draw_group(app, &mut child, g, &cx, &mut actions, &mut strips, &mut body);
                });
            // Resize from the bottom-right corner.
            if !collapsed && !locked {
                let outer = fr.response.rect;
                let grip = Rect::from_min_max(outer.right_bottom() - vec2(14.0, 14.0), outer.right_bottom());
                for k in [5.0, 9.0] {
                    ui.painter().line_segment([grip.right_bottom() - vec2(k, 3.0), grip.right_bottom() - vec2(3.0, k)], Stroke::new(1.0, t.text_faint));
                }
                grip_resp = Some(ui.interact(grip, id.with("grip"), Sense::drag()));
            }
        });
        frames.push((g, shown_area.response.rect));
        if let Some(gr) = grip_resp {
            if gr.hovered() || gr.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            let d = gr.drag_delta();
            if d != egui::Vec2::ZERO {
                app.ui.dock.float(g, Rect::from_min_size(frame.min, frame.size() + d));
            }
        }
        let Some(sr) = strip_resp else { continue };
        if locked {
            continue;
        }
        if sr.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            let d = sr.drag_delta();
            if d != egui::Vec2::ZERO {
                app.ui.dock.float(g, frame.translate(d));
            }
        }
        if sr.drag_stopped()
            && let Some(p) = ctx.pointer_interact_pos()
        {
            // Dropped on the dock column (or the window's right edge when every group floats).
            let over = column.map_or(p.x >= screen.right() - FLOAT_GRIP, |c| c.expand2(vec2(FLOAT_OUT * 0.5, 0.0)).contains(p));
            if over {
                actions.push(Action::Dock(g, drop_before(&docked, &docked_rects, g, p.y)));
            }
        }
    }
    ctx.data_mut(|d| {
        d.insert_temp(floats_id(), frames);
        d.insert_temp(float_strips_id(), strips);
    });
    apply_actions(app, actions, t.pro);
}

fn float_strips_id() -> egui::Id {
    egui::Id::new("dock-floating-strips")
}

fn rects_after_layout(heights: &[(Group, f32)], area: Rect) -> Vec<(Group, Rect)> {
    let mut y = area.top();
    heights
        .iter()
        .map(|(g, h)| {
            let r = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h.max(0.0)));
            y = r.bottom() + GAP;
            (*g, r)
        })
        .collect()
}

/// The group the dragged one lands before when released at `y` (`None` = last).
fn drop_before(order: &[Group], rects: &[(Group, Rect)], dragged: Group, y: f32) -> Option<Group> {
    let hit = rects.iter().find(|(_, r)| y < r.center().y).map(|(g, _)| *g);
    match hit {
        // Dropping onto itself, or just below itself, keeps the place.
        Some(g) if g == dragged => Some(dragged),
        Some(g) => {
            let after_self = order.iter().position(|x| *x == dragged).zip(order.iter().position(|x| *x == g)).is_some_and(|(a, b)| b == a + 1);
            if after_self { Some(dragged) } else { Some(g) }
        }
        None => None,
    }
}

/// Splitter `i` (below group `i`) dragged by `dy`: group `i` grows or shrinks against the next
/// expanded group (or the filler, which absorbs the difference).
fn resize(layout: &mut DockLayout, heights: &[(Group, f32)], i: usize, dy: f32) {
    if !dy.is_finite() || dy == 0.0 {
        return;
    }
    let Some(&(g, h)) = heights.get(i) else { return };
    let filler = heights.iter().rposition(|(g, _)| !layout.is_collapsed(*g));
    let Some(j) = heights.iter().enumerate().skip(i + 1).find(|(_, (n, _))| !layout.is_collapsed(*n)).map(|(j, _)| j) else { return };
    let Some(&(n, nh)) = heights.get(j) else { return };
    // The first drag pins the other groups at the heights they show, so groups still at their
    // defaults (which give way to the filler) don't shift while this one is resized.
    for (k, (o, oh)) in heights.iter().enumerate() {
        if Some(k) != filler && !layout.is_collapsed(*o) {
            layout.heights.entry(*o).or_insert(*oh);
        }
    }
    let new_h = (h + dy).clamp(g.min_height(), (h + nh - n.min_height()).max(g.min_height()));
    layout.heights.insert(g, new_h);
    if Some(j) != filler {
        layout.heights.insert(n, (nh - (new_h - h)).max(n.min_height()));
    }
}

/// What `prefs.panelLayout` holds: the live layout and open panels.
fn snapshot(app: &PhotocraftApp) -> Value {
    json!({"workspace": app.ui.workspace, "panels": app.ui.panels, "dockTabs": app.ui.dock_tabs, "dock": app.ui.dock, "timelineOpen": app.ui.timeline.open})
}

/// Remember the layout in the preferences once the user lets go of the mouse (Workspace ›
/// Remember Workspace Changes). Cheap: a small JSON compare per frame.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.session.prefs().workspace.remember_workspace_changes || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().panel_layout != now {
        app.session.prefs.edit(|p| p.panel_layout = now);
    }
}

/// Restore the remembered layout at launch. Unreadable parts keep their defaults.
pub fn restore(app: &mut PhotocraftApp) {
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let saved = app.session.prefs().panel_layout.clone();
    apply(app, &saved);
    if let Some(ws) = saved.get("workspace").and_then(Value::as_str) {
        app.ui.workspace = ws.to_string();
    }
}

/// Apply the `panels`, `dockTabs`, `dock` and `timelineOpen` parts of a saved layout
/// (a workspace or `panelLayout`). Missing or invalid dock parts are left alone; old
/// layouts without Timeline visibility restore it closed.
pub fn apply(app: &mut PhotocraftApp, v: &Value) {
    if let Some(p) = v.get("panels").and_then(|p| serde_json::from_value(p.clone()).ok()) {
        app.ui.panels = p;
    }
    if let Some(t) = v.get("dockTabs").and_then(|t| serde_json::from_value(t.clone()).ok()) {
        app.ui.dock_tabs = t;
    }
    if let Some(d) = v.get("dock").and_then(|d| serde_json::from_value(d.clone()).ok()) {
        app.ui.dock = d;
    }
    app.ui.timeline.open = v.get("timelineOpen").and_then(Value::as_bool).unwrap_or(false);
    if !app.ui.timeline.open {
        app.ui.timeline.playing = false;
    }
}

#[cfg(test)]
#[path = "dock_tests.rs"]
mod tests;
