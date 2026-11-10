//! The Celestial Order's ornaments, after the Chronos portrait: the tarot frame, the measured
//! divider, the state glyphs, the fixed star field and the portrait itself.
//! The geometry functions are pure, so the drawing can be tested without a window.
use super::model::UnitState;
use super::theme::Tokens;
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
        UnitState::Pending => dot(window, cx, cy, (r * 0.26).max(1.4), color.opacity(0.85)),
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
    let steps = ((to_deg - from_deg).abs() / 2.0).ceil().max(8.0) as usize;
    stroke(window, &arc(cx, cy, r, from_deg, to_deg, steps), false, 1.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

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
