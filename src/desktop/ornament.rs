//! The Celestial Order's ornaments, after the Chronos portrait: the tarot frame, the measured
//! divider, the state glyphs, the fixed star field and the portrait itself.
//! The geometry functions are pure, so the drawing can be tested without a window.
use super::model::UnitState;
use super::theme::Tokens;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::*;
use std::borrow::Cow;
use std::f32::consts::TAU;

/// The asset path of the portrait. Served by [`Assets`] from the PNG inside `assets/chronos.ico`.
pub const PORTRAIT: &str = "chronos/portrait.png";
static ICON: &[u8] = include_bytes!("../../assets/chronos.ico");

/// How far the frame's second hairline sits inside the first.
pub const FRAME_INSET: f32 = 3.0;
/// Half the diagonal of a corner stud.
pub const STUD: f32 = 3.0;
/// Half the diagonal of the divider's central diamond, and the gap the lines leave around it.
const DIVIDER_DIAMOND: f32 = 4.5;
const DIVIDER_GAP: f32 = 9.0;
pub const DIVIDER_H: f32 = 13.0;

/// Seconds per breath for everything that moves: within the 1.5 to 3 s the design allows.
pub const BREATH_SECONDS: f32 = 2.4;

/// The fixed star field: one cell in this many pixels may hold a star.
const STAR_CELL: f32 = 86.0;
const STAR_CHANCE: u32 = 62;
/// Stars stay below this opacity, so they are atmosphere and never compete with content.
pub const STAR_MAX_ALPHA: f32 = 0.055;

/// The largest PNG image inside an ICO file. Windows icons store their 256px image as PNG.
pub fn ico_png(ico: &[u8]) -> Option<&[u8]> {
    let u16_at = |i: usize| Some(u16::from_le_bytes(ico.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(ico.get(i..i + 4)?.try_into().ok()?) as usize);
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return None;
    }
    (0..u16_at(4)? as usize)
        .filter_map(|i| {
            let entry = 6 + 16 * i;
            let side = match *ico.get(entry)? {
                0 => 256,
                n => n as u32,
            };
            let (len, offset) = (u32_at(entry + 8)?, u32_at(entry + 12)?);
            let data = ico.get(offset..offset.checked_add(len)?)?;
            data.starts_with(b"\x89PNG\r\n\x1a\n").then_some((side, data))
        })
        .max_by_key(|(side, _)| *side)
        .map(|(_, data)| data)
}

/// The app's asset source: the kit's icons, plus the portrait cut from the window icon so the art
/// is stored once.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == PORTRAIT {
            return match ico_png(ICON) {
                Some(png) => Ok(Some(Cow::Borrowed(png))),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "assets/chronos.ico holds no PNG image to draw the portrait from",
                )
                .into()),
            };
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = gpui_kit::assets::Assets.list(path)?;
        if PORTRAIT.starts_with(path) {
            names.push(PORTRAIT.into());
        }
        Ok(names)
    }
}

/// The portrait at a given size.
pub fn portrait(side: f32) -> Img {
    img(PORTRAIT).flex_none().size(px(side))
}

/// The portrait large and dim, for the space behind an empty state. Its edges dissolve into the
/// canvas, and the bottom fades out fully, so text placed below it sits on the plain canvas.
pub fn portrait_backdrop(t: &Tokens, side: f32, dark: bool) -> Div {
    let fade = |angle: f32| linear_gradient(angle, linear_color_stop(t.canvas, 0.0), linear_color_stop(t.canvas.opacity(0.0), 1.0));
    let edge = relative(0.3);
    div()
        .relative()
        .flex_none()
        .size(px(side))
        .child(portrait(side).opacity(if dark { 0.16 } else { 0.12 }))
        .child(div().absolute().left_0().right_0().top_0().h(edge).bg(fade(180.0)))
        .child(div().absolute().top_0().bottom_0().left_0().w(edge).bg(fade(90.0)))
        .child(div().absolute().top_0().bottom_0().right_0().w(edge).bg(fade(270.0)))
        .child(div().absolute().left_0().right_0().bottom_0().h(relative(0.5)).bg(fade(0.0)))
}

/// A periodic 0..1 swell, 0 at rest. Everything that moves in the app breathes at this pace.
pub fn breath(seconds: f32) -> f32 {
    0.5 - 0.5 * (TAU * seconds / BREATH_SECONDS).cos()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Box2 {
    fn of(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            w: f32::from(bounds.size.width),
            h: f32::from(bounds.size.height),
        }
    }

    fn bounds(&self) -> Bounds<Pixels> {
        Bounds { origin: point(px(self.x), px(self.y)), size: size(px(self.w), px(self.h)) }
    }
}

/// Where the parts of a tarot frame go inside `outer`.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameGeometry {
    pub outer: Box2,
    /// The second hairline, or `None` when the frame is too small to hold it.
    pub inner: Option<Box2>,
    /// Centres of the corner studs, clockwise from the top left.
    pub studs: [(f32, f32); 4],
}

