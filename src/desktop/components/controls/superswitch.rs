//! The superswitch's activation and painted emblem.
use crate::desktop::theme::superswitch_art::*;
use crate::desktop::theme::{module_color, Tokens};
use gpui_kit::*;
use std::f32::consts::{PI, TAU};
use std::time::Instant;

/// Seconds from the click to the settled, lit emblem.
pub const DURATION: f32 = 1.6;
/// The moment the three comets meet in the hourglass.
const IGNITE: f32 = 0.95;
/// Horizontal centre of the emblem, from the card's left edge. The card's text starts after it.
pub const EMBLEM_X: f32 = 46.0;
/// Room the emblem takes on the left of the card.
pub const EMBLEM_GUTTER: f32 = 94.0;
/// Radius of the hourglass ring.
const R: f32 = 14.0;
const RX: f32 = 31.0;
const RY: f32 = 10.0;
const DIAMETER_FACTOR: f32 = 2.0;
const WAVE_FADE_EXPONENT: i32 = 2;

/// What the emblem shows at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Off,
    /// Seconds into the activation.
    Igniting(f32),
    Lit,
}

/// Whether the switch is on, and since when. A new dialog starts from `default()`, which is off:
/// nothing but a click turns the council on.
#[derive(Clone, Debug, Default)]
pub struct Activation {
    on: bool,
    since: Option<Instant>,
    /// Demo only: hold the activation at this many seconds, so one frame can be captured.
    held: Option<f32>,
}

impl Activation {
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// A click. Turning on starts the activation at `now`; turning off is immediate and quiet.
    pub fn set(&mut self, on: bool, now: Instant) {
        self.on = on;
        self.since = on.then_some(now);
        self.held = None;
    }

    /// Demo only: on, frozen `seconds` into the activation.
    pub fn hold(&mut self, seconds: f32) {
        self.on = true;
        self.since = None;
        self.held = Some(seconds);
    }

    pub fn look(&self, now: Instant, reduce_motion: bool) -> Look {
        if !self.on {
            return Look::Off;
        }
        if reduce_motion {
            return Look::Lit;
        }
        let seconds = match (self.held, self.since) {
            (Some(held), _) => held,
            (None, Some(since)) => now.saturating_duration_since(since).as_secs_f32(),
            (None, None) => DURATION,
        };
        if seconds >= DURATION {
            Look::Lit
        } else {
            Look::Igniting(seconds.max(0.0))
        }
    }

    /// True only while the activation is playing. Once it settles nothing asks for another frame.
    pub fn wants_frame(&self, now: Instant, reduce_motion: bool) -> bool {
        self.held.is_none() && matches!(self.look(now, reduce_motion), Look::Igniting(_))
    }
}

/// The card's border: the usual line while off, old gold once lit.
pub fn border(look: Look, t: &Tokens) -> Hsla {
    let lit = t.gold.opacity(ALPHA_0_6);
    match look {
        Look::Off => t.border,
        Look::Lit => lit,
        Look::Igniting(s) => mix(t.border, lit, smooth(seg(s, IGNITE, 1.3))),
    }
}

