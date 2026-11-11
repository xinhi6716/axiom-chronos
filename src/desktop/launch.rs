//! The launch: a short ritual that opens the app, after the Chronos portrait.
//!
//! In darkness a faint star field gathers; the keeper emerges from shadow as light descends from
//! the face to the hourglass; gold orbits sweep in around the glass; the hourglass turns once and
//! its sand catches the light; then the night dissolves into the app and the portrait settles into
//! its place at the top of the rail. About 2.4 seconds, once per start.
//!
//! It plays only on a normal start: never in an `--offscreen` capture unless the scene asks for
//! it, and never under `reduce_motion`, which goes straight to the app. Any key or click ends it at
//! once. The app's data loads underneath it, so it costs no waiting.
use super::motion::{ease_out, lerp, mix, seg, smooth, Pace, Settling};
use super::ornament::PORTRAIT;
use super::theme::{self, Density, TypeSize, Tokens};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::f32::consts::{PI, TAU};
use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

/// Where the hourglass sits in the portrait, as fractions of its side.
const GLASS_X: f32 = 0.5;
const GLASS_Y: f32 = 0.735;
/// Half the hourglass's height and width, as fractions of the portrait's side.
const GLASS_H: f32 = 0.158;
const GLASS_W: f32 = 0.088;

/// The beats of the ritual, in seconds.
const STARS_FROM: f32 = 0.0;
const EMERGE: (f32, f32) = (0.22, 1.15);
const LIGHT_DESCENDS: (f32, f32) = (0.3, 1.35);
const ORBITS_FROM: f32 = 0.9;
const TURN: (f32, f32) = (1.25, 1.78);
const SAND_LIGHT: (f32, f32) = (1.62, 1.88);
const DISSOLVE: (f32, f32) = (1.86, 2.4);

pub fn duration() -> f32 {
    Pace::Launch.seconds()
}

/// The demo scene that holds the launch at a moment, `launch-at-<ms>`.
pub fn held_ms(scene: Option<&str>) -> Option<u64> {
    scene?.strip_prefix("launch-at-")?.parse().ok()
}

/// Whether this start plays the launch. A capture plays it only when its scene asks.
pub fn plays(offscreen: bool, scene: Option<&str>, reduce_motion: bool) -> bool {
    if reduce_motion {
        return false;
    }
    let asked = scene.is_some_and(|scene| scene == "launch" || held_ms(Some(scene)).is_some());
    asked || (!offscreen && scene.is_none())
}

/// The launch's progress: when it began, or `None` once it is over or was never played.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Launch {
    since: Option<Instant>,
}

impl Launch {
    pub fn none() -> Self {
        Self { since: None }
    }

    pub fn start(now: Instant) -> Self {
        Self { since: Some(now) }
    }

    /// Any key or click: straight to the app.
    pub fn skip(&mut self) {
        self.since = None;
    }

    /// Seconds into the ritual, or `None` when nothing is playing.
    pub fn seconds(&self, now: Instant) -> Option<f32> {
        let s = now.saturating_duration_since(self.since?).as_secs_f32();
        (s < duration()).then_some(s)
    }

    pub fn playing(&self, now: Instant) -> bool {
        self.seconds(now).is_some()
    }
}

impl Settling for Launch {
    fn settled(&self, now: Instant, reduce: bool) -> bool {
        reduce || !self.playing(now)
    }
}

/// A square on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub x: f32,
    pub y: f32,
    pub side: f32,
}

impl Square {
    fn lerp(self, to: Square, k: f32) -> Square {
        Square { x: lerp(self.x, to.x, k), y: lerp(self.y, to.y, k), side: lerp(self.side, to.side, k) }
    }

    fn glass(self) -> (f32, f32) {
        (self.x + self.side * GLASS_X, self.y + self.side * GLASS_Y)
    }
}

/// Where the portrait stands while it is the centre of the ritual: centred a little above the
/// middle, as large as the window comfortably allows.
pub fn stage(w: f32, h: f32) -> Square {
    let side = (h * 0.5).clamp(220.0, 420.0).min(w * 0.42);
    Square { x: (w - side) / 2.0, y: (h - side) / 2.0 - h * 0.035, side }
}