pub fn frame_geometry(outer: Box2) -> FrameGeometry {
    let room = FRAME_INSET * 2.0 + 2.0;
    let inner = (outer.w > room && outer.h > room).then(|| Box2 {
        x: outer.x + FRAME_INSET,
        y: outer.y + FRAME_INSET,
        w: outer.w - FRAME_INSET * 2.0,
        h: outer.h - FRAME_INSET * 2.0,
    });
    // On the outer hairline's centre line, half a pixel inside the box.
    let (l, t, r, b) = (outer.x + 0.5, outer.y + 0.5, outer.x + outer.w - 0.5, outer.y + outer.h - 0.5);
    FrameGeometry { outer, inner, studs: [(l, t), (r, t), (r, b), (l, b)] }
}

fn diamond(window: &mut Window, (x, y): (f32, f32), half: f32, color: Hsla) {
    let mut b = PathBuilder::fill();
    b.move_to(point(px(x), px(y - half)));
    b.line_to(point(px(x + half), px(y)));
    b.line_to(point(px(x), px(y + half)));
    b.line_to(point(px(x - half), px(y)));
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn paint_frame(window: &mut Window, bounds: Bounds<Pixels>, line: Hsla, inner: Hsla, stud: Hsla, fill: Hsla) {
    let g = frame_geometry(Box2::of(bounds));
    window.paint_quad(outline(g.outer.bounds(), line, BorderStyle::Solid));
    if let Some(box2) = g.inner {
        window.paint_quad(outline(box2.bounds(), inner, BorderStyle::Solid));
    }
    for center in g.studs {
        diamond(window, center, STUD + 1.5, fill);
        diamond(window, center, STUD, stud);
    }
}

/// The tarot frame for moments that matter: a hairline in `line`, a second hairline inset 3px in
/// `orbit`, and gold diamond studs at the corners. `fill` is the frame's opaque background.
/// Add padding and children to the returned element as to any `div`.
pub fn frame(t: &Tokens, line: Hsla, fill: Hsla) -> Div {
    let (inner, stud) = (t.orbit, t.gold);
    div().relative().bg(fill).child(
        canvas(|_, _, _| (), move |bounds, _, window, _| paint_frame(window, bounds, line, inner, stud, fill))
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}

/// The measured divider: two hairlines with a small diamond between them.
#[derive(Clone, Debug, PartialEq)]
pub struct DividerGeometry {
    /// The hairlines as horizontal spans; empty when there is no room for them.
    pub lines: Vec<(f32, f32)>,
    pub center: (f32, f32),
    pub half: f32,
}

pub fn divider_geometry(width: f32, height: f32, margin: f32) -> DividerGeometry {
    let center = (width / 2.0, height / 2.0);
    let lines = [(margin, center.0 - DIVIDER_GAP), (center.0 + DIVIDER_GAP, width - margin)]
        .into_iter()
        .filter(|(a, b)| b - a >= 2.0)
        .collect();
    DividerGeometry { lines, center, half: DIVIDER_DIAMOND }
}

fn paint_divider(window: &mut Window, bounds: Bounds<Pixels>, line: Hsla, gold: Hsla, margin: f32) {
    let o = Box2::of(bounds);
    let g = divider_geometry(o.w, o.h, margin);
    let y = (o.y + g.center.1).floor();
    for (a, b) in &g.lines {
        let bar = Bounds { origin: point(px(o.x + a), px(y)), size: size(px(b - a), px(1.0)) };
        window.paint_quad(fill(bar, line));
    }
    let (cx, cy) = (o.x + g.center.0, y + 0.5);
    let mut b = PathBuilder::stroke(px(1.2));
    b.move_to(point(px(cx), px(cy - g.half)));
    b.line_to(point(px(cx + g.half), px(cy)));
    b.line_to(point(px(cx), px(cy + g.half)));
    b.line_to(point(px(cx - g.half), px(cy)));
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, gold);
    }
    diamond(window, (cx, cy), 1.4, gold);
}

/// A measured divider across the available width, leaving `margin` at each end.
pub fn divider(t: &Tokens, margin: f32) -> impl IntoElement {
    let (line, gold) = (t.orbit, t.gold);
    canvas(|_, _, _| (), move |bounds, _, window, _| paint_divider(window, bounds, line, gold, margin))
        .w_full()
        .h(px(DIVIDER_H))
        .flex_none()
}

fn arc(cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, steps: usize) -> Vec<Point<Pixels>> {
    (0..=steps)
        .map(|i| {
            let a = (from_deg + (to_deg - from_deg) * i as f32 / steps as f32).to_radians();
            point(px(cx + r * a.cos()), px(cy + r * a.sin()))
        })
        .collect()
}

