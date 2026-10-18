//! Preferences › Tools › Enable Flick Panning: a Hand-tool drag let go while still moving keeps
//! the view gliding in that direction, slowing to a stop. Any press stops it.

use egui::{Context, Id, Vec2};

/// Below this release speed (screen points per second) a drag just stops.
pub const MIN_SPEED: f32 = 150.0;
/// How quickly a glide slows: its speed falls by e every 1/`DECAY` seconds.
const DECAY: f32 = 4.0;
/// A glide never starts faster than this, whatever a hostile frame time says.
const MAX_SPEED: f32 = 8000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Flick {
    /// Screen points per second.
    velocity: Vec2,
    gliding: bool,
}

fn dt(ctx: &Context) -> f32 {
    let dt = ctx.input(|i| i.stable_dt);
    if dt.is_finite() { dt.clamp(1.0 / 240.0, 0.25) } else { 1.0 / 60.0 }
}

fn get(ctx: &Context, id: Id) -> Flick {
    ctx.data(|d| d.get_temp::<Flick>(id)).unwrap_or_default()
}

/// While the Hand drags the view: follow how fast it moves (`delta` screen points this frame).
pub fn track(ctx: &Context, id: Id, delta: Vec2) {
    let now = delta / dt(ctx);
    let prev = get(ctx, id);
    let velocity = if prev.gliding { now } else { prev.velocity * 0.5 + now * 0.5 };
    ctx.data_mut(|d| d.insert_temp(id, Flick { velocity, gliding: false }));
}

/// The Hand drag ended: glide on when `enabled` and it was let go fast enough.
pub fn release(ctx: &Context, id: Id, enabled: bool) {
    let v = get(ctx, id).velocity;
    let speed = v.length();
    if !enabled || !speed.is_finite() || speed < MIN_SPEED {
        ctx.data_mut(|d| d.remove::<Flick>(id));
        return;
    }
    let velocity = if speed > MAX_SPEED { v * (MAX_SPEED / speed) } else { v };
    ctx.data_mut(|d| d.insert_temp(id, Flick { velocity, gliding: true }));
    ctx.request_repaint();
}

/// This frame's glide in screen points (pan by it as by a drag), or `None` when there is none.
/// A press anywhere ends the glide.
pub fn step(ctx: &Context, id: Id) -> Option<Vec2> {
    let f = get(ctx, id);
    if !f.gliding {
        return None;
    }
    if ctx.input(|i| i.pointer.any_pressed()) {
        ctx.data_mut(|d| d.remove::<Flick>(id));
        return None;
    }
    let dt = dt(ctx);
    let velocity = f.velocity * (-DECAY * dt).exp();
    if velocity.length() < MIN_SPEED / 4.0 {
        ctx.data_mut(|d| d.remove::<Flick>(id));
        return None;
    }
    ctx.data_mut(|d| d.insert_temp(id, Flick { velocity, gliding: true }));
    ctx.request_repaint();
    Some(velocity * dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of `ctx` (optionally with a press) and returns `f`'s result.
    fn frame<T>(ctx: &Context, press: bool, f: impl FnOnce(&Context) -> T) -> T {
        let mut input = egui::RawInput { predicted_dt: 1.0 / 60.0, ..Default::default() };
        if press {
            input.events.push(egui::Event::PointerButton {
                pos: egui::pos2(5.0, 5.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        ctx.begin_pass(input);
        let out = f(ctx);
        ctx.end_pass().textures_delta.clear();
        out
    }

    #[test]
    fn a_fast_release_glides_to_a_stop_and_a_slow_one_does_not() {
        let id = Id::new("flick");
        let ctx = Context::default();
        for _ in 0..4 {
            frame(&ctx, false, |c| track(c, id, egui::vec2(30.0, 0.0)));
        }
        frame(&ctx, false, |c| release(c, id, true));
        let glide: Vec<f32> = std::iter::from_fn(|| frame(&ctx, false, |c| step(c, id)).map(|d| d.x)).take(1000).collect();
        assert!(glide.len() > 5 && glide.len() < 1000, "glides for a while, then stops: {}", glide.len());
        assert!(glide.windows(2).all(|w| w[1] < w[0] && w[1] > 0.0), "keeps going the same way, slowing");
        // Off, or let go slowly: no glide.
        for (delta, on) in [(30.0, false), (1.0, true)] {
            for _ in 0..4 {
                frame(&ctx, false, |c| track(c, id, egui::vec2(delta, 0.0)));
            }
            frame(&ctx, false, |c| release(c, id, on));
            assert_eq!(frame(&ctx, false, |c| step(c, id)), None, "delta {delta}, on {on}");
        }
        // A press stops a glide.
        for _ in 0..4 {
            frame(&ctx, false, |c| track(c, id, egui::vec2(0.0, -40.0)));
        }
        frame(&ctx, false, |c| release(c, id, true));
        assert!(frame(&ctx, false, |c| step(c, id)).is_some());
        assert_eq!(frame(&ctx, true, |c| step(c, id)), None);
        assert_eq!(frame(&ctx, false, |c| step(c, id)), None);
    }
}
