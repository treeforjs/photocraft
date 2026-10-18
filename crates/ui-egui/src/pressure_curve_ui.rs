//! Pressure curve editor (TOOL-213-11): a small square graph of output against pen pressure with
//! five handles at fixed pressures, each dragged up or down, and Linear / Soft / Firm presets.
//! Preferences › Tools shows it for the global curve, and Brush Settings beside every control
//! set to Pen Pressure for that control's own curve.
use egui::{Pos2, Sense, Stroke, Ui, vec2};
use photocraft_engine::paint::PressureCurve;
use photocraft_engine::paint::pressure_curve::POINTS;

use crate::theme::Tokens;

/// Handle radius (points).
const HANDLE: f32 = 4.0;

/// The graph and its presets; returns whether the curve changed.
pub fn editor(ui: &mut Ui, id: &str, curve: &mut PressureCurve, side: f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= graph(ui, id, curve, side);
        ui.vertical(|ui| {
            for (label, preset) in [(tl!("Linear"), PressureCurve::LINEAR), (tl!("Soft"), PressureCurve::SOFT), (tl!("Firm"), PressureCurve::FIRM)] {
                if ui.add(egui::Button::selectable(*curve == preset, label)).clicked() && *curve != preset {
                    *curve = preset;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// The square graph: drag a handle up or down; double-click resets to the straight line.
pub fn graph(ui: &mut Ui, id: &str, curve: &mut PressureCurve, side: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let side = if side.is_finite() { side.clamp(48.0, 400.0) } else { 96.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let label = tl!("Pressure curve").to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &label));
    let inner = rect.shrink(HANDLE + 1.0);
    let to_screen = |x: f32, y: f32| Pos2::new(inner.left() + x * inner.width(), inner.bottom() - y * inner.height());
    let p = ui.painter_at(rect.expand(1.0));
    p.rect_filled(rect, t.radius_sm, t.field);
    let grid = Stroke::new(1.0, t.separator.gamma_multiply(0.6));
    for k in 1..4 {
        let f = k as f32 / 4.0;
        p.line_segment([to_screen(f, 0.0), to_screen(f, 1.0)], grid);
        p.line_segment([to_screen(0.0, f), to_screen(1.0, f)], grid);
    }
    p.line_segment([to_screen(0.0, 0.0), to_screen(1.0, 1.0)], Stroke::new(1.0, t.text_faint.gamma_multiply(0.5)));
    // The handle being dragged lives in egui memory for the gesture.
    let drag_id = ui.make_persistent_id((id, "pressure-curve-drag"));
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let mut changed = false;
    let nearest = |pos: Pos2| {
        (0..POINTS).min_by(|a, b| {
            let d = |i: usize| (to_screen(PressureCurve::input(i), curve.output(i)) - pos).length();
            d(*a).total_cmp(&d(*b))
        })
    };
    // The handle nearest where the press began (the pointer has moved on by the time a drag starts).
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        dragging = nearest(pos);
    }
    if resp.dragged()
        && let (Some(i), Some(pos)) = (dragging, resp.interact_pointer_pos())
        && inner.height() > 0.0
    {
        let y = ((inner.bottom() - pos.y) / inner.height()).clamp(0.0, 1.0);
        // The ends stay at 0 and 1 apart from the dragged point's own range, as in a Curves editor.
        if let Some(v) = curve.0.get_mut(i)
            && (*v - y).abs() > f32::EPSILON
        {
            *v = y;
            changed = true;
        }
    }
    if resp.drag_stopped() {
        dragging = None;
    }
    ui.data_mut(|d| {
        if let Some(i) = dragging {
            d.insert_temp(drag_id, i);
        } else {
            d.remove::<usize>(drag_id);
        }
    });
    if resp.double_clicked() && *curve != PressureCurve::LINEAR {
        *curve = PressureCurve::LINEAR;
        changed = true;
    }
    // The curve, then its handles.
    let line: Vec<Pos2> = (0..=48).map(|k| k as f32 / 48.0).map(|x| to_screen(x, curve.apply(x))).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.5, t.accent)));
    for i in 0..POINTS {
        let c = to_screen(PressureCurve::input(i), curve.output(i));
        let hot = dragging == Some(i);
        p.circle(c, HANDLE, if hot { t.accent } else { t.card }, Stroke::new(1.2, if hot { t.accent_text } else { t.accent }));
    }
    if changed {
        ui.ctx().request_repaint();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn dragging_a_handle_bends_the_curve_and_presets_reset_it() {
        let mut h = Harness::builder().with_size(vec2(300.0, 160.0)).build_ui_state(
            |ui, c: &mut PressureCurve| {
                editor(ui, "test", c, 120.0);
            },
            PressureCurve::LINEAR,
        );
        h.run();
        let r = h.get_by_label("Pressure curve").rect();
        // The middle handle sits at the centre; drag it up.
        let mid = r.center();
        h.drag_at(mid);
        h.run();
        h.hover_at(mid - vec2(0.0, 30.0));
        h.run();
        h.drop_at(mid - vec2(0.0, 30.0));
        h.run();
        let c = *h.state();
        assert!(c.0[2] > 0.7, "{c:?}");
        assert_eq!((c.0[0], c.0[4]), (0.0, 1.0), "other handles stay");
        h.get_by_label("Firm").click();
        h.run();
        assert_eq!(*h.state(), PressureCurve::FIRM);
        h.get_by_label("Linear").click();
        h.run();
        assert_eq!(*h.state(), PressureCurve::LINEAR);
    }
}