fn stroke(window: &mut Window, points: &[Point<Pixels>], closed: bool, width: f32, color: Hsla) {
    let mut b = PathBuilder::stroke(px(width));
    b.add_polygon(points, closed);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn dot(window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
    let bounds = Bounds { origin: point(px(cx - r), px(cy - r)), size: size(px(r * 2.0), px(r * 2.0)) };
    window.paint_quad(fill(bounds, color).corner_radii(px(r)));
}

/// Paints the shape that says a state, centred on `(cx, cy)` within radius `r`:
/// a steady star when verified, a breathing ring when running, an open eye when it needs review,
/// a broken ring with a cross when it failed, and a faint point while it waits.
/// `breath` is 0..1; pass a constant when nothing should move.
pub fn paint_glyph(window: &mut Window, cx: f32, cy: f32, r: f32, state: UnitState, color: Hsla, breath: f32) {
    let line = (r * 0.16).clamp(1.1, 1.8);
    let at = |x: f32, y: f32| point(px(cx + x * r), px(cy + y * r));
    match state {
        UnitState::Verified => {
            let mut b = PathBuilder::fill();
            let k = 0.17;
            b.move_to(at(0.0, -1.0));
            b.curve_to(at(1.0, 0.0), at(k, -k));
            b.curve_to(at(0.0, 1.0), at(k, k));
            b.curve_to(at(-1.0, 0.0), at(-k, k));
            b.curve_to(at(0.0, -1.0), at(-k, -k));
            b.close();
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        }
        UnitState::Running => {
            let ring = r * (0.62 + 0.26 * breath);
            stroke(window, &arc(cx, cy, ring, 0.0, 360.0, 40), true, line, color.opacity(0.95 - 0.45 * breath));
            dot(window, cx, cy, r * 0.3, color);
        }
        UnitState::Built => {
            let mut b = PathBuilder::stroke(px(line));
            b.move_to(at(-1.0, 0.0));
            b.curve_to(at(1.0, 0.0), at(0.0, -0.95));
            b.curve_to(at(-1.0, 0.0), at(0.0, 0.95));
            b.close();
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
            dot(window, cx, cy, r * 0.3, color);
        }
        UnitState::Failed => {
            let ring = r * 0.86;
            stroke(window, &arc(cx, cy, ring, -30.0, 120.0, 20), false, line, color);
            stroke(window, &arc(cx, cy, ring, 150.0, 300.0, 20), false, line, color);
            let c = 0.36;
            stroke(window, &[at(-c, -c), at(c, c)], false, line, color);
            stroke(window, &[at(-c, c), at(c, -c)], false, line, color);
        }
        UnitState::Pending => dot(window, cx, cy, (r * 0.26).max(1.7), color),
    }
}

/// A state glyph as an element of `side` pixels.
pub fn glyph(state: UnitState, side: f32, color: Hsla, breath: f32) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let b = Box2::of(bounds);
        paint_glyph(window, b.x + b.w / 2.0, b.y + b.h / 2.0, side * 0.42, state, color, breath);
    })
    .flex_none()
    .size(px(side))
}

fn ring_points(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<Point<Pixels>> {
    let (sin, cos) = (-20.0f32).to_radians().sin_cos();
    (0..=32)
        .map(|i| {
            let a = from + (to - from) * i as f32 / 32.0;
            let (x, y) = (r * a.cos(), r * 0.34 * a.sin());
            point(px(cx + x * cos - y * sin), px(cy + x * sin + y * cos))
        })
        .collect()
}

/// A ringed body, as the graph draws a module: a planet holding the state glyph, inside a tilted
/// ring whose near half passes in front of it. `fill` is the planet's face.
#[allow(clippy::too_many_arguments)]
pub fn paint_body(window: &mut Window, cx: f32, cy: f32, r: f32, state: UnitState, color: Hsla, ring: Hsla, fill: Hsla, breath: f32) {
    let planet = r * 0.62;
    stroke(window, &ring_points(cx, cy, r, std::f32::consts::PI, TAU), false, 1.0, ring);
    dot(window, cx, cy, planet, fill);
    stroke(window, &arc(cx, cy, planet, 0.0, 360.0, 36), true, 1.0, color.opacity(0.7));
    paint_glyph(window, cx, cy, planet * 0.74, state, color, breath);
    stroke(window, &ring_points(cx, cy, r, 0.0, std::f32::consts::PI), false, 1.0, ring);
}

pub fn body(state: UnitState, side: f32, color: Hsla, ring: Hsla, fill: Hsla, breath: f32) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let b = Box2::of(bounds);
        paint_body(window, b.x + b.w / 2.0, b.y + b.h / 2.0, side * 0.5, state, color, ring, fill, breath);
    })
    .flex_none()
    .size(px(side))
}

/// One fixed star: position in panel coordinates, radius and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub alpha: f32,
}