/// Everything drawn behind the card's text: the emblem on the left and, while igniting, the
/// shockwave and the star field it uncovers. Fills its parent, which must be `relative`.
pub fn backdrop(look: Look, t: Tokens) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| match look {
            Look::Off => paint_off(window, bounds, &t),
            Look::Igniting(s) => paint_on(window, bounds, &t, s),
            Look::Lit => paint_on(window, bounds, &t, DURATION),
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

struct Orbit {
    tone: Hsla,
    /// Resting tilt of the ellipse, in degrees.
    tilt: f32,
    /// Where the orbit sweeps in from, in degrees, measured like screen angles.
    from: f32,
    spin: f32,
    /// Where its planet rests on the settled orbit, in radians.
    rest: f32,
}

fn orbits(t: &Tokens) -> [Orbit; 3] {
    [
        // Astra, the sky blue the app uses for running work.
        Orbit {
            tone: t.run,
            tilt: -28.0,
            from: -38.0,
            spin: 1.0,
            rest: 0.5,
        },
        // Fable, an aged rose pigment.
        Orbit {
            tone: module_color(1),
            tilt: 32.0,
            from: 8.0,
            spin: -1.0,
            rest: 2.5,
        },
        // Argon, an aged violet pigment.
        Orbit {
            tone: module_color(3),
            tilt: 92.0,
            from: 52.0,
            spin: 1.0,
            rest: 4.3,
        },
    ]
}

fn seg(s: f32, from: f32, to: f32) -> f32 {
    ((s - from) / (to - from)).clamp(0.0, 1.0)
}
fn smooth(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}
fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}
fn ease_in(x: f32) -> f32 {
    x * x * x
}
fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}
fn mix(a: Hsla, b: Hsla, x: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    Rgba {
        r: lerp(a.r, b.r, x),
        g: lerp(a.g, b.g, x),
        b: lerp(a.b, b.b, x),
        a: lerp(a.a, b.a, x),
    }
    .into()
}
/// A settle with one soft overshoot: 0 at 0, exactly 1 at 1.
fn settle(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3) * (x * TAU).cos()
}