/// Where the portrait is at `s`: it grows a little as it emerges, then flies to the rail.
pub fn portrait_at(s: f32, w: f32, h: f32, rail: Option<Square>) -> Square {
    let full = stage(w, h);
    let grow = lerp(0.955, 1.0, ease_out(seg(s, EMERGE.0, 1.75)));
    let side = full.side * grow;
    let centred = Square { x: full.x + (full.side - side) / 2.0, y: full.y + (full.side - side) / 2.0, side };
    match rail {
        Some(rail) => centred.lerp(rail, smooth(seg(s, DISSOLVE.0, DISSOLVE.1))),
        None => centred,
    }
}

/// How visible each layer is at `s`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers {
    /// The night behind everything; it dissolves to show the app.
    pub night: f32,
    pub portrait: f32,
    /// The drawn hourglass and its orbits.
    pub emblem: f32,
    pub title: f32,
}

pub fn layers(s: f32, flies: bool) -> Layers {
    let gone = smooth(seg(s, DISSOLVE.0, DISSOLVE.1));
    let emerged = 0.9 * smooth(seg(s, EMERGE.0, EMERGE.1));
    Layers {
        night: 1.0 - gone,
        // Landing on the rail, the portrait becomes the rail's own; with no rail it dissolves too.
        portrait: if flies { lerp(emerged, 1.0, gone) } else { emerged * (1.0 - gone) },
        emblem: smooth(seg(s, 0.85, 1.15)) * (1.0 - smooth(seg(s, 1.84, 2.12))),
        title: smooth(seg(s, 1.0, 1.45)) * (1.0 - smooth(seg(s, 1.8, 2.05))),
    }
}

/// The overlay for one moment of the launch. `rail` is where the rail's portrait is drawn, so the
/// keeper can come to rest there. Fills the window; the caller makes it take the clicks.
pub fn overlay(s: f32, w: f32, h: f32, rail: Option<Square>, night: Tokens) -> Div {
    let rect = portrait_at(s, w, h, rail);
    let layer = layers(s, rail.is_some());
    let bg = backdrop_color(&night);
    // Light descends over the portrait from the face to the hourglass: above the line it is lit,
    // below it the night still covers the card.
    let line = lerp(-0.4, 1.08, smooth(seg(s, LIGHT_DESCENDS.0, LIGHT_DESCENDS.1)));
    let veil = linear_gradient(
        180.0,
        linear_color_stop(bg.opacity(0.0), line.clamp(0.0, 1.0)),
        linear_color_stop(bg.opacity(layer.night), (line + 0.32).clamp(0.0, 1.0)),
    );
    let fade = |angle: f32| {
        linear_gradient(angle, linear_color_stop(bg.opacity(layer.night), 0.0), linear_color_stop(bg.opacity(0.0), 1.0))
    };
    let edge = px(rect.side * 0.13);
    let place = |d: Div| d.absolute().left(px(rect.x)).top(px(rect.y)).size(px(rect.side));
    let title_top = stage(w, h).y + stage(w, h).side + 20.0;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            canvas(|_, _, _| (), move |bounds, _, window, _| paint_backdrop(window, bounds, s, rect, &night))
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .child(img(PORTRAIT).absolute().left(px(rect.x)).top(px(rect.y)).size(px(rect.side)).opacity(layer.portrait))
        .child(
            place(div())
                .when(line < 1.08, |this| this.child(div().absolute().top_0().left_0().size_full().bg(veil)))
                .child(div().absolute().left_0().right_0().top_0().h(edge).bg(fade(180.0)))
                .child(div().absolute().left_0().right_0().bottom_0().h(edge).bg(fade(0.0)))
                .child(div().absolute().top_0().bottom_0().left_0().w(edge).bg(fade(90.0)))
                .child(div().absolute().top_0().bottom_0().right_0().w(edge).bg(fade(270.0))),
        )
        .child(
            canvas(|_, _, _| (), move |_, _, window, _| paint_emblem(window, s, rect, layer, &night))
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .when(layer.title > 0.0, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(title_top))
                    .flex()
                    .justify_center()
                    .text_size(theme::font(Density::Comfortable, TypeSize::Body))
                    .text_color(night.gold.opacity(0.85 * layer.title))
                    .child("C\u{2002}H\u{2002}R\u{2002}O\u{2002}N\u{2002}O\u{2002}S"),
            )
        })
}