fn hash(a: u32, b: u32, salt: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9e37_79b1) ^ b.wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// The sparse star field behind the graph. Each star belongs to a fixed cell of the panel, so a
/// star never moves, and resizing only reveals or hides stars at the edges.
pub fn star_field(width: f32, height: f32) -> Vec<Star> {
    if width <= 0.0 || height <= 0.0 {
        return vec![];
    }
    let (cols, rows) = ((width / STAR_CELL).ceil() as u32, (height / STAR_CELL).ceil() as u32);
    let unit = |h: u32| (h % 10_000) as f32 / 10_000.0;
    let mut stars = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            if hash(col, row, 1) % 100 >= STAR_CHANCE {
                continue;
            }
            let x = (col as f32 + unit(hash(col, row, 2))) * STAR_CELL;
            let y = (row as f32 + unit(hash(col, row, 3))) * STAR_CELL;
            if x >= width || y >= height {
                continue;
            }
            let bright = unit(hash(col, row, 4));
            stars.push(Star { x, y, r: 0.6 + bright * 0.8, alpha: 0.018 + bright * (STAR_MAX_ALPHA - 0.018) });
        }
    }
    stars
}

pub fn paint_stars(window: &mut Window, origin: (f32, f32), stars: &[Star], ink: Hsla) {
    for star in stars {
        dot(window, origin.0 + star.x, origin.1 + star.y, star.r, ink.opacity(star.alpha));
    }
}

/// Paints part of a circle of radius `r` around `(cx, cy)`, as the graph's faint orbits.
pub fn paint_orbit_arc(window: &mut Window, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Hsla) {
    paint_orbit_arc_with_width(window, cx, cy, r, from_deg, to_deg, color, 1.0);
}

fn paint_orbit_arc_with_width(window: &mut Window, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Hsla, width: f32) {
    let steps = ((to_deg - from_deg).abs() / 2.0).ceil().max(8.0) as usize;
    stroke(window, &arc(cx, cy, r, from_deg, to_deg, steps), false, width, color);
}

/// Static layers shared by the backdrop and its contrast checks.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropLayers {
    pub stars: Vec<Star>,
    pub glow_alpha: f32,
    pub vignette_alpha: f32,
}

pub fn backdrop_layers(w: f32, h: f32, dark: bool) -> BackdropLayers {
    BackdropLayers { stars: star_field(w, h), glow_alpha: if dark { 0.05 } else { 0.02 }, vignette_alpha: if dark { 0.15 } else { 0.04 } }
}

/// A fixed canvas: no clock, animation, or frame requests.
pub fn backdrop(t: &Tokens, dark: bool) -> impl IntoElement {
    let (ground, glow, ink) = (t.canvas, t.gold, t.text);
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let b = Box2::of(bounds);
        let layers = backdrop_layers(b.w, b.h, dark);
        window.paint_quad(fill(bounds, ground));
        // Nested, translucent rounded rectangles approximate a diffuse central glow. Their
        // cumulative opacity at the centre is exactly glow_alpha.
        let n = 24;
        let alpha = 1.0 - (1.0 - layers.glow_alpha).powf(1.0 / n as f32);
        for i in (1..=n).rev() {
            let scale = i as f32 / n as f32;
            let (w, h) = (b.w * 0.8 * scale, b.h * 0.8 * scale);
            let rect = Box2 { x: b.x + (b.w - w) / 2.0, y: b.y + (b.h - h) / 2.0, w, h };
            window.paint_quad(fill(rect.bounds(), glow.opacity(alpha)).corner_radii(px(w.min(h) / 2.0)));
        }
        let shade: Hsla = rgb(0x000000).into();
        for (rect, angle) in [
            (Box2 { x: b.x, y: b.y, w: b.w * 0.3, h: b.h }, 90.0),
            (Box2 { x: b.x + b.w * 0.7, y: b.y, w: b.w * 0.3, h: b.h }, 270.0),
            (Box2 { x: b.x, y: b.y, w: b.w, h: b.h * 0.3 }, 180.0),
            (Box2 { x: b.x, y: b.y + b.h * 0.7, w: b.w, h: b.h * 0.3 }, 0.0),
        ] {
            let fade = linear_gradient(angle, linear_color_stop(shade.opacity(layers.vignette_alpha), 0.0), linear_color_stop(shade.opacity(0.0), 1.0));
            window.paint_quad(fill(rect.bounds(), fade));
        }
        paint_stars(window, (b.x, b.y), &layers.stars, ink);
    }).absolute().top_0().left_0().size_full()
}

pub const TITLE_FALLBACK: &str = "Palatino Linotype";

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x2fff | 0x3040..=0x30ff | 0x3100..=0x312f | 0x31a0..=0x31bf | 0x31c0..=0x31ef | 0x31f0..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x323af)
}

pub fn title_family(text: &str) -> &'static str {
    if text.chars().any(is_cjk) { "Microsoft JhengHei" } else { "Constantia" }
}

pub fn section_caps(text: &str) -> String {
    text.chars().map(|c| c.to_ascii_uppercase()).collect()
}

pub fn tracked(text: &str) -> String {
    let mut out = String::new();
    let mut previous_cjk = false;
    for c in text.chars() {
        let cjk = is_cjk(c);
        if previous_cjk && cjk { out.push('\u{200a}'); }
        out.push(c);
        previous_cjk = cjk;
    }
    out
}

struct TitleStyle {
    family: &'static str,
    fallback: &'static str,
    bold: bool,
    size: f32,
}