fn p(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

fn disc(window: &mut Window, c: Point<Pixels>, r: f32, color: Hsla) {
    let bounds = Bounds {
        origin: point(c.x - px(r), c.y - px(r)),
        size: size(px(r * DIAMETER_FACTOR), px(r * DIAMETER_FACTOR)),
    };
    window.paint_quad(fill(bounds, color).corner_radii(px(r)));
}

fn glow(window: &mut Window, c: Point<Pixels>, r: f32, blur: f32, color: Hsla) {
    let bounds = Bounds {
        origin: point(c.x - px(r), c.y - px(r)),
        size: size(px(r * DIAMETER_FACTOR), px(r * DIAMETER_FACTOR)),
    };
    window.paint_drop_shadows(
        bounds,
        Corners::all(px(r)),
        &[BoxShadow {
            color,
            offset: Point::default(),
            blur_radius: px(blur),
            spread_radius: px(ZERO),
            inset: false,
        }],
    );
}

fn line(window: &mut Window, points: &[Point<Pixels>], closed: bool, width: f32, color: Hsla) {
    if color.a <= 0.003 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.add_polygon(points, closed);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn circle(window: &mut Window, c: (f32, f32), r: f32, width: f32, color: Hsla) {
    let points: Vec<_> = (0..96)
        .map(|i| {
            let a = i as f32 / 96.0 * TAU;
            p(c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect();
    line(window, &points, true, width, color);
}

/// An ellipse of the council at `scale`, tilted `tilt` degrees, centred at `c`.
#[derive(Clone, Copy)]
struct Ring {
    c: (f32, f32),
    scale: f32,
    tilt: f32,
}

impl Ring {
    fn at(&self, a: f32) -> (f32, f32) {
        let (x, y) = (RX * self.scale * a.cos(), RY * self.scale * a.sin());
        let (sin, cos) = self.tilt.to_radians().sin_cos();
        (self.c.0 + x * cos - y * sin, self.c.1 + x * sin + y * cos)
    }
    /// The half of the ellipse that passes in front of the hourglass.
    fn front(a: f32) -> bool {
        a.rem_euclid(TAU) < PI
    }
}

/// The ring and hourglass of the Chronos mark. `lit` turns the ring and sand to gold;
/// `turn` is the hourglass's rotation in radians.
fn hourglass(window: &mut Window, c: (f32, f32), t: &Tokens, lit: f32, turn: f32, presence: f32) {
    let at = |x: f32, y: f32| p(c.0 + x * R, c.1 + y * R);
    let (sin, cos) = turn.sin_cos();
    let turned = |x: f32, y: f32| at(x * cos - y * sin, x * sin + y * cos);
    let arc = |from: f32, to: f32| -> Vec<Point<Pixels>> {
        (0..=24)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / 24.0).to_radians();
                at(a.cos(), a.sin())
            })
            .collect()
    };
    let ring = mix(t.idle.opacity(ALPHA_0_7 * presence), t.gold, lit);
    line(window, &arc(-150.0, -30.0), false, 1.6, ring);
    line(window, &arc(30.0, 150.0), false, 1.6, ring);
    line(
        window,
        &arc(-30.0, 30.0),
        false,
        1.0,
        t.idle.opacity(ALPHA_0_6 * presence),
    );
    line(
        window,
        &arc(150.0, 210.0),
        false,
        1.0,
        t.idle.opacity(ALPHA_0_6 * presence),
    );
    let glass_color = mix(
        t.muted.opacity(ALPHA_0_75 * presence),
        t.text.opacity(ALPHA_0_92),
        lit,
    );
    for side in [-1.0f32, 1.0] {
        let mut b = PathBuilder::stroke(px(GLASS_STROKE));
        b.move_to(turned(side * 0.36, -0.60));
        b.cubic_bezier_to(
            turned(side * 0.07, -0.05),
            turned(side * 0.36, -0.30),
            turned(side * 0.22, -0.16),
        );
        b.line_to(turned(side * 0.07, 0.05));
        b.cubic_bezier_to(
            turned(side * 0.36, 0.60),
            turned(side * 0.22, 0.16),
            turned(side * 0.36, 0.30),
        );
        if let Ok(path) = b.build() {
            window.paint_path(path, glass_color);
        }
    }
    for y in [-0.66f32, 0.66] {
        line(
            window,
            &[turned(-0.42, y), turned(0.42, y)],
            false,
            1.4,
            mix(t.idle.opacity(ALPHA_0_8 * presence), t.gold, lit),
        );
    }
    if lit > 0.0 {
        let mut sand = PathBuilder::fill();
        sand.move_to(at(-0.23, -0.40));
        sand.curve_to(at(0.23, -0.40), at(0.0, -0.33));
        sand.curve_to(at(0.0, -0.09), at(0.14, -0.20));
        sand.curve_to(at(-0.23, -0.40), at(-0.14, -0.20));
        sand.close();
        sand.move_to(at(-0.24, 0.50));
        sand.curve_to(at(0.0, 0.20), at(-0.09, 0.37));
        sand.curve_to(at(0.24, 0.50), at(0.09, 0.37));
        sand.close();
        if let Ok(path) = sand.build() {
            window.paint_path(path, t.gold.opacity(lit));
        }
    }
}

/// Off: the hourglass alone, dim, waiting.
fn paint_off(window: &mut Window, bounds: Bounds<Pixels>, t: &Tokens) {
    let c = (
        f32::from(bounds.origin.x) + EMBLEM_X,
        f32::from(bounds.center().y),
    );
    hourglass(window, c, t, 0.0, 0.0, 1.0);
}

fn hash(n: u32) -> f32 {
    ((n as f32 * 12.9898).sin() * 43_758.547).fract().abs()
}

/// One frame of the activation, `s` seconds in. `paint_on(.., DURATION)` is the settled look,
/// and every passing effect has faded to nothing by then, so settling never jumps.
fn paint_on(window: &mut Window, bounds: Bounds<Pixels>, t: &Tokens, s: f32) {
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let c = (left + EMBLEM_X, top + h / 2.0);
    let center = p(c.0, c.1);
    let after = seg(s, IGNITE, DURATION);
    let fading = 1.0 - smooth(seg(s, 1.15, DURATION));

    // The shockwave runs out across the card and leaves a still star chart behind it.
    let wave = ease_out(seg(s, IGNITE, 1.55));
    let wave_r = 10.0 + (w + 40.0) * wave;
    // Stars stay out of the text, so the settled card reads as easily as before.
    let text_zone = |x: f32, y: f32| {
        x > left + EMBLEM_GUTTER - 6.0 && x < left + w - 84.0 && y > top + 9.0 && y < top + h - 7.0
    };
    for i in 0..120u32 {
        let (sx, sy) = (
            left + 4.0 + hash(i * 2 + 1) * (w - 8.0),
            top + 3.0 + hash(i * 2 + 2) * (h - 6.0),
        );
        let d = ((sx - c.0).powi(2) + (sy - c.1).powi(2)).sqrt();
        if text_zone(sx, sy) || d < RX + 4.0 {
            continue;
        }
        let reveal = smooth(((wave_r - d) / 40.0).clamp(0.0, 1.0));
        if reveal <= 0.0 {
            continue;
        }
        let base = 0.1 + 0.22 * hash(i + 97);
        let alpha = reveal * (base + 0.6 * fading);
        let color = if i % 4 == 0 { t.gold } else { t.text };
        disc(
            window,
            p(sx, sy),
            0.5 + 0.55 * hash(i + 31),
            color.opacity(alpha),
        );
    }

    // A calm warmth behind the emblem once lit; brighter during the flash.
    let warmth = smooth(seg(s, IGNITE, 1.3));
    let flash = (1.0 - seg(s, IGNITE, 1.45)).powf(1.6) * (s >= IGNITE) as u8 as f32;
    let gather = smooth(seg(s, 0.55, IGNITE)) * (s < IGNITE) as u8 as f32;
    glow(
        window,
        center,
        RX + 6.0,
        40.0 + 40.0 * flash,
        t.gold
            .opacity(ALPHA_0_10 * warmth + ALPHA_0_35 * flash + ALPHA_0_08 * gather),
    );
    glow(
        window,
        center,
        R + 2.0,
        16.0 + 18.0 * flash,
        t.gold
            .opacity(ALPHA_0_22 * warmth + ALPHA_0_6 * flash + ALPHA_0_25 * gather),
    );

    if s >= IGNITE {
        circle(
            window,
            c,
            wave_r,
            2.4 * (1.0 - wave) + 0.6,
            t.gold
                .opacity(ALPHA_0_8 * (ALPHA_1_0 - wave).powi(WAVE_FADE_EXPONENT)),
        );
        let second = ease_out(seg(s, IGNITE + 0.09, 1.6));
        circle(
            window,
            c,
            8.0 + (w * 0.7) * second,
            1.0,
            t.text.opacity(
                ALPHA_0_35
                    * (ALPHA_1_0 - second).powi(WAVE_FADE_EXPONENT)
                    * (second > ALPHA_0_0) as u8 as f32,
            ),
        );
    }

    // The three orbits. Before ignition each is a comet trail that sweeps in, tightens and dives
    // into the hourglass; after it, each is a whole ring with its planet.
    let squeeze = if s < 0.6 {
        0.0
    } else if s < IGNITE {
        0.3 * ease_in(seg(s, 0.6, IGNITE))
    } else {
        0.3 * (1.0 - settle(after))
    };
    let dive = ease_in(seg(s, 0.7, IGNITE));
    let mut heads = Vec::new();
    let mut fronts = Vec::new();
    for (i, orbit) in orbits(t).iter().enumerate() {
        let start = i as f32 * 0.07;
        let arrive = ease_out(seg(s, start, start + 0.6));
        let presence = smooth(seg(s, start, start + 0.22));
        let off = 120.0 * (1.0 - arrive).powf(1.5);
        let (fs, fc) = orbit.from.to_radians().sin_cos();
        let ring = Ring {
            c: (c.0 + fc * off, c.1 + fs * off),
            scale: lerp(3.0, 1.0, arrive) * (1.0 - squeeze),
            tilt: orbit.tilt + orbit.spin * 150.0 * (1.0 - arrive),
        };
        let gilt = smooth(seg(s, 0.8, IGNITE)) * (1.0 - smooth(seg(s, 1.05, DURATION)));
        let tone = mix(orbit.tone, t.gold, gilt);
        if s < IGNITE {
            let head = orbit.rest - orbit.spin * 10.5 * (IGNITE - s).powf(1.15);
            let span = TAU * lerp(0.3, 0.98, smooth(seg(s, start + 0.05, 0.85)));
            const N: usize = 44;
            let mut segments = Vec::with_capacity(N);
            for k in 0..N {
                let (a0, a1) = (k as f32 / N as f32, (k + 1) as f32 / N as f32);
                let angle = |x: f32| head - orbit.spin * span * (1.0 - x);
                let (p0, p1) = (ring.at(angle(a0)), ring.at(angle(a1)));
                let alpha = presence * a1.powf(1.7) * (1.0 - 0.5 * dive);
                segments.push((
                    p(p0.0, p0.1),
                    p(p1.0, p1.1),
                    alpha,
                    Ring::front(angle((a0 + a1) / 2.0)),
                ));
            }
            for &(a, b, alpha, _) in &segments {
                line(
                    window,
                    &[a, b],
                    false,
                    5.0,
                    tone.opacity(alpha * ALPHA_0_18),
                );
                line(window, &[a, b], false, 1.7, tone.opacity(alpha));
            }
            fronts.push((
                tone,
                segments.into_iter().filter(|seg| seg.3).collect::<Vec<_>>(),
            ));
            let (hx, hy) = ring.at(head);
            let at = (lerp(hx, c.0, dive), lerp(hy, c.1, dive));
            heads.push((
                p(at.0, at.1),
                mix(tone, t.text, dive * 0.6),
                presence,
                2.4 + 1.4 * dive,
            ));
        } else {
            let whole: Vec<_> = (0..=96).map(|k| ring.at(k as f32 / 96.0 * TAU)).collect();
            let all: Vec<_> = whole.iter().map(|q| p(q.0, q.1)).collect();
            // Bright as they ignite, then calm.
            let calm = lerp(1.0, 0.72, smooth(seg(s, 1.1, DURATION)));
            line(window, &all, true, 4.0, tone.opacity(ALPHA_0_12 * calm));
            line(window, &all, true, 1.4, tone.opacity(calm));
            let front: Vec<_> = (0..=48)
                .map(|k| ring.at(k as f32 / 48.0 * PI))
                .map(|q| p(q.0, q.1))
                .collect();
            fronts.push((
                tone,
                front
                    .windows(2)
                    .map(|pair| (pair[0], pair[1], calm, true))
                    .collect(),
            ));
            let out = ease_out(seg(s, IGNITE + 0.03, 1.4));
            let (rx, ry) = ring.at(orbit.rest);
            let planet = mix(t.gold, orbit.tone, smooth(seg(s, 1.1, 1.55)));
            heads.push((
                p(lerp(c.0, rx, out), lerp(c.1, ry, out)),
                planet,
                smooth(seg(s, IGNITE, IGNITE + 0.08)),
                2.3,
            ));
        }
    }

    // The hourglass sits inside the orbits: its face hides their far side, and it turns over as
    // the comets close in, the way time is turned.
    let lit = smooth(seg(s, IGNITE - 0.04, 1.1));
    let approach = smooth(seg(s, 0.3, IGNITE));
    let backing = mix(t.canvas, t.surface, 0.3);
    disc(window, center, R + 0.5, backing);
    hourglass(
        window,
        c,
        t,
        lit,
        PI * smooth(seg(s, 0.5, 0.92)),
        1.0 + 0.4 * approach,
    );
    for (tone, segments) in &fronts {
        for (a, b, alpha, _) in segments {
            line(window, &[*a, *b], false, 1.5, tone.opacity(*alpha));
        }
    }

    // Sparks flung out by the ignition.
    if s >= IGNITE {
        let k = seg(s, IGNITE, 1.4);
        let alpha = (1.0 - k).powf(1.5) * 0.95;
        for j in 0..14u32 {
            let a = j as f32 / 14.0 * TAU + 0.3 * hash(j + 7);
            let reach = 0.75 + 0.5 * hash(j + 53);
            let inner = 9.0 + 46.0 * ease_out(k) * reach;
            let outer = inner + 13.0 * (1.0 - k) * reach;
            let color = if j % 3 == 0 { t.text } else { t.gold };
            line(
                window,
                &[
                    p(c.0 + inner * a.cos(), c.1 + inner * a.sin()),
                    p(c.0 + outer * a.cos(), c.1 + outer * a.sin()),
                ],
                false,
                1.2,
                color.opacity(alpha),
            );
        }
    }

    for (at, color, alpha, r) in heads {
        if alpha <= 0.0 {
            continue;
        }
        glow(window, at, r, 7.0, color.opacity(ALPHA_0_45 * alpha));
        disc(window, at, r, color.opacity(alpha));
    }

    // The white-gold core of the ignition.
    if flash > 0.0 {
        glow(
            window,
            center,
            4.0 + 6.0 * flash,
            14.0,
            t.text.opacity(ALPHA_0_7 * flash),
        );
        disc(
            window,
            center,
            2.0 + 4.0 * flash,
            mix(t.gold, t.text, 0.6).opacity(flash),
        );
    }

    // A glint of gold runs along the card's edges once, then the border holds the light.
    let run = seg(s, 1.0, 1.5);
    if run > 0.0 && run < 1.0 {
        let x = left + 12.0 + (w - 24.0) * smooth(run);
        let alpha = (run * PI).sin();
        for (y, len) in [(top + 0.75, 70.0), (top + h - 0.75, 46.0)] {
            for (width, a) in [(3.0, 0.18), (1.2, 0.9)] {
                line(
                    window,
                    &[p((x - len).max(left), y), p(x, y)],
                    false,
                    width,
                    t.gold.opacity(a * alpha * ALPHA_0_5),
                );
                line(
                    window,
                    &[p((x - len * 0.35).max(left), y), p(x, y)],
                    false,
                    width,
                    t.gold.opacity(a * alpha),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{backdrop, border, Activation, EMBLEM_GUTTER};
    use crate::desktop::{superswitch as original, theme};
    use std::time::{Duration, Instant};

    #[test]
    fn matches_original_activation_and_looks() {
        let now = Instant::now();
        let mut actual = Activation::default();
        let mut expected = original::Activation::default();
        fn check(actual: &Activation, expected: &original::Activation, now: Instant) {
            for reduce in [false, true] {
                let a = actual.look(now, reduce);
                let b = expected.look(now, reduce);
                assert_eq!(format!("{a:?}"), format!("{b:?}"));
                assert_eq!(
                    actual.wants_frame(now, reduce),
                    expected.wants_frame(now, reduce)
                );
                for t in theme::both() {
                    assert_eq!(border(a, &t), original::border(b, &t));
                    drop(backdrop(a, t));
                    drop(original::backdrop(b, t));
                }
            }
            assert_eq!(actual.is_on(), expected.is_on());
        }
        assert_eq!(EMBLEM_GUTTER, original::EMBLEM_GUTTER);
        check(&actual, &expected, now);
        actual.set(true, now);
        expected.set(true, now);
        for elapsed in [0, 950, 1100, 1300, 1600, 2000] {
            check(&actual, &expected, now + Duration::from_millis(elapsed));
        }
        for seconds in [-1.0, 0.5, 1.2, 1.6, 2.0] {
            actual.hold(seconds);
            expected.hold(seconds);
            check(&actual, &expected, now);
        }
        actual.set(false, now);
        expected.set(false, now);
        check(&actual, &expected, now);
    }
}