/// The navy of the portrait's own ground, sampled from `assets/chronos.ico`.
const PORTRAIT_NAVY: u32 = 0x142837;

/// The night of the launch: the dark theme's canvas, deeper.
fn backdrop_color(t: &Tokens) -> Hsla {
    mix(t.canvas, hsla(0.6, 0.5, 0.02, 1.0), 0.5)
}

fn p(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

fn line(window: &mut Window, points: &[Point<Pixels>], closed: bool, width: f32, color: Hsla) {
    if color.a <= 0.003 || points.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.add_polygon(points, closed);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn disc(window: &mut Window, c: (f32, f32), r: f32, color: Hsla) {
    if color.a <= 0.003 {
        return;
    }
    let bounds = Bounds { origin: p(c.0 - r, c.1 - r), size: size(px(r * 2.0), px(r * 2.0)) };
    window.paint_quad(fill(bounds, color).corner_radii(px(r)));
}

fn glow(window: &mut Window, c: (f32, f32), r: f32, blur: f32, color: Hsla) {
    if color.a <= 0.003 {
        return;
    }
    let bounds = Bounds { origin: p(c.0 - r, c.1 - r), size: size(px(r * 2.0), px(r * 2.0)) };
    window.paint_drop_shadows(
        bounds,
        Corners::all(px(r)),
        &[BoxShadow { color, offset: Point::default(), blur_radius: px(blur), spread_radius: px(0.), inset: false }],
    );
}

fn hash(a: u32, b: u32, salt: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9e37_79b1) ^ b.wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h % 10_000) as f32 / 10_000.0
}

/// One star of the launch sky: where it is, how bright, when it appears.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaunchStar {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub alpha: f32,
    pub appear: f32,
    pub gold: bool,
}

const SKY_CELL: f32 = 54.0;

/// The launch sky: fixed stars that each kindle at their own moment in the first second.
pub fn sky(w: f32, h: f32) -> Vec<LaunchStar> {
    let (cols, rows) = ((w / SKY_CELL).ceil() as u32, (h / SKY_CELL).ceil() as u32);
    let mut stars = vec![];
    for row in 0..rows {
        for col in 0..cols {
            if hash(col, row, 11) > 0.6 {
                continue;
            }
            let (x, y) = ((col as f32 + hash(col, row, 12)) * SKY_CELL, (row as f32 + hash(col, row, 13)) * SKY_CELL);
            if x >= w || y >= h {
                continue;
            }
            let bright = hash(col, row, 14).powi(3);
            stars.push(LaunchStar {
                x,
                y,
                r: 0.45 + 1.15 * bright,
                alpha: 0.09 + 0.5 * bright,
                appear: STARS_FROM + 0.05 + 0.85 * hash(col, row, 15),
                gold: hash(col, row, 16) < 0.12,
            });
        }
    }
    stars
}

fn paint_backdrop(window: &mut Window, bounds: Bounds<Pixels>, s: f32, rect: Square, t: &Tokens) {
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let layer = layers(s, true);
    window.paint_quad(fill(bounds, backdrop_color(t).opacity(layer.night)));

    // A cold glow of the observatory behind the keeper, then the warmth of the lit sand.
    let centre = (rect.x + rect.side / 2.0, rect.y + rect.side / 2.0);
    let presence = smooth(seg(s, 0.1, 1.2)) * layer.night;
    glow(window, centre, rect.side * 0.42, rect.side * 0.55, mix(t.canvas, t.run, 0.22).opacity(0.55 * presence));
    let warmth = smooth(seg(s, SAND_LIGHT.0, SAND_LIGHT.1)) * layer.night;
    glow(window, rect.glass(), rect.side * 0.12, rect.side * 0.32, t.gold.opacity(0.16 * warmth));

    for star in sky(w, h) {
        let kindle = ease_out(seg(s, star.appear, star.appear + 0.45));
        if kindle <= 0.0 {
            continue;
        }
        let ink = if star.gold { t.gold } else { t.text };
        let at = (left + star.x, top + star.y);
        disc(window, at, star.r, ink.opacity(star.alpha * kindle * layer.night));
        // The brightest catch a brief glint as they kindle.
        if star.alpha > 0.3 {
            let glint = (1.0 - seg(s, star.appear, star.appear + 0.7)) * kindle * layer.night;
            let reach = 2.0 + 6.0 * glint;
            let color = ink.opacity(0.5 * glint * star.alpha);
            line(window, &[p(at.0 - reach, at.1), p(at.0 + reach, at.1)], false, 0.8, color);
            line(window, &[p(at.0, at.1 - reach), p(at.0, at.1 + reach)], false, 0.8, color);
        }
    }
}