fn title_style(text: &str) -> TitleStyle {
    TitleStyle { family: title_family(text), fallback: TITLE_FALLBACK, bold: text.chars().any(is_cjk), size: 22.0 }
}

const LABEL_SIZE: f32 = 12.0;

fn label_text(text: &str) -> String {
    let caps = tracked(&section_caps(text));
    let mut spaced = String::new();
    let mut previous_latin = false;
    for c in caps.chars() {
        let latin = c.is_ascii_alphabetic();
        if previous_latin && latin { spaced.push('\u{2009}'); }
        spaced.push(c);
        previous_latin = latin;
    }
    spaced
}

pub fn page_title(t: &Tokens, text: impl Into<SharedString>) -> Div {
    let text = text.into();
    let style = title_style(&text);
    let mut title = div().font_family(style.family).text_color(t.text).text_size(px(style.size))
        .font_weight(if style.bold { FontWeight::BOLD } else { FontWeight::SEMIBOLD }).child(text.clone().test_text(text));
    title.text_style().font_fallbacks = Some(FontFallbacks::from_fonts(vec![style.fallback.to_string()]));
    title
}

pub fn section_label(t: &Tokens, text: &str) -> Div {
    div().flex().flex_col().text_color(t.gold).text_size(px(LABEL_SIZE)).child(label_text(text)).child(divider(t, 0.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emblem { Hourglass, Scroll, StarBook, Astrolabe, Armillary }

/// Each polyline is in the square's coordinate system; shapes keep a margin for the ink.
pub fn emblem_strokes(kind: Emblem, side: f32) -> Vec<Vec<(f32, f32)>> {
    let circle = |cx: f32, cy: f32, rx: f32, ry: f32| -> Vec<(f32, f32)> {
        (0..=48).map(|i| { let a = TAU * i as f32 / 48.0; (cx + rx * a.cos(), cy + ry * a.sin()) }).collect()
    };
    let lines: Vec<Vec<(f32, f32)>> = match kind {
        Emblem::Hourglass => vec![vec![(0.22,0.14),(0.78,0.14)], vec![(0.22,0.86),(0.78,0.86)], vec![(0.28,0.14),(0.31,0.28),(0.5,0.5),(0.69,0.72),(0.72,0.86)], vec![(0.72,0.14),(0.69,0.28),(0.5,0.5),(0.31,0.72),(0.28,0.86)], vec![(0.39,0.75),(0.5,0.65),(0.61,0.75)]],
        Emblem::Scroll => vec![vec![(0.23,0.23),(0.23,0.72),(0.29,0.79),(0.76,0.79),(0.81,0.73),(0.76,0.67),(0.68,0.67),(0.68,0.79)], vec![(0.23,0.23),(0.29,0.17),(0.73,0.17),(0.78,0.23),(0.73,0.29),(0.23,0.29)], vec![(0.36,0.41),(0.66,0.41)], vec![(0.36,0.52),(0.66,0.52)], vec![(0.36,0.63),(0.57,0.63)]],
        Emblem::StarBook => vec![vec![(0.12,0.24),(0.32,0.2),(0.5,0.27),(0.68,0.2),(0.88,0.24),(0.88,0.76),(0.68,0.72),(0.5,0.79),(0.32,0.72),(0.12,0.76),(0.12,0.24)], vec![(0.5,0.27),(0.5,0.79)], vec![(0.69,0.35),(0.72,0.43),(0.8,0.46),(0.72,0.49),(0.69,0.57),(0.66,0.49),(0.58,0.46),(0.66,0.43),(0.69,0.35)]],
        Emblem::Astrolabe => vec![circle(0.5,0.48,0.34,0.34), circle(0.5,0.48,0.25,0.25), vec![(0.5,0.1),(0.5,0.86)], vec![(0.12,0.48),(0.88,0.48)], vec![(0.28,0.7),(0.72,0.26)], vec![(0.37,0.9),(0.63,0.9)]],
        Emblem::Armillary => vec![circle(0.5,0.45,0.34,0.34), circle(0.5,0.45,0.15,0.34), circle(0.5,0.45,0.34,0.12), vec![(0.2,0.62),(0.8,0.28)], vec![(0.5,0.79),(0.5,0.9)], vec![(0.3,0.9),(0.7,0.9)]],
    };
    let side = side.max(0.0);
    lines.into_iter().map(|line| line.into_iter().map(|(x,y)| (x * side, y * side)).collect()).collect()
}

pub fn emblem(kind: Emblem, side: f32, ink: Hsla) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let b = Box2::of(bounds);
        for line in emblem_strokes(kind, side) {
            let points: Vec<_> = line.into_iter().map(|(x,y)| point(px(b.x + x), px(b.y + y))).collect();
            stroke(window, &points, false, 1.4, ink);
        }
    }).flex_none().size(px(side))
}