/// An orbit of the hourglass: an ellipse tilted about the glass.
#[derive(Clone, Copy)]
struct Orbit {
    rx: f32,
    ry: f32,
    tilt: f32,
    start: f32,
    spin: f32,
    rest: f32,
}

const ORBITS: [Orbit; 3] = [
    // The flat ring the portrait draws around the glass.
    Orbit { rx: 0.215, ry: 0.048, tilt: 0.0, start: ORBITS_FROM, spin: 1.0, rest: 0.4 },
    Orbit { rx: 0.25, ry: 0.07, tilt: -23.0, start: ORBITS_FROM + 0.1, spin: -1.0, rest: 2.6 },
    Orbit { rx: 0.232, ry: 0.062, tilt: 25.0, start: ORBITS_FROM + 0.2, spin: 1.0, rest: 4.4 },
];

impl Orbit {
    fn at(&self, c: (f32, f32), side: f32, scale: f32, a: f32) -> (f32, f32) {
        let (x, y) = (self.rx * side * scale * a.cos(), self.ry * side * scale * a.sin());
        let (sin, cos) = self.tilt.to_radians().sin_cos();
        (c.0 + x * cos - y * sin, c.1 + x * sin + y * cos)
    }
}

/// The far half of an orbit passes behind the glass.
fn in_front(a: f32) -> bool {
    a.rem_euclid(TAU) < PI
}

type Segment = (Point<Pixels>, Point<Pixels>, f32);

/// One orbit at `s`: its segments and the head of its comet, split into the halves behind and in
/// front of the glass.
fn orbit_segments(o: &Orbit, c: (f32, f32), side: f32, s: f32) -> (Vec<Segment>, Vec<Segment>, Option<((f32, f32), f32)>) {
    let arrive = ease_out(seg(s, o.start, o.start + 0.78));
    let presence = smooth(seg(s, o.start, o.start + 0.2));
    if presence <= 0.0 {
        return (vec![], vec![], None);
    }
    let scale = lerp(1.55, 1.0, arrive);
    let head = o.rest - o.spin * TAU * 1.15 * (1.0 - arrive).powf(1.3);
    let span = TAU * lerp(0.1, 1.0, smooth(seg(s, o.start + 0.05, o.start + 0.8)));
    // Bright as it arrives, then a calm hairline.
    let calm = lerp(1.0, 0.5, smooth(seg(s, o.start + 0.7, 1.95)));
    const N: usize = 72;
    let (mut back, mut front) = (vec![], vec![]);
    for k in 0..N {
        let (a0, a1) = (k as f32 / N as f32, (k + 1) as f32 / N as f32);
        let angle = |x: f32| head - o.spin * span * (1.0 - x);
        let (q0, q1) = (o.at(c, side, scale, angle(a0)), o.at(c, side, scale, angle(a1)));
        // A comet's tail thins away from its head; a closed ring is even.
        let tail = lerp(a1.powf(1.6), 1.0, smooth(seg(span, TAU * 0.92, TAU)));
        let segment = (p(q0.0, q0.1), p(q1.0, q1.1), presence * tail * calm);
        if in_front(angle((a0 + a1) / 2.0)) {
            front.push(segment);
        } else {
            back.push(segment);
        }
    }
    let lead = 1.0 - smooth(seg(s, o.start + 0.75, o.start + 1.0));
    (back, front, Some((o.at(c, side, scale, head), presence * lead)))
}