fn underline_points(b: Box2, lit: f32) -> Vec<Point<Pixels>> {
    if lit.clamp(0.0, 1.0) <= 0.0 { return Vec::new(); }
    (0..=32).map(|i| {
        let a = std::f32::consts::PI * (0.12 + 0.76 * i as f32 / 32.0);
        point(px(b.x + b.w / 2.0 + b.w * 0.46 * a.cos()), px(b.y + b.h * 0.12 + b.h * 0.75 * a.sin()))
    }).collect()
}

pub fn orbit_underline(ink: Hsla, lit: f32) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let lit = lit.clamp(0.0, 1.0);
        let points = underline_points(Box2::of(bounds), lit);
        if !points.is_empty() { stroke(window, &points, false, 1.0, ink.opacity(ink.a * lit)); }
    }).w_full().h(px(8.0)).flex_none()
}

pub fn ring_sweep(verified: usize, total: usize) -> Option<f32> {
    (total != 0).then(|| (verified as f64 / total as f64).min(1.0) as f32)
}

const RING_TRACK_WIDTH: f32 = 1.0;
const RING_ARC_WIDTH: f32 = 2.0;
const RING_TRACK_ALPHA: f32 = 0.35;
const RING_ARC_ALPHA: f32 = 1.0;

fn ring_arcs(t: &Tokens, sweep: f32) -> Vec<(f32, f32, f32, Hsla)> {
    let mut arcs = vec![(0.0, 360.0, RING_TRACK_WIDTH, t.orbit.opacity(RING_TRACK_ALPHA))];
    if sweep > 0.0 {
        arcs.push((-90.0, -90.0 + 360.0 * sweep, RING_ARC_WIDTH, t.gold.opacity(RING_ARC_ALPHA)));
    }
    arcs
}

pub fn progress_ring(t: &Tokens, verified: usize, total: usize, side: f32) -> Option<AnyElement> {
    let sweep = ring_sweep(verified, total)?;
    let arcs = ring_arcs(t, sweep);
    Some(canvas(|_, _, _| (), move |bounds, _, window, _| {
        let b = Box2::of(bounds);
        let (cx, cy, r) = (b.x + b.w / 2.0, b.y + b.h / 2.0, (side / 2.0 - 1.0).max(0.0));
        for &(from, to, width, color) in &arcs {
            paint_orbit_arc_with_width(window, cx, cy, r, from, to, color, width);
        }
    }).flex_none().size(px(side)).into_any_element())
}

fn gold_fill_stops(t: &Tokens) -> [LinearColorStop; 2] {
    let leaf = t.gold_leaf(super::theme::Press::Normal);
    [linear_color_stop(leaf.top, 0.0), linear_color_stop(leaf.bottom, 1.0)]
}

pub fn gold_fill(t: &Tokens) -> Background {
    let [top, bottom] = gold_fill_stops(t);
    linear_gradient(180.0, top, bottom)
}

pub fn gold_glow(t: &Tokens, strength: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color: t.gold.opacity(0.24 * strength.clamp(0.0, 1.0)), offset: Point::default(), blur_radius: px(12.0), spread_radius: px(0.0), inset: false }]
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    #[test]
    fn backdrop_is_fixed_and_keeps_stars_faint() {
        for dark in [false, true] {
            let layers = backdrop_layers(1200.0, 900.0, dark);
            assert_eq!(layers, backdrop_layers(1200.0, 900.0, dark));
            assert_eq!(layers.stars, star_field(1200.0, 900.0));
            assert!(!layers.stars.is_empty());
            assert!(layers.stars.iter().all(|s| s.alpha > 0.0 && s.alpha < STAR_MAX_ALPHA));
            assert!(layers.glow_alpha > 0.0 && layers.glow_alpha < 1.0);
            assert!(layers.vignette_alpha > 0.0 && layers.vignette_alpha < 1.0);
        }
    }

    #[test]
    fn backdrop_centre_preserves_text_contrast() {
        let luminance = |c: Rgba| {
            let linear = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let contrast = |a: Rgba, b: Rgba| { let (a,b) = (luminance(a), luminance(b)); (a.max(b) + 0.05) / (a.min(b) + 0.05) };
        assert!(contrast(rgb(0x000000).into(), rgb(0xffffff).into()) > 4.5);
        let themes = super::super::theme::both();
        let dark = themes.iter().find(|t| t.canvas.l < 0.5).expect("dark theme");
        let light = themes.iter().find(|t| t.canvas.l >= 0.5).expect("light theme");
        for (t, dark) in [(dark, true), (light, false)] {
            let a = backdrop_layers(1200.0, 900.0, dark).glow_alpha;
            let (canvas, glow) = (Rgba::from(t.canvas), Rgba::from(t.gold));
            let bg = Rgba { r: glow.r * a + canvas.r * (1.0 - a), g: glow.g * a + canvas.g * (1.0 - a), b: glow.b * a + canvas.b * (1.0 - a), a: 1.0 };
            for fg in [t.text, t.muted] {
                let ratio = contrast(Rgba::from(fg), bg);
                assert!(ratio >= 4.5, "dark={dark}, {fg:?}: {ratio}");
            }
            assert_ne!(bg, canvas);
        }
    }

    #[test]
    fn typography_respects_latin_chinese_and_mixed_text() {
        assert_eq!(section_caps("alpha 12"), "ALPHA 12");
        assert_eq!(section_caps("中文"), "中文");
        assert_eq!(section_caps("中a文2é"), "中A文2é");
        assert_eq!(tracked("Alpha 12"), "Alpha 12");
        assert_eq!(tracked("中文標題"), "中\u{200a}文\u{200a}標\u{200a}題");
        assert_eq!(label_text("中文標題"), "中\u{200a}文\u{200a}標\u{200a}題");
        assert_eq!(tracked("中文A測試2"), "中\u{200a}文A測\u{200a}試2");
        assert_eq!(title_family("Alpha"), "Constantia");
        assert_eq!(title_family("中文"), "Microsoft JhengHei");
        assert_eq!(title_family("Alpha 中"), "Microsoft JhengHei");
    }

    #[test]
    fn title_and_label_styles_follow_typography_contract() {
        for (text, bold) in [("Alpha", false), ("中文", true), ("Alpha 中", true)] {
            let style = title_style(text);
            assert_eq!(style.family, title_family(text));
            assert_eq!(style.bold, bold);
            assert!((20.0..=24.0).contains(&style.size));
            assert_eq!(style.fallback, TITLE_FALLBACK);
        }
        assert!(LABEL_SIZE >= 12.0);
        assert_eq!(label_text("alpha 中文"), "A\u{2009}L\u{2009}P\u{2009}H\u{2009}A 中\u{200a}文");
        assert_eq!(label_text("中a文2é"), tracked(&section_caps("中a文2é")));
    }

    #[test]
    fn underline_is_empty_at_rest_and_has_an_arc_when_lit() {
        let bounds = Box2 { x: 10.0, y: 20.0, w: 100.0, h: 8.0 };
        assert!(underline_points(bounds, 0.0).is_empty());
        assert!(underline_points(bounds, -1.0).is_empty());
        for lit in [0.5, 1.0] {
            let points = underline_points(bounds, lit);
            assert!(points.len() > 2);
            assert_ne!(points.first(), points.last());
            assert!(points.iter().all(|p| {
                let (x, y) = (f32::from(p.x), f32::from(p.y));
                (bounds.x..=bounds.x + bounds.w).contains(&x)
                    && (bounds.y..=bounds.y + bounds.h).contains(&y)
            }));
        }
    }

    #[test]
    fn progress_ring_arc_is_more_visible_than_track() {
        for t in super::super::theme::both() {
            let sweep = 0.5;
            let arcs = ring_arcs(&t, sweep);
            assert_eq!(arcs.len(), 2);
            let (track, gold) = (arcs[0], arcs[1]);
            assert_eq!((track.0, track.1), (0.0, 360.0));
            assert_eq!((gold.0, gold.1), (-90.0, -90.0 + 360.0 * sweep));
            assert_eq!(track.3, t.orbit.opacity(RING_TRACK_ALPHA));
            assert_eq!(gold.3, t.gold.opacity(RING_ARC_ALPHA));
            let gold_is_wider = |arcs: &[(f32, f32, f32, Hsla)]| arcs[1].2 > arcs[0].2;
            assert!(gold_is_wider(&arcs));
            assert!(gold.3.a > track.3.a);
            let mut swapped = arcs.clone();
            swapped[0].2 = gold.2;
            swapped[1].2 = track.2;
            assert!(!gold_is_wider(&swapped));
            assert_eq!(ring_arcs(&t, 0.0), vec![track]);
        }
    }

    #[test]
    fn progress_arcs_are_static_fractions() {
        for t in super::super::theme::both() {
            for (verified, total) in [(0, 7), (3, 7), (7, 7)] {
                let draw = || ring_arcs(&t, ring_sweep(verified, total).unwrap());
                let first = draw();
                std::thread::sleep(std::time::Duration::from_millis(20));
                assert_eq!(first, draw());
                assert_eq!((first[0].0, first[0].1), (0.0, 360.0));
                if verified == 0 {
                    assert_eq!(first.len(), 1);
                } else {
                    assert_eq!((first[1].0, first[1].1), (-90.0, -90.0 + 360.0 * ring_sweep(verified, total).unwrap()));
                }
            }
        }
    }

    #[test]
    fn progress_ring_and_gold_effects_follow_contract() {
        for t in super::super::theme::both() {
            assert!(progress_ring(&t, 0, 0, 20.0).is_none());
            assert!(progress_ring(&t, 1, 2, 20.0).is_some());
            let off = gold_glow(&t, 0.0);
            let full = gold_glow(&t, 1.0);
            assert!(!off.is_empty());
            assert!(off.iter().all(|shadow| shadow.color.a == 0.0));
            assert!(full.iter().any(|shadow| shadow.color.a > 0.0 && shadow.blur_radius > px(0.0)));
            assert_eq!(gold_glow(&t, 2.0), full);
            let stops = gold_fill_stops(&t);
            assert_eq!((stops[0].percentage, stops[1].percentage), (0.0, 1.0));
            assert_ne!(stops[0].color, stops[1].color);
            assert_eq!(gold_fill(&t), linear_gradient(180.0, stops[0], stops[1]));
        }
    }

    #[test]
    fn progress_sweep_handles_empty_zero_and_overfull() {
        assert_eq!(ring_sweep(0, 7), Some(0.0));
        assert_eq!(ring_sweep(3, 6), Some(0.5));
        assert_eq!(ring_sweep(7, 7), Some(1.0));
        assert_eq!(ring_sweep(8, 7), Some(1.0));
        assert_eq!(ring_sweep(0, 0), None);
    }

    #[test]
    fn emblem_geometry_is_bounded_and_distinct() {
        let kinds = [Emblem::Hourglass, Emblem::Scroll, Emblem::StarBook, Emblem::Astrolabe, Emblem::Armillary];
        let shapes: Vec<_> = kinds.into_iter().map(|kind| emblem_strokes(kind, 80.0)).collect();
        for (i, shape) in shapes.iter().enumerate() {
            assert!(!shape.is_empty());
            assert!(shape.iter().all(|line| line.len() >= 2 && line.iter().all(|&(x,y)| (0.0..=80.0).contains(&x) && (0.0..=80.0).contains(&y))));
            for other in &shapes[..i] { assert_ne!(shape, other); }
        }
    }

    #[test]
    fn the_portrait_is_the_256px_png_inside_the_window_icon() {
        let png = ico_png(ICON).expect("assets/chronos.ico must hold a PNG image");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let side = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        assert_eq!((side(16), side(20)), (256, 256), "IHDR width and height");
        assert!(Assets.load(PORTRAIT).unwrap().is_some_and(|data| data.len() == png.len()));
    }

    #[test]
    fn a_broken_icon_is_reported_not_drawn_blank() {
        assert_eq!(ico_png(&[]), None);
        assert_eq!(ico_png(b"not an icon at all"), None);
        // A directory that points past the end of the file.
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend([0, 0, 0, 0, 1, 0, 32, 0]);
        ico.extend(1000u32.to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        assert_eq!(ico_png(&ico), None);
    }

    #[test]
    fn kit_icons_still_load_through_the_app_assets() {
        assert!(Assets.load("icons/check.svg").unwrap().is_some());
        assert!(Assets.list("chronos/").unwrap().iter().any(|name| name == PORTRAIT));
    }

    #[test]
    fn the_frame_insets_its_second_hairline_three_pixels_and_studs_its_corners() {
        let outer = Box2 { x: 10.0, y: 20.0, w: 200.0, h: 80.0 };
        let g = frame_geometry(outer);
        assert_eq!(g.inner, Some(Box2 { x: 13.0, y: 23.0, w: 194.0, h: 74.0 }));
        assert_eq!(g.studs, [(10.5, 20.5), (209.5, 20.5), (209.5, 99.5), (10.5, 99.5)]);
        assert_eq!(frame_geometry(Box2 { x: 0.0, y: 0.0, w: 6.0, h: 40.0 }).inner, None);
    }

    #[test]
    fn the_divider_leaves_room_around_its_central_diamond() {
        let g = divider_geometry(300.0, 13.0, 16.0);
        assert_eq!(g.center, (150.0, 6.5));
        assert_eq!(g.lines, vec![(16.0, 150.0 - DIVIDER_GAP), (150.0 + DIVIDER_GAP, 284.0)]);
        assert!(DIVIDER_GAP > g.half, "the lines must not touch the diamond");
        assert!(divider_geometry(20.0, 13.0, 4.0).lines.is_empty());
    }

    #[test]
    fn motion_breathes_slowly_and_rests_at_zero() {
        assert!((1.5..=3.0).contains(&BREATH_SECONDS));
        assert_eq!(breath(0.0), 0.0);
        for t in [0.3, 1.1, 2.0, 7.7] {
            assert!((breath(t) - breath(t + BREATH_SECONDS)).abs() < 1e-4);
            assert!((0.0..=1.0).contains(&breath(t)));
        }
        assert!((breath(BREATH_SECONDS / 2.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_star_field_is_sparse_faint_and_fixed() {
        let small = star_field(600.0, 400.0);
        let large = star_field(1200.0, 900.0);
        assert!(!small.is_empty());
        assert!(small.iter().all(|s| s.alpha < 0.06 && s.alpha > 0.0 && s.x < 600.0 && s.y < 400.0));
        // Growing the panel keeps every star where it was.
        for star in &small {
            assert!(large.contains(star), "{star:?} moved when the panel grew");
        }
        assert_eq!(star_field(1200.0, 900.0), large);
        let per_megapixel = large.len() as f32 / (1200.0 * 900.0) * 1_000_000.0;
        assert!((40.0..=140.0).contains(&per_megapixel), "{per_megapixel} stars per megapixel");
        assert!(star_field(0.0, 500.0).is_empty());
    }
}