/// An orbit as a line of light: a soft gold halo with a brighter core.
fn paint_segments(window: &mut Window, segments: &[Segment], t: &Tokens, alpha: f32) {
    let core = mix(t.gold, t.text, 0.22);
    for &(a, b, k) in segments {
        line(window, &[a, b], false, 6.0, t.gold.opacity(0.16 * k * alpha));
        line(window, &[a, b], false, 1.7, core.opacity(k * alpha));
    }
}

/// The drawn hourglass over the portrait's own, turned by `turn` radians. `light` is how far the
/// sand has caught the light.
fn paint_hourglass(window: &mut Window, c: (f32, f32), side: f32, t: &Tokens, turn: f32, light: f32, alpha: f32, ground: Hsla) {
    let (gh, gw) = (GLASS_H * side, GLASS_W * side);
    let (sin, cos) = turn.sin_cos();
    let at = |x: f32, y: f32| {
        let (x, y) = (x * gw, y * gh);
        p(c.0 + x * cos - y * sin, c.1 + x * sin + y * cos)
    };
    // Cover the portrait's own glass in the card's navy, so only one hourglass is seen to turn.
    // Its soft edge reads as a pool of shadow around the glass.
    let cover = Bounds { origin: p(c.0 - gw * 1.3, c.1 - gh * 1.1), size: size(px(gw * 2.6), px(gh * 2.2)) };
    window.paint_drop_shadows(
        cover,
        Corners::all(px(gw * 1.3)),
        &[BoxShadow { color: ground.opacity(alpha), offset: Point::default(), blur_radius: px(gw * 0.9), spread_radius: px(0.), inset: false }],
    );
    window.paint_quad(fill(cover, ground.opacity(alpha)).corner_radii(px(gw * 1.3)));

    let frame = mix(t.text.opacity(0.85), t.gold, 0.45 + 0.55 * light).opacity(alpha);
    let glass = mix(t.text.opacity(0.8), t.gold, 0.4 * light).opacity(alpha);
    // The caps and the posts of the frame.
    for y in [-1.0f32, 1.0] {
        line(window, &[at(-1.0, y), at(1.0, y)], false, 2.6, frame);
        line(window, &[at(-0.86, y * 1.08), at(0.86, y * 1.08)], false, 1.2, frame);
    }
    for x in [-0.84f32, 0.84] {
        line(window, &[at(x, -1.0), at(x, 1.0)], false, 1.6, frame);
        disc(window, (at(x, 0.0).x.into(), at(x, 0.0).y.into()), 1.6, frame);
    }
    // The two bulbs, meeting at the neck.
    for side in [-1.0f32, 1.0] {
        let mut b = PathBuilder::stroke(px(1.4));
        b.move_to(at(side * 0.6, -0.92));
        b.cubic_bezier_to(at(side * 0.08, -0.06), at(side * 0.72, -0.5), at(side * 0.12, -0.2));
        b.cubic_bezier_to(at(side * 0.6, 0.92), at(side * 0.12, 0.2), at(side * 0.72, 0.5));
        if let Ok(path) = b.build() {
            window.paint_path(path, glass);
        }
    }
    // The sand lies in the bottom bulb of the glass's own frame, so turning the glass lifts it.
    let sand = mix(t.muted.opacity(0.55), t.gold, light).opacity(alpha);
    let mut pile = PathBuilder::fill();
    pile.move_to(at(-0.52, 0.86));
    pile.curve_to(at(0.0, 0.42), at(-0.3, 0.55));
    pile.curve_to(at(0.52, 0.86), at(0.3, 0.55));
    pile.close();
    if let Ok(path) = pile.build() {
        window.paint_path(path, sand);
    }
}

fn paint_emblem(window: &mut Window, s: f32, rect: Square, layer: Layers, t: &Tokens) {
    let c = rect.glass();
    let side = rect.side;
    // The light that wakes the card: it opens at the keeper's brow and descends to the glass.
    let descent = smooth(seg(s, LIGHT_DESCENDS.0 + 0.1, LIGHT_DESCENDS.1));
    let bloom = (seg(s, LIGHT_DESCENDS.0, LIGHT_DESCENDS.1 + 0.2) * PI).sin().max(0.0) * layer.night;
    if bloom > 0.0 {
        let at = (rect.x + side * 0.5, rect.y + side * lerp(0.21, GLASS_Y, descent));
        glow(window, at, side * 0.05, side * 0.2, t.gold.opacity(0.22 * bloom));
        glow(window, at, 2.0, side * 0.05, mix(t.gold, t.text, 0.5).opacity(0.35 * bloom));
    }
    let alpha = layer.emblem;
    if alpha <= 0.0 {
        return;
    }
    let ground = mix(backdrop_color(t), rgb(PORTRAIT_NAVY).into(), layer.portrait);
    let turn = PI * smooth(seg(s, TURN.0, TURN.1));
    let light = smooth(seg(s, SAND_LIGHT.0, SAND_LIGHT.1));

    // A pool of shadow round the glass, so the orbits read as light against the card.
    glow(window, c, side * 0.17, side * 0.16, ground.opacity(0.8 * alpha));
    let mut fronts = vec![];
    let mut heads = vec![];
    for orbit in &ORBITS {
        let (back, front, head) = orbit_segments(orbit, c, side, s);
        paint_segments(window, &back, t, alpha);
        fronts.push(front);
        heads.extend(head);
    }
    paint_hourglass(window, c, side, t, turn, light, alpha, ground);
    // Warmth gathers in the glass as the sand catches the light.
    glow(window, c, side * 0.05, side * 0.12, t.gold.opacity(0.22 * light * alpha));
    for front in &fronts {
        paint_segments(window, front, t, alpha);
    }
    for (at, k) in heads {
        glow(window, at, 3.0, 12.0, t.gold.opacity(0.7 * k * alpha));
        disc(window, at, 2.4, mix(t.gold, t.text, 0.6).opacity(k * alpha));
    }

    // The sand catches the light: a glint on the lifted sand and a stream begins to fall.
    if light > 0.0 {
        let gh = GLASS_H * side;
        let glint = (light * PI).sin().max(0.0) * (1.0 - seg(s, SAND_LIGHT.1, 2.1));
        let top = (c.0, c.1 - gh * 0.62);
        glow(window, top, 3.0 + 4.0 * glint, 14.0, t.gold.opacity(0.55 * glint * alpha));
        disc(window, top, 1.2 + 1.6 * glint, mix(t.gold, t.text, 0.7).opacity(glint * alpha));
        let stream = ease_out(seg(s, SAND_LIGHT.0 + 0.08, 2.05));
        let (from, to) = (c.1 - gh * 0.04, c.1 + gh * lerp(0.04, 0.78, stream));
        line(window, &[p(c.0, from), p(c.0, to)], false, 1.1, t.gold.opacity(0.85 * light * alpha));
        // One ring of light runs out from the glass as it wakes.
        let ring = ease_out(seg(s, SAND_LIGHT.0 + 0.12, 2.35));
        if ring > 0.0 && ring < 1.0 {
            let r = lerp(gh * 0.9, side * 0.95, ring);
            let points: Vec<_> = (0..=120).map(|i| {
                let a = i as f32 / 120.0 * TAU;
                p(c.0 + r * a.cos(), c.1 + r * a.sin())
            }).collect();
            line(window, &points, true, 1.0, t.gold.opacity(0.3 * (1.0 - ring).powi(2) * alpha.max(0.4)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    #[test]
    fn the_launch_lasts_two_to_two_and_a_half_seconds() {
        assert!((2.0..=2.5).contains(&duration()), "{}", duration());
        assert!(DISSOLVE.1 <= duration() + 1e-6);
        assert!(EMERGE.0 < TURN.0 && TURN.0 < SAND_LIGHT.0 && SAND_LIGHT.0 < DISSOLVE.0, "the beats are in order");
    }

    #[test]
    fn it_plays_on_a_normal_start_and_never_in_a_capture_unless_asked() {
        assert!(plays(false, None, false), "a normal start plays the ritual");
        assert!(!plays(true, None, false), "an --offscreen capture skips it");
        assert!(!plays(true, Some("palette"), false), "a scene about something else skips it");
        assert!(!plays(false, Some("palette"), false));
        assert!(plays(true, Some("launch"), false), "a capture can ask for it");
        assert!(plays(true, Some("launch-at-900"), false));
        assert!(!plays(false, None, true), "reduce motion goes straight to the app");
        assert!(!plays(true, Some("launch-at-900"), true));
        assert_eq!(held_ms(Some("launch-at-1250")), Some(1250));
        assert_eq!(held_ms(Some("launch-at-")), None);
        assert_eq!(held_ms(Some("launch")), None);
        assert_eq!(held_ms(None), None);
    }

    #[test]
    fn any_key_or_click_ends_it_at_once() {
        let t0 = Instant::now();
        let mut launch = Launch::start(t0);
        let mid = t0 + Duration::from_millis(700);
        assert!(launch.playing(mid));
        assert!(!launch.settled(mid, false), "a playing launch asks for frames");
        launch.skip();
        assert_eq!(launch.seconds(mid), None);
        assert!(launch.settled(mid, false), "a skipped launch asks for nothing more");
    }

    #[test]
    fn it_ends_by_itself_and_then_rests() {
        let t0 = Instant::now();
        let launch = Launch::start(t0);
        assert_eq!(launch.seconds(t0), Some(0.0));
        let end = t0 + Pace::Launch.duration();
        assert!(!launch.playing(end));
        for later in [end, end + Duration::from_secs(60)] {
            assert!(launch.settled(later, false));
        }
        assert!(Launch::none().settled(t0, false));
        assert!(launch.settled(t0, true), "reduce motion settles it at once");
    }

    #[test]
    fn it_begins_in_darkness_and_ends_with_the_app_fully_shown() {
        let start = layers(0.0, true);
        assert_eq!(start.night, 1.0);
        assert_eq!((start.portrait, start.emblem, start.title), (0.0, 0.0, 0.0));
        let end = layers(duration(), true);
        assert!(end.night.abs() < 1e-6, "the night has dissolved");
        assert!((end.portrait - 1.0).abs() < 1e-6, "the portrait has become the rail's own");
        assert!(end.emblem.abs() < 1e-6 && end.title.abs() < 1e-6);
        assert!(layers(duration(), false).portrait.abs() < 1e-6, "with no rail to land on, it dissolves");
        // In the middle the keeper is present and the hourglass is drawn.
        let mid = layers(1.5, true);
        assert!(mid.portrait > 0.8 && mid.emblem > 0.9 && mid.night == 1.0);
    }

    #[test]
    fn the_keeper_comes_to_rest_exactly_on_the_rail_portrait() {
        let rail = Square { x: 18.0, y: 46.0, side: 44.0 };
        let landed = portrait_at(duration(), 1440.0, 900.0, Some(rail));
        assert!((landed.x - rail.x).abs() < 1e-3 && (landed.y - rail.y).abs() < 1e-3 && (landed.side - rail.side).abs() < 1e-3);
        let staged = portrait_at(1.5, 1440.0, 900.0, Some(rail));
        let full = stage(1440.0, 900.0);
        assert!(staged.side > full.side * 0.95 && staged.side <= full.side + 1e-3);
    }

    #[test]
    fn the_stage_fits_every_window_the_app_allows() {
        for (w, h) in [(1100.0, 700.0), (1440.0, 900.0), (2560.0, 1440.0), (900.0, 600.0)] {
            let s = stage(w, h);
            assert!(s.x >= 0.0 && s.y >= 0.0 && s.x + s.side <= w && s.y + s.side + 40.0 <= h, "{w}x{h}: {s:?}");
        }
    }

    #[test]
    fn the_sky_is_fixed_and_faint() {
        let a = sky(1440.0, 900.0);
        assert_eq!(a, sky(1440.0, 900.0));
        assert!(a.len() > 80, "{} stars", a.len());
        assert!(a.iter().all(|s| s.alpha <= 0.6 && s.appear < 1.0 && s.x < 1440.0 && s.y < 900.0));
        let gold = a.iter().filter(|s| s.gold).count();
        assert!(gold * 4 < a.len(), "gold stays the rare light");
    }
}
