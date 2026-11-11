//! The graph on the Run screen: Chronos on the left, then modules, then their units.
//! `layout` is pure and decides every position; the rest of the file draws what it returns.
mod logic;

use super::i18n::{self, tr, Lang, Table};
use logic::{glow_targets, menu_anchor_node, menu_target, mesh_view, node_center, scene_runs_needed, should_auto_refit, stage_role, MeshView};
use super::model::*;
use super::palette;
use super::motion::{self, mix, Curve, Driver, Pace, Tween};
use super::ornament;
use super::plain_text;
use super::theme::{self, Step as SpaceStep, TypeSize, Tokened as _, Tokens};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::*;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const TABLE: Table = &[
    ("Loading the plan…", "正在載入計畫…"),
    ("Try again", "重試"),
    ("Running: a unit is being worked on now", "執行中：單元目前正在處理。"),
    ("Built: a unit is ready for verification", "已建造：單元等待驗收。"),
    ("Failed: a unit needs attention", "失敗：單元需要處理。"),
    ("Verified: a unit has passed verification", "已驗收：單元通過驗收。"),
    ("Pending: a unit is waiting to start", "待處理：單元等待開始。"),
    ("verified", "已驗收"),
    ("pending", "待處理"),
    ("No wish to show", "還沒有願望"),
    ("Make a wish and its work appears here as a live graph.", "許願之後，這裡會即時畫出工作的進展。"),
    ("Make a wish", "許願"),
    ("Astra is planning…", "Astra 正在規劃…"),
    ("The critic is checking the plan…", "第二審正在檢查計畫…"),
    ("The architect is splitting the work into modules…", "正在把工作切成模組…"),
    ("Getting ready…", "正在準備…"),
    ("Waiting for the background batch…", "正在等待背景批次…"),
    ("Waiting for the usage limit to reset…", "正在等待用量額度恢復…"),
    ("It stopped before a plan was made.", "在做出計畫之前就停止了。"),
    ("Stopped before a plan was made.", "在計畫完成前就停下了。"),
    ("This run ended without a plan.", "這次執行沒有產生計畫就結束了。"),
    ("Modules and units appear here once the plan is ready.", "計畫完成後，模組和單元會出現在這裡。"),
    ("Attempt {a}", "第 {a} 次嘗試"),
    ("Module {a}", "模組 {a}"),
    ("Depends on {a}", "相依於 {a}"),
    ("{a} of {b} verified", "{b} 個裡有 {a} 個已驗收"),
    ("{a} verified units of {b}", "{b} 的 {a} 個已驗收單元"),
    ("{a} pending units of {b}", "{b} 的 {a} 個待處理單元"),
    ("Click to list them in the graph", "點一下在圖上展開"),
    ("Stage: {a}", "階段：{a}"),
    ("Click to return to the overview", "點一下回到總覽"),
    ("and {a} more", "還有 {a} 個"),
    ("Not verified", "尚未驗收"),
    ("All {a} units verified", "{a} 個單元全部驗收"),
    ("{a} of {b} units verified", "{b} 個單元中有 {a} 個已驗收"),
    ("Passed", "通過"),
    ("Not run", "未執行"),
    ("Cost", "成本"),
    ("Time", "用時"),
    ("Open report", "開啟報告"),
    ("Memory", "記憶"),
    ("Zoom out (Ctrl and the wheel)", "縮小（Ctrl 加滾輪）"),
    ("Zoom in (Ctrl and the wheel)", "放大（Ctrl 加滾輪）"),
    ("Back to true size", "回到原始大小"),
    ("Fit", "全覽"),
    ("Frame everything (F)", "把整張圖放進畫面（F）"),
];

/// Modules with more units than this fold away their quiet ones.
const FOLD_ABOVE: usize = 6;
const CARD_H: f32 = 40.0;
const CHIP_H: f32 = 24.0;
const CHIP_MIN_W: f32 = 112.0;
const CHIP_MAX_W: f32 = 200.0;
const CHIP_GAP: f32 = 10.0;
/// Vertical distance between chips in one column; a second column sits half a step down so wires pass between.
const STEP_SINGLE: f32 = 30.0;
const STEP_DOUBLE: f32 = 32.0;
const LANE_GAP_MIN: f32 = 6.0;
const LANE_GAP_MAX: f32 = 16.0;
const EDGE_PAD: f32 = 14.0;
/// How far a card may sit below the top of its lane, so tall lanes keep it in view.
const MAX_CARD_DROP: f32 = 36.0;
const SCROLLBAR_ROOM: f32 = 12.0;
const REVEAL_MARGIN: f32 = 24.0;
const OVERSCAN: f32 = 240.0;
const PARTICLE_SPEED: f32 = 90.0;
/// Seconds for the core's small planet to go once around the hourglass.
const ORBIT_SECONDS: f32 = 3.0;
/// A core wire disappears when its module is this far beyond the panel, so scrolling never leaves a bundle of lines.
const WIRE_FADE_DISTANCE: f32 = 120.0;
const FADE_H: f32 = 22.0;
/// How far out and in the graph can be zoomed. The floor is low enough to frame a 900-unit run.
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 3.0;
/// One press of a zoom button or key.
const ZOOM_STEP: f32 = 1.25;
/// Zoom per pixel of Ctrl+wheel: one wheel notch (about 20 px) is roughly 10%.
const WHEEL_ZOOM_RATE: f32 = 0.005;
/// Lines one wheel notch scrolls when the wheel reports lines instead of pixels.
const WHEEL_LINE: f32 = 20.0;
/// However far the graph is panned, this much of it stays in the panel.
const KEEP_VISIBLE: f32 = 80.0;
const FIT_PAD: f32 = 24.0;
/// The zoom controls' box at the panel's bottom-right, and its gap from the panel's edge. The width
/// has a little slack for the longer English label.
const CONTROLS_W: f32 = 200.0;
const CONTROLS_H: f32 = 36.0;
const CONTROLS_INSET: f32 = 10.0;
/// Clear space kept between the controls and anything else drawn near them.
const CONTROLS_GAP: f32 = 12.0;
/// The strip along the bottom that Fit and keyboard reveal leave free for the controls.
const CONTROLS_BAND: f32 = CONTROLS_H + CONTROLS_INSET + CONTROLS_GAP;
const TIP_W: f32 = 260.0;
/// A tooltip is cut off here, so its placement can be worked out before it is measured.
const TIP_MAX_H: f32 = 220.0;
const TIP_EDGE: f32 = 8.0;
/// A press that moves less than this is a click, not a drag.
const DRAG_SLOP: f32 = 3.0;
/// Below this chip height on screen, chips and cards are drawn without words.
const LABEL_MIN_CHIP_H: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pt {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }

    fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    fn union(&self, other: &Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Rect { x, y, w: self.right().max(other.right()) - x, h: self.bottom().max(other.bottom()) - y }
    }
}

/// How the graph is looked at. The layout is in graph units; a point is drawn at
/// `graph * zoom + pan`, in pixels from the panel's top left. Zoom 1 with no pan is the graph
/// exactly as laid out, and a vertical pan is what scrolling used to be.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    zoom: f32,
    pan: Pt,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, pan: Pt::default() }
    }
}

impl View {
    fn to_screen(&self, p: Pt) -> Pt {
        Pt { x: p.x * self.zoom + self.pan.x, y: p.y * self.zoom + self.pan.y }
    }

    fn to_graph(&self, p: Pt) -> Pt {
        Pt { x: (p.x - self.pan.x) / self.zoom, y: (p.y - self.pan.y) / self.zoom }
    }

    fn rect(&self, r: Rect) -> Rect {
        let at = self.to_screen(Pt { x: r.x, y: r.y });
        Rect { x: at.x, y: at.y, w: r.w * self.zoom, h: r.h * self.zoom }
    }

    /// Zoomed by `factor` about a point on screen, which keeps the same spot of the graph under it.
    fn zoomed_about(self, anchor: Pt, factor: f32) -> View {
        let zoom = (self.zoom * factor).clamp(self.zoom.min(MIN_ZOOM), MAX_ZOOM);
        let under = self.to_graph(anchor);
        View { zoom, pan: Pt { x: anchor.x - under.x * zoom, y: anchor.y - under.y * zoom } }
    }

    fn panned(self, dx: f32, dy: f32) -> View {
        View { zoom: self.zoom, pan: Pt { x: self.pan.x + dx, y: self.pan.y + dy } }
    }
}

/// The zoom factor for a Ctrl+wheel movement: up zooms in, down out, by the same ratio.
fn wheel_zoom(dy: f32) -> f32 {
    (dy * WHEEL_ZOOM_RATE).exp()
}

/// Keeps a pan on one axis within reach: at least `KEEP_VISIBLE` of the content (or all of it,
/// when smaller) stays in the panel.
fn clamp_axis(pan: f32, lo: f32, hi: f32, zoom: f32, panel: f32) -> f32 {
    let keep = KEEP_VISIBLE.min((hi - lo) * zoom).min(panel).max(0.0);
    let (min, max) = (keep - hi * zoom, panel - keep - lo * zoom);
    if min > max {
        (min + max) / 2.0
    } else {
        pan.clamp(min, max)
    }
}

/// The view with the graph kept in reach and the zoom within its range.
fn clamp_view(view: View, content: Option<Rect>, width: f32, height: f32) -> View {
    // An initial fit can be smaller than the interactive zoom floor for very large runs.
    let zoom = view.zoom.clamp(f32::MIN_POSITIVE, MAX_ZOOM);
    let Some(c) = content else {
        return View { zoom, ..view };
    };
    View {
        zoom,
        pan: Pt {
            x: clamp_axis(view.pan.x, c.x, c.right(), zoom, width),
            y: clamp_axis(view.pan.y, c.y, c.bottom(), zoom, height),
        },
    }
}

/// The view that frames all of `content` in the panel, centred, never enlarged past its true size.
fn fit_view(content: Rect, width: f32, height: f32) -> View {
    // The bottom strip belongs to the zoom controls, so the graph is framed above it.
    let height = (height - CONTROLS_BAND).max(1.0);
    let zoom = ((width - FIT_PAD * 2.0) / content.w.max(1.0))
        .min((height - FIT_PAD * 2.0) / content.h.max(1.0))
        .clamp(MIN_ZOOM, 1.0);
    View {
        zoom,
        pan: Pt {
            x: (width - content.w * zoom) / 2.0 - content.x * zoom,
            y: (height - content.h * zoom) / 2.0 - content.y * zoom,
        },
    }
}

/// Where the zoom controls sit in a panel of this size.
fn controls_rect(width: f32, height: f32) -> Rect {
    Rect {
        x: width - CONTROLS_INSET - CONTROLS_W,
        y: height - CONTROLS_INSET - CONTROLS_H,
        w: CONTROLS_W,
        h: CONTROLS_H,
    }
}

/// Where a tooltip goes: its corner, and whether that corner is its bottom-left (it grows upward).
#[derive(Clone, Copy, Debug, PartialEq)]
struct TipPlace {
    at: Pt,
    from_bottom: bool,
}

/// Where a tooltip placed at `place` really ends up: the window keeps it `TIP_EDGE` inside its
/// edges (the panel stands for the window here), so a corner near the right or bottom edge is
/// pushed back in. It is measured at its tallest, which is the safe side.
fn tip_rect(place: TipPlace, width: f32, height: f32) -> Rect {
    let top = if place.from_bottom { place.at.y - TIP_MAX_H } else { place.at.y };
    Rect {
        x: place.at.x.min(width - TIP_W - TIP_EDGE).max(TIP_EDGE),
        y: top.min(height - TIP_MAX_H - TIP_EDGE).max(TIP_EDGE),
        w: TIP_W,
        h: TIP_MAX_H,
    }
}

/// Moves a tooltip off the zoom controls. `at` is where it would go, `node` is what it describes
/// (both in panel pixels). Above the controls is tried first, then beside them; the node stays
/// uncovered when either allows it.
fn place_tip(at: Pt, node: Rect, width: f32, height: f32) -> TipPlace {
    let c = controls_rect(width, height);
    let keep_out = Rect { x: c.x - CONTROLS_GAP, y: c.y - CONTROLS_GAP, w: c.w + CONTROLS_GAP * 2.0, h: c.h + CONTROLS_GAP * 2.0 };
    // Every check is made on where the window leaves the tooltip, not where it was asked for.
    let shown = |at: Pt, from_bottom: bool| tip_rect(TipPlace { at, from_bottom }, width, height);
    if !shown(at, false).overlaps(&keep_out) {
        return TipPlace { at, from_bottom: false };
    }
    let above = TipPlace { at: Pt { x: at.x, y: keep_out.y }, from_bottom: true };
    if keep_out.y - TIP_MAX_H >= TIP_EDGE && !shown(above.at, true).overlaps(&node) && !shown(above.at, true).overlaps(&keep_out) {
        return above;
    }
    let left = TipPlace { at: Pt { x: keep_out.x - TIP_W, y: at.y }, from_bottom: false };
    if left.at.x >= TIP_EDGE && !shown(left.at, false).overlaps(&node) && !shown(left.at, false).overlaps(&keep_out) {
        return left;
    }
    above
}

/// The view that brings `rect` (in graph units) on screen, leaving the view alone when it already is.
fn reveal_view(view: View, rect: Rect, width: f32, height: f32) -> View {
    let s = view.rect(rect);
    let mut pan = view.pan;
    if s.y < REVEAL_MARGIN || s.bottom() > height - CONTROLS_BAND {
        pan.y += height / 2.0 - s.cy();
    }
    if s.x < 0.0 || s.right() > width {
        pan.x += width / 2.0 - (s.x + s.w / 2.0);
    }
    View { zoom: view.zoom, pan }
}

/// A thing on the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Core,
    Module(usize),
    Unit(usize),
    /// Quiet units folded into one node: the module, and the state they share.
    Folded(usize, UnitState),
}

/// A cubic from `from` to the knee, then a straight run to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wire {
    from: Pt,
    knee: f32,
    to: Pt,
}

impl Wire {
    /// The two control points of the curve, and where it ends.
    fn curve(&self) -> [Pt; 3] {
        let dx = (self.knee - self.from.x) * 0.55;
        [
            Pt { x: self.from.x + dx, y: self.from.y },
            Pt { x: self.knee - dx, y: self.to.y },
            Pt { x: self.knee, y: self.to.y },
        ]
    }

    fn curve_len(&self) -> f32 {
        (self.knee - self.from.x).hypot(self.to.y - self.from.y)
    }

    fn length(&self) -> f32 {
        self.curve_len() + (self.to.x - self.knee)
    }

    fn at(&self, t: f32) -> Pt {
        let [c1, c2, end] = self.curve();
        let share = self.curve_len() / self.length().max(f32::EPSILON);
        if t < share {
            let s = t / share;
            let u = 1.0 - s;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s);
            Pt {
                x: a * self.from.x + b * c1.x + c * c2.x + d * end.x,
                y: a * self.from.y + b * c1.y + c * c2.y + d * end.y,
            }
        } else {
            let s = (t - share) / (1.0 - share).max(f32::EPSILON);
            Pt { x: end.x + (self.to.x - end.x) * s, y: end.y }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ModuleBox {
    index: usize,
    rect: Rect,
    state: UnitState,
    running: bool,
    verified: usize,
    total: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct ChipBox {
    node: Node,
    module: usize,
    rect: Rect,
    state: UnitState,
    /// Units this chip stands for: one, or the number folded.
    count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct Graph {
    width: f32,
    height: f32,
    core_x: f32,
    core_r: f32,
    /// Where the fan of wires ends and straight runs into the second column begin.
    knee: f32,
    live: bool,
    modules: Vec<ModuleBox>,
    chips: Vec<ChipBox>,
}

/// Column positions for a given panel width.
struct Columns {
    core_x: f32,
    core_r: f32,
    card_x: f32,
    card_w: f32,
    chip_x: [f32; 2],
    chip_w: f32,
    count: usize,
}

fn columns(width: f32, min_card_w: f32) -> Columns {
    let pad = (width * 0.03).clamp(8.0, 20.0);
    let core_r = if width < 420.0 { 15.0 } else { 26.0 };
    let reach = (width * 0.085).clamp(14.0, 96.0);
    let card_w = (width * 0.26).clamp(92.0, 220.0).max(min_card_w);
    let fan = (width * 0.08).clamp(16.0, 96.0);
    let core_x = pad + core_r;
    let card_x = pad + core_r * 2.0 + reach;
    let chips_at = card_x + card_w + fan;
    let room = (width - chips_at - SCROLLBAR_ROOM - pad).max(64.0);
    let count = if room >= CHIP_MIN_W * 2.0 + CHIP_GAP { 2 } else { 1 };
    let chip_w = ((room - CHIP_GAP * (count - 1) as f32) / count as f32).min(CHIP_MAX_W);
    let used = chip_w * count as f32 + CHIP_GAP * (count - 1) as f32;
    let spare = (room - used).max(0.0);
    let card_x = card_x + spare * 0.4;
    let chips_at = chips_at + spare;
    Columns {
        core_x,
        core_r,
        card_x,
        card_w,
        chip_x: [chips_at, chips_at + chip_w + CHIP_GAP],
        chip_w,
        count,
    }
}

/// The aggregate state of a module. Work in progress outranks a failure beside it; the failed chip still shows.
fn module_state(run: &Run, module: &Module) -> UnitState {
    let has = |state| module.units.iter().any(|&u| run.units[u].state == state);
    if has(UnitState::Running) {
        UnitState::Running
    } else if has(UnitState::Failed) {
        UnitState::Failed
    } else if has(UnitState::Built) {
        UnitState::Built
    } else if has(UnitState::Pending) || module.units.is_empty() {
        UnitState::Pending
    } else {
        UnitState::Verified
    }
}

/// The chips of one module: its units in order, with quiet ones folded when the module is large.
fn lane_items(run: &Run, index: usize, selection: &Selection) -> Vec<(Node, UnitState, usize)> {
    let module = &run.modules[index];
    let keep_all = module.units.len() <= FOLD_ABOVE || *selection == Selection::Module(index);
    let folded_count = |state: UnitState| {
        let n = module
            .units
            .iter()
            .filter(|&&u| run.units[u].state == state && *selection != Selection::Unit(u))
            .count();
        if keep_all || n < 2 { 0 } else { n }
    };
    let (verified, pending) = (folded_count(UnitState::Verified), folded_count(UnitState::Pending));
    let mut items: Vec<(Node, UnitState, usize)> = module
        .units
        .iter()
        .filter(|&&u| {
            let state = run.units[u].state;
            let folded = (state == UnitState::Verified && verified > 0) || (state == UnitState::Pending && pending > 0);
            !folded || *selection == Selection::Unit(u)
        })
        .map(|&u| (Node::Unit(u), run.units[u].state, 1))
        .collect();
    if verified > 0 {
        items.push((Node::Folded(index, UnitState::Verified), UnitState::Verified, verified));
    }
    if pending > 0 {
        items.push((Node::Folded(index, UnitState::Pending), UnitState::Pending, pending));
    }
    items
}

/// Lays the run out inside a panel of the given size. Pure: the same inputs give the same graph.
fn layout(run: &Run, width: f32, view_h: f32, selection: &Selection) -> Graph {
    layout_with_min_card(run, width, view_h, selection, 0.0)
}

/// Choose the same layout for painting and fitting a panel.
fn panel_layout(run: &Run, width: f32, view_h: f32, selection: &Selection) -> Graph {
    let graph = layout(run, width, view_h, selection);
    if width < 720.0 && graph.content().is_some_and(|c| c.h > (view_h - CONTROLS_BAND).max(0.0)) {
        layout_with_min_card(run, 720.0, view_h, selection, 240.0)
    } else {
        graph
    }
}

fn layout_with_min_card(run: &Run, width: f32, view_h: f32, selection: &Selection, min_card_w: f32) -> Graph {
    let cols = columns(width, min_card_w);
    let slot = |i: usize| -> (usize, f32) {
        match cols.count {
            1 => (0, i as f32 * STEP_SINGLE),
            _ => (i % 2, (i / 2) as f32 * STEP_DOUBLE + (i % 2) as f32 * STEP_DOUBLE / 2.0),
        }
    };
    let lanes: Vec<_> = (0..run.modules.len())
        .map(|index| {
            let items = lane_items(run, index, selection);
            let items_h = (0..items.len()).map(|i| slot(i).1 + CHIP_H).fold(0.0, f32::max);
            (items, items_h, items_h.max(CARD_H))
        })
        .collect();
    let total: f32 = lanes.iter().map(|(_, _, h)| h).sum();
    let gaps = lanes.len().saturating_sub(1) as f32;
    let gap = if gaps > 0.0 {
        ((view_h - EDGE_PAD * 2.0 - total) / gaps).clamp(LANE_GAP_MIN, LANE_GAP_MAX)
    } else {
        0.0
    };
    let content = total + gap * gaps + EDGE_PAD * 2.0;
    let mut y = EDGE_PAD + ((view_h - content) / 2.0).max(0.0);
    let mut modules = Vec::with_capacity(lanes.len());
    let mut chips = Vec::new();
    for (index, (items, items_h, lane_h)) in lanes.into_iter().enumerate() {
        let module = &run.modules[index];
        modules.push(ModuleBox {
            index,
            rect: Rect { x: cols.card_x, y: y + ((lane_h - CARD_H) / 2.0).min(MAX_CARD_DROP), w: cols.card_w, h: CARD_H },
            state: module_state(run, module),
            running: module.units.iter().any(|&u| run.units[u].state == UnitState::Running),
            verified: run.module_done(module),
            total: module.units.len(),
        });
        let items_y = y + (lane_h - items_h) / 2.0;
        for (i, (node, state, count)) in items.into_iter().enumerate() {
            let (column, dy) = slot(i);
            chips.push(ChipBox {
                node,
                module: index,
                rect: Rect { x: cols.chip_x[column], y: items_y + dy, w: cols.chip_w, h: CHIP_H },
                state,
                count,
            });
        }
        y += lane_h + gap;
    }
    Graph {
        width,
        height: content.max(view_h),
        core_x: cols.core_x,
        core_r: cols.core_r,
        knee: cols.chip_x[0] - 8.0,
        live: run.is_live(),
        modules,
        chips,
    }
}

impl Graph {
    /// Where the core is drawn. It follows the view sideways but stays at the middle of the panel
    /// while the rest pans up and down, so it is never lost.
    fn core(&self, view_h: f32, view: &View) -> Pt {
        Pt { x: self.core_x * view.zoom + view.pan.x, y: view_h / 2.0 }
    }

    /// The core's radius on screen: it scales with the view, within limits that keep the hourglass legible.
    fn core_radius(&self, view: &View) -> f32 {
        self.core_r * view.zoom.clamp(0.6, 1.6)
    }

    /// A core wire, in panel pixels.
    fn core_wire(&self, m: &ModuleBox, view_h: f32, view: &View) -> Wire {
        let from = self.core(view_h, view);
        let card = view.rect(m.rect);
        Wire {
            from: Pt { x: from.x + self.core_radius(view), y: from.y },
            knee: card.x,
            to: Pt { x: card.x, y: card.cy() },
        }
    }

    /// A wire from a module to one of its chips, in panel pixels.
    fn chip_wire(&self, chip: &ChipBox, view: &View) -> Wire {
        let card = self.modules[chip.module].rect;
        let knee = if chip.rect.x > self.knee + 8.5 { self.knee } else { chip.rect.x };
        let from = view.to_screen(Pt { x: card.right(), y: card.cy() });
        let to = view.to_screen(Pt { x: chip.rect.x, y: chip.rect.cy() });
        Wire { from, knee: knee * view.zoom + view.pan.x, to }
    }

    /// What is under a point in panel pixels, seen through `view`.
    fn hit(&self, p: Pt, view_h: f32, view: &View) -> Option<Node> {
        let core = self.core(view_h, view);
        if (p.x - core.x).hypot(p.y - core.y) <= self.core_radius(view) + 4.0 {
            return Some(Node::Core);
        }
        let at = view.to_graph(p);
        self.chips
            .iter()
            .find(|c| c.rect.contains(at))
            .map(|c| c.node)
            .or_else(|| self.modules.iter().find(|m| m.rect.contains(at)).map(|m| Node::Module(m.index)))
    }

    /// Everything that pans, in graph units: the cards, the chips and the core's column.
    fn content(&self) -> Option<Rect> {
        let mut rects = self.modules.iter().map(|m| m.rect).chain(self.chips.iter().map(|c| c.rect));
        let first = rects.next()?;
        let all = rects.fold(first, |acc, r| acc.union(&r));
        let left = self.core_x - self.core_r;
        Some(Rect { x: left.min(all.x), y: all.y, w: all.right() - left.min(all.x), h: all.h })
    }

    /// The chips of one module, in the order they are laid out.
    fn chips_of(&self, module: usize) -> impl Iterator<Item = &ChipBox> {
        self.chips.iter().filter(move |c| c.module == module)
    }

    /// Where an arrow key moves the keyboard cursor: right from a module to its first unit, left
    /// back to the module and then the core, up and down between siblings. A move with nowhere
    /// to go stays put. With no cursor, or one on something no longer drawn, it starts at the
    /// first module.
    fn step(&self, from: Option<Node>, step: Step) -> Option<Node> {
        let first = || self.modules.first().map(|m| Node::Module(m.index)).or(Some(Node::Core));
        let Some(from) = from.filter(|&n| n == Node::Core || self.rect_of(n).is_some()) else {
            return first();
        };
        let to = match (from, step) {
            (Node::Core, Step::Right | Step::Down) => first(),
            (Node::Core, _) => None,
            (Node::Module(_), Step::Left) => Some(Node::Core),
            (Node::Module(m), Step::Right) => self.chips_of(m).next().map(|c| c.node),
            (Node::Module(m), Step::Up) => m.checked_sub(1).map(Node::Module),
            (Node::Module(m), Step::Down) => (m + 1 < self.modules.len()).then_some(Node::Module(m + 1)),
            (chip, step) => {
                let module = self.module_of(chip)?;
                let siblings: Vec<Node> = self.chips_of(module).map(|c| c.node).collect();
                let at = siblings.iter().position(|&n| n == chip)?;
                match step {
                    Step::Left => Some(Node::Module(module)),
                    Step::Right => None,
                    Step::Up => at.checked_sub(1).map(|i| siblings[i]),
                    Step::Down => siblings.get(at + 1).copied(),
                }
            }
        };
        to.or(Some(from))
    }

    fn rect_of(&self, node: Node) -> Option<Rect> {
        match node {
            Node::Module(m) => self.modules.get(m).map(|b| b.rect),
            Node::Core => None,
            _ => self.chips.iter().find(|c| c.node == node).map(|c| c.rect),
        }
    }

    fn module_of(&self, node: Node) -> Option<usize> {
        match node {
            Node::Core => None,
            Node::Module(m) | Node::Folded(m, _) => Some(m),
            _ => self.chips.iter().find(|c| c.node == node).map(|c| c.module),
        }
    }
}

/// An arrow key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Up,
    Down,
    Left,
    Right,
}

/// What a node selects when it is clicked or opened with Enter. A fold opens its module.
fn selection_of(node: Option<Node>) -> Selection {
    match node {
        Some(Node::Module(m)) | Some(Node::Folded(m, _)) => Selection::Module(m),
        Some(Node::Unit(u)) => Selection::Unit(u),
        Some(Node::Core) | None => Selection::None,
    }
}

/// Fit the laid-out content above the controls, without enlarging small runs.
fn initial_view(graph: &Graph, width: f32, view_h: f32) -> View {
    let Some(content) = graph.content() else { return View::default() };
    let height = (view_h - CONTROLS_BAND).max(f32::MIN_POSITIVE);
    let mut zoom = (width / content.w.max(f32::MIN_POSITIVE)).min(height / content.h.max(f32::MIN_POSITIVE)).min(1.0);
    // The core stays legible below 60% instead of shrinking with the cards.
    // Account for its actual radius when horizontal space is the limiting axis.
    let radius = graph.core_r * zoom.clamp(0.6, 1.6);
    let left = (content.x * zoom).min(graph.core_x * zoom - radius);
    if content.right() * zoom - left > width && zoom < 0.6 {
        zoom = zoom.min(((width - graph.core_r * 0.6) / (content.right() - graph.core_x)).max(f32::MIN_POSITIVE));
    }
    if zoom == 1.0 && content.x >= 0.0 && content.right() <= width && content.y >= 0.0 && content.bottom() <= height {
        return View::default();
    }
    let left = (content.x * zoom).min(graph.core_x * zoom - graph.core_r * zoom.clamp(0.6, 1.6));
    View { zoom, pan: Pt {
        x: (width - (content.right() * zoom - left)) / 2.0 - left,
        y: (height - content.h * zoom) / 2.0 - content.y * zoom,
    } }
}

fn selected_node(selection: &Selection) -> Option<Node> {
    match selection {
        Selection::None => None,
        Selection::Module(m) => Some(Node::Module(*m)),
        Selection::Unit(u) => Some(Node::Unit(*u)),
    }
}

/// Whether a wire lies on the path to `focus`: the core wire of a module, or the wire to a chip.
fn on_core_wire(graph: &Graph, focus: Node, module: usize) -> bool {
    graph.module_of(focus) == Some(module) || focus == Node::Core
}

fn on_chip_wire(focus: Node, chip: &ChipBox) -> bool {
    focus == chip.node || focus == Node::Module(chip.module)
}

fn animates(run: &Run, reduce_motion: bool) -> bool {
    run.is_live() && !reduce_motion
}

/// How long each ceremony lasts, in seconds: inside the 1.2 to 2 s the design allows.
const CEREMONY_SECONDS: f32 = 1.6;

/// What the ceremonies need to know about a moment in a ceremony: 0..1 through it, or `None` when
/// it is over (or has not begun), so a settled graph is drawn exactly as a static one.
fn ceremony_progress(elapsed: Duration) -> Option<f32> {
    let p = elapsed.as_secs_f32() / CEREMONY_SECONDS;
    (p < 1.0).then_some(p)
}

fn ease(p: f32) -> f32 {
    p * p * (3.0 - 2.0 * p)
}

/// How visible the orbit is: fading in during its ceremony, fully drawn at rest.
fn orbit_visibility(progress: Option<f32>) -> f32 {
    progress.map_or(1.0, ease)
}

/// The hourglass's turn: one full turn, so it ends exactly as it began.
fn hourglass_turn(progress: Option<f32>) -> f32 {
    progress.map_or(0.0, |p| ease(p) * std::f32::consts::TAU)
}

/// Which runs the ceremonies have already seen, so they play for what happens while the app is
/// open and never for what was already true when it was first looked at.
#[derive(Default)]
struct Watch {
    known: HashMap<SharedString, RunStatus>,
}

#[derive(Clone, Debug, PartialEq)]
enum Moment {
    /// A wish started from this app has appeared.
    Started(SharedString),
    /// A run that was seen unfinished has finished.
    Ended(SharedString),
}

impl Watch {
    fn observe(&mut self, runs: &[Run], renames: &HashMap<SharedString, SharedString>) -> Vec<Moment> {
        let mut moments = vec![];
        let mut now = HashMap::with_capacity(runs.len());
        for run in runs {
            // A wish keeps its place when its provisional id is replaced by the real one.
            let before = self.known.get(&run.session).or_else(|| renames.get(&run.session).and_then(|old| self.known.get(old)));
            match before {
                None if run.hosted && run.status != RunStatus::Done => moments.push(Moment::Started(run.session.clone())),
                Some(&was) if was != RunStatus::Done && run.status == RunStatus::Done => {
                    moments.push(Moment::Ended(run.session.clone()))
                }
                _ => {}
            }
            now.insert(run.session.clone(), run.status);
        }
        self.known = now;
        moments
    }
}

/// The two ceremonies and when they play.
#[derive(Default)]
struct Ceremonies {
    watch: Watch,
    /// The wish whose orbit has yet to fade in, and when it began to (the first frame the graph exists).
    orbit: Option<(SharedString, Option<Instant>)>,
    turn: Option<(SharedString, Instant)>,
}

impl Ceremonies {
    fn observe(&mut self, runs: &[Run], renames: &HashMap<SharedString, SharedString>, reduce_motion: bool, now: Instant) {
        // A ceremony belongs to the wish, not to the id it had at the time: when the provisional id
        // is replaced by the real one, the orbit and the turn move over with it.
        for run in runs {
            let Some(old) = renames.get(&run.session) else { continue };
            if let Some((owner, _)) = self.orbit.as_mut().filter(|(owner, _)| owner == old) {
                *owner = run.session.clone();
            }
            if let Some((owner, _)) = self.turn.as_mut().filter(|(owner, _)| owner == old) {
                *owner = run.session.clone();
            }
        }
        for moment in self.watch.observe(runs, renames) {
            if reduce_motion {
                continue;
            }
            match moment {
                Moment::Started(session) => self.orbit = Some((session, None)),
                Moment::Ended(session) => self.turn = Some((session, now)),
            }
        }
    }

    /// How far through the orbit's fade the graph of `session` is; starts the clock on the first call.
    fn orbit(&mut self, session: &SharedString, reduce_motion: bool, now: Instant) -> Option<f32> {
        let (owner, started) = self.orbit.as_mut()?;
        if owner != session {
            return None;
        }
        let progress = ceremony_progress(now.saturating_duration_since(*started.get_or_insert(now)));
        if progress.is_none() || reduce_motion {
            self.orbit = None;
            return None;
        }
        progress
    }

    fn turn(&mut self, session: &SharedString, reduce_motion: bool, now: Instant) -> Option<f32> {
        let (owner, at) = self.turn.as_ref()?;
        let progress = ceremony_progress(now.saturating_duration_since(*at));
        if progress.is_none() || reduce_motion {
            self.turn = None;
            return None;
        }
        (owner == session).then_some(progress).flatten()
    }

    /// True while a frame must still be drawn for this run's ceremonies.
    fn playing(&self, session: &SharedString, reduce_motion: bool, now: Instant) -> bool {
        if reduce_motion {
            return false;
        }
        let orbit = self.orbit.as_ref().is_some_and(|(owner, started)| {
            owner == session && started.map_or(true, |at| ceremony_progress(now.saturating_duration_since(at)).is_some())
        });
        let turn = self
            .turn
            .as_ref()
            .is_some_and(|(owner, at)| owner == session && ceremony_progress(now.saturating_duration_since(*at)).is_some());
        orbit || turn
    }
}

/// Where along a wire particle `i` of `count` is at a given time, from 0 at the start to 1 at the end.
fn particle_t(time: f32, length: f32, i: usize, count: usize) -> f32 {
    (time * PARTICLE_SPEED / length.max(1.0) + i as f32 / count as f32).fract()
}

fn particle_count(length: f32) -> usize {
    ((length / 70.0).ceil() as usize).clamp(2, 8)
}

/// How visible a core wire is, by how far its module has scrolled out of the panel.
fn offscreen_fade(y: f32, view_h: f32) -> f32 {
    let beyond = (-y).max(y - view_h).max(0.0);
    (1.0 - beyond / WIRE_FADE_DISTANCE).clamp(0.0, 1.0)
}

fn fmt_ab(text: &str, a: impl ToString, b: impl ToString) -> String {
    text.replace("{a}", &a.to_string()).replace("{b}", &b.to_string())
}

fn fmt_a(text: &str, a: impl ToString) -> String {
    text.replace("{a}", &a.to_string())
}

fn stage_key(name: &str) -> Option<&'static str> {
    ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report"]
        .into_iter()
        .find(|key| *key == name)
}

fn stage_label(cx: &App, name: &SharedString) -> SharedString {
    match stage_key(name) {
        Some(key) => tr(cx, TABLE, key).into(),
        None => name.clone(),
    }
}

fn current_stage(run: &Run) -> Option<&Stage> {
    run.stages.iter().find(|s| s.phase == Phase::Running)
}

fn planning_line(run: &Run) -> &'static str {
    match run.status {
        RunStatus::Blocked => return "Stopped before a plan was made.",
        RunStatus::Waiting => return "Waiting for the background batch…",
        RunStatus::LimitWait => return "Waiting for the usage limit to reset…",
        RunStatus::Interrupted => return "It stopped before a plan was made.",
        RunStatus::Done => return "This run ended without a plan.",
        RunStatus::Active => {}
    }
    match current_stage(run).map(|s| s.name.as_ref()) {
        Some("Astra") => "Astra is planning…",
        Some("Critic") => "The critic is checking the plan…",
        Some("Architect") => "The architect is splitting the work into modules…",
        _ => "Getting ready…",
    }
}

/// How strongly a wire is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Lit {
    No,
    Selected,
    Hovered,
}

fn state_rank(state: UnitState) -> u8 {
    match state {
        UnitState::Pending => 0,
        UnitState::Verified => 1,
        UnitState::Built => 2,
        UnitState::Failed => 3,
        UnitState::Running => 4,
    }
}

/// The strokes of a wire, widest first. Wires are `orbit` hairlines; only running work, the
/// selection and the hovered path light up, and only running work and the selection in gold.
fn wire_layers(t: &Tokens, state: UnitState, lit: Lit, dark: bool) -> Vec<(f32, Hsla)> {
    let glow = |color: Hsla, a: f32, b: f32, c: f32, width: f32| {
        if dark {
            vec![(6.0, color.opacity(a)), (3.0, color.opacity(b)), (width, color.opacity(c))]
        } else {
            vec![(3.0, color.opacity(b * 0.6)), (width, color.opacity(c))]
        }
    };
    match lit {
        Lit::Hovered => glow(t.text, 0.05, 0.14, 0.85, 1.3),
        Lit::Selected => glow(t.gold, 0.08, 0.2, 1.0, 1.4),
        Lit::No => match state {
            UnitState::Running => glow(t.gold, 0.06, 0.16, 0.9, 1.2),
            UnitState::Pending => vec![(1.0, t.orbit.opacity(if dark { 0.75 } else { 0.9 }))],
            _ => vec![(1.0, t.orbit.opacity(if dark { 0.95 } else { 1.0 }))],
        },
    }
}

fn dashed(state: UnitState, lit: Lit) -> bool {
    state == UnitState::Pending && lit == Lit::No
}

fn pt(origin: Pt, p: Pt) -> Point<Pixels> {
    point(px(origin.x + p.x), px(origin.y + p.y))
}

fn paint_wire(window: &mut Window, origin: Pt, wire: &Wire, state: UnitState, lit: Lit, layers: &[(f32, Hsla)], fade: f32) {
    for (width, color) in layers {
        let mut b = PathBuilder::stroke(px(*width));
        if dashed(state, lit) {
            b = b.dash_array(&[px(3.0), px(4.0)]);
        }
        let [c1, c2, end] = wire.curve();
        b.move_to(pt(origin, wire.from));
        b.cubic_bezier_to(pt(origin, end), pt(origin, c1), pt(origin, c2));
        if wire.to.x > wire.knee {
            b.line_to(pt(origin, wire.to));
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color.opacity(fade));
        }
    }
}

fn disc(window: &mut Window, center: Point<Pixels>, r: f32, color: Hsla) {
    let bounds = Bounds {
        origin: point(center.x - px(r), center.y - px(r)),
        size: size(px(r * 2.0), px(r * 2.0)),
    };
    window.paint_quad(fill(bounds, color).corner_radii(px(r)));
}

/// A blurred halo around a point.
fn soft_glow(window: &mut Window, center: Point<Pixels>, r: f32, blur: f32, color: Hsla) {
    let bounds = Bounds {
        origin: point(center.x - px(r), center.y - px(r)),
        size: size(px(r * 2.0), px(r * 2.0)),
    };
    window.paint_drop_shadows(
        bounds,
        Corners::all(px(r)),
        &[BoxShadow { color, offset: Point::default(), blur_radius: px(blur), spread_radius: px(0.), inset: false }],
    );
}

fn stroke_points(window: &mut Window, points: &[Point<Pixels>], closed: bool, width: f32, color: Hsla) {
    let mut b = PathBuilder::stroke(px(width));
    b.add_polygon(points, closed);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// The Chronos mark: an hourglass in a gold ring with a tilted orbit. `phase`, in radians, moves the small planet.
fn paint_emblem(window: &mut Window, center: Pt, r: f32, t: &Tokens, phase: f32, orbit_in: f32, turn: f32) {
    let at = |x: f32, y: f32| point(px(center.x + x * r), px(center.y + y * r));
    // The hourglass alone turns, about the core.
    let (turn_sin, turn_cos) = turn.sin_cos();
    let at_glass = |x: f32, y: f32| at(x * turn_cos - y * turn_sin, x * turn_sin + y * turn_cos);
    let c = at(0.0, 0.0);
    let orbit_at = |a: f32| {
        let (ex, ey) = (1.39 * a.cos(), 0.49 * a.sin());
        let (sin, cos) = (-28.0f32).to_radians().sin_cos();
        (ex * cos - ey * sin, ex * sin + ey * cos)
    };
    let orbit: Vec<_> = (0..64)
        .map(|i| {
            let (x, y) = orbit_at(i as f32 / 64.0 * std::f32::consts::TAU);
            at(x, y)
        })
        .collect();
    stroke_points(window, &orbit, true, 1.0, t.idle.opacity(0.8 * orbit_in));
    disc(window, c, r + 0.5, t.surface);
    let arc = |from: f32, to: f32| -> Vec<Point<Pixels>> {
        (0..=24)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / 24.0).to_radians();
                at(a.cos(), a.sin())
            })
            .collect()
    };
    let ring = (r * 0.09).max(1.5);
    stroke_points(window, &arc(-150.0, -30.0), false, ring, t.gold);
    stroke_points(window, &arc(30.0, 150.0), false, ring, t.gold);
    stroke_points(window, &arc(-30.0, 30.0), false, 1.0, t.idle.opacity(0.7));
    stroke_points(window, &arc(150.0, 210.0), false, 1.0, t.idle.opacity(0.7));

    let glass = |side: f32| -> PathBuilder {
        let mut b = PathBuilder::stroke(px((r * 0.07).max(1.1)));
        b.move_to(at_glass(side * 0.36, -0.60));
        b.cubic_bezier_to(at_glass(side * 0.07, -0.05), at_glass(side * 0.36, -0.30), at_glass(side * 0.22, -0.16));
        b.line_to(at_glass(side * 0.07, 0.05));
        b.cubic_bezier_to(at_glass(side * 0.36, 0.60), at_glass(side * 0.22, 0.16), at_glass(side * 0.36, 0.30));
        b
    };
    for side in [-1.0, 1.0] {
        if let Ok(path) = glass(side).build() {
            window.paint_path(path, t.text.opacity(0.9));
        }
    }
    let mut sand = PathBuilder::fill();
    sand.move_to(at_glass(-0.23, -0.40));
    sand.curve_to(at_glass(0.23, -0.40), at_glass(0.0, -0.33));
    sand.curve_to(at_glass(0.0, -0.09), at_glass(0.14, -0.20));
    sand.curve_to(at_glass(-0.23, -0.40), at_glass(-0.14, -0.20));
    sand.close();
    sand.move_to(at_glass(-0.24, 0.50));
    sand.curve_to(at_glass(0.0, 0.20), at_glass(-0.09, 0.37));
    sand.curve_to(at_glass(0.24, 0.50), at_glass(0.09, 0.37));
    sand.close();
    if let Ok(path) = sand.build() {
        window.paint_path(path, t.gold);
    }
    for y in [-0.66, 0.66] {
        stroke_points(window, &[at_glass(-0.42, y), at_glass(0.42, y)], false, (r * 0.09).max(1.4), t.gold);
    }
    let (px_, py_) = orbit_at(phase + 0.6);
    disc(window, at(px_, py_), (r * 0.09).max(1.8), t.gold.opacity(orbit_in));
}

/// Everything drawn behind the node cards.
struct Backdrop {
    graph: Rc<Graph>,
    view: View,
    t: Tokens,
    dark: bool,
    /// Seconds since the graph opened, or `None` when nothing should move.
    time: Option<f32>,
    selected: Option<Node>,
    hovered: Option<Node>,
    /// 0..1: how much of the orbit has faded in. 1 at rest.
    orbit_in: f32,
    /// Radians the hourglass has turned. 0 at rest.
    turn: f32,
}

impl Backdrop {
    fn lit(&self, core_module: Option<usize>, chip: Option<&ChipBox>) -> Lit {
        let on = |focus: Node| match (core_module, chip) {
            (Some(m), _) => on_core_wire(&self.graph, focus, m),
            (_, Some(c)) => on_chip_wire(focus, c),
            _ => false,
        };
        if self.hovered.is_some_and(on) {
            Lit::Hovered
        } else if self.selected.is_some_and(on) {
            Lit::Selected
        } else {
            Lit::No
        }
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let origin = Pt { x: f32::from(bounds.origin.x), y: f32::from(bounds.origin.y) };
        let (view_w, view_h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let graph = &self.graph;
        let view = &self.view;
        let core = graph.core(view_h, view);
        // The faint orbits stay centred on the core over the shared backdrop.
        let arc_color = self.t.orbit.opacity((if self.dark { 0.55 } else { 0.7 }) * self.orbit_in);
        for r in orbit_radii(graph) {
            ornament::paint_orbit_arc(window, origin.x + core.x, origin.y + core.y, r * view.zoom, -88.0, 88.0, arc_color);
        }
        let mut wires: Vec<(Wire, UnitState, Lit, bool, f32)> = Vec::with_capacity(graph.modules.len() * 3);
        for m in &graph.modules {
            let wire = graph.core_wire(m, view_h, view);
            let fade = offscreen_fade(wire.to.y, view_h);
            if fade > 0.0 {
                wires.push((wire, m.state, self.lit(Some(m.index), None), graph.live && m.running, fade));
            }
        }
        let panel = Rect { x: 0.0, y: 0.0, w: view_w, h: view_h };
        for chip in &graph.chips {
            let wire = graph.chip_wire(chip, view);
            let reach = Rect {
                x: wire.from.x.min(wire.to.x),
                y: wire.from.y.min(wire.to.y),
                w: (wire.to.x - wire.from.x).abs() + chip.rect.w * view.zoom,
                h: (wire.to.y - wire.from.y).abs() + CHIP_H * view.zoom,
            };
            if !reach.overlaps(&panel) {
                continue;
            }
            let flows = graph.live && chip.state == UnitState::Running;
            wires.push((wire, chip.state, self.lit(None, Some(chip)), flows, 1.0));
        }
        wires.sort_by_key(|(_, state, lit, _, _)| (*lit, state_rank(*state)));
        for (wire, state, lit, _, fade) in &wires {
            let layers = wire_layers(&self.t, *state, *lit, self.dark);
            paint_wire(window, origin, wire, *state, *lit, &layers, *fade);
        }
        for (wire, _, _, flows, fade) in &wires {
            if *flows {
                self.paint_particles(window, origin, wire, *fade);
            }
        }
        let center = Pt { x: origin.x + core.x, y: origin.y + core.y };
        let core_r = graph.core_radius(view);
        let breath = self.time.map_or(0.5, ornament::breath);
        let halo = self.t.gold.opacity(if graph.live { 0.22 + 0.16 * breath } else { 0.24 });
        soft_glow(window, point(px(center.x), px(center.y)), core_r * 0.96, core_r * 1.1, halo);
        let phase = self.time.map_or(0.0, |time| time * std::f32::consts::TAU / ORBIT_SECONDS);
        paint_emblem(window, center, core_r, &self.t, phase, self.orbit_in, self.turn);
    }

    fn paint_particles(&self, window: &mut Window, origin: Pt, wire: &Wire, fade_out: f32) {
        let count = particle_count(wire.length());
        let time = self.time.unwrap_or(0.0);
        let core = if self.dark { self.t.text } else { self.t.gold };
        for i in 0..count {
            let t = particle_t(time, wire.length(), i, count);
            let fade = (t / 0.08).min((1.0 - t) / 0.08).clamp(0.0, 1.0) * fade_out;
            let center = pt(origin, wire.at(t));
            soft_glow(window, center, 1.8, 6.0, self.t.gold.opacity(0.6 * fade));
            disc(window, center, 1.5, core.opacity(0.9 * fade));
        }
    }
}

/// Radii of the faint orbits around the core: through the module bodies, through the first
/// column of units, and one further out when the panel has room.
fn orbit_radii(graph: &Graph) -> Vec<f32> {
    let Some(card) = graph.modules.first().map(|m| m.rect) else {
        return vec![];
    };
    let inner = card.x + card.w / 2.0 - graph.core_x;
    let units = graph.chips.first().map_or(graph.knee + 8.0, |c| c.rect.x + c.rect.w / 2.0) - graph.core_x;
    let edge = graph.width - SCROLLBAR_ROOM - graph.core_x;
    let mut radii = vec![inner];
    if units > inner + 40.0 && units < edge {
        radii.push(units);
    }
    let outer = edge - 18.0;
    if outer > radii[radii.len() - 1] + 80.0 {
        radii.push(outer);
    }
    radii
}

/// The small shape that says a state without relying on color. `breath` swells the running ring.
fn state_mark(state: UnitState, size_px: f32, color: Hsla, breath: f32) -> AnyElement {
    ornament::glyph(state, size_px, color, breath).into_any_element()
}

/// A state mark that is changing: the old shape fades out under the new one as it fades in.
fn mark_between(blend: Option<(UnitState, f32)>, state: UnitState, size_px: f32, t: &Tokens, breath: f32) -> AnyElement {
    let Some((from, k)) = blend else { return state_mark(state, size_px, t.unit(state), breath) };
    div()
        .relative()
        .flex_none()
        .size(px(size_px))
        .child(div().absolute().top_0().left_0().child(state_mark(from, size_px, t.unit(from).opacity(1.0 - k), 0.0)))
        .child(div().absolute().top_0().left_0().child(state_mark(state, size_px, t.unit(state).opacity(k), breath)))
        .into_any_element()
}

/// A node's look partway from one state to the next.
fn look_between(from: &NodeLook, to: &NodeLook, k: f32) -> NodeLook {
    let glow = match (from.glow, to.glow) {
        (None, None) => None,
        (Some(a), None) => Some(a.opacity(a.a * (1.0 - k))),
        (None, Some(b)) => Some(b.opacity(b.a * k)),
        (Some(a), Some(b)) => Some(mix(a, b, k)),
    };
    NodeLook { border: mix(from.border, to.border, k), fill: mix(from.fill, to.fill, k), glow, text: mix(from.text, to.text, k) }
}

fn glow(color: Hsla, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Border, fill, glow and text colors for a node in a given state.
#[derive(Clone, Copy)]
struct NodeLook {
    border: Hsla,
    fill: Hsla,
    glow: Option<Hsla>,
    text: Hsla,
}

fn node_look(t: &Tokens, state: UnitState, base: Hsla, dark: bool) -> NodeLook {
    let color = t.unit(state);
    let tint = |c: Hsla, a: f32| base.blend(c.opacity(a));
    match state {
        UnitState::Pending => NodeLook { border: t.orbit.opacity(0.8), fill: base, glow: None, text: t.muted },
        UnitState::Verified => NodeLook { border: t.orbit, fill: base, glow: None, text: t.muted },
        UnitState::Built => NodeLook { border: color.opacity(0.6), fill: tint(color, 0.05), glow: None, text: t.text },
        UnitState::Running => NodeLook {
            border: t.gold.opacity(0.85),
            fill: tint(t.gold, if dark { 0.06 } else { 0.08 }),
            glow: Some(t.gold.opacity(if dark { 0.22 } else { 0.16 })),
            text: t.text,
        },
        UnitState::Failed => NodeLook { border: color.opacity(0.75), fill: tint(color, 0.07), glow: None, text: t.text },
    }
}

/// A press on empty space that may become a pan.
#[derive(Clone, Copy, Debug)]
struct Drag {
    start: Point<Pixels>,
    pan: Pt,
    moved: bool,
}

/// Demo only: what a scene does once the graph has been laid out.
#[derive(Clone, Copy, Debug)]
enum SceneView {
    /// Zoom by this factor about the given node, with the keyboard cursor on it.
    ZoomAt(f32, Node),
    Fit,
}

pub struct Mesh {
    state: Entity<AppState>,
    focus: FocusHandle,
    menu: Option<(Entity<PopupMenu>, Point<Pixels>, Subscription)>,
    /// The current run's view; each opening starts fitted again.
    view: View,
    drag: Option<Drag>,
    /// The last press turned into a pan, so its release is not a click.
    dragged: bool,
    /// The keyboard cursor: what Enter would open.
    cursor: Option<Node>,
    /// The last layout, kept for pointer events.
    graph: Rc<Graph>,
    origin: Point<Pixels>,
    view_w: f32,
    view_h: f32,
    inside: bool,
    hover: Option<Node>,
    select_glows: motion::Glows<Node>,
    hover_glows: motion::Glows<Node>,
    clock: Instant,
    seen_selection: Selection,
    seen_session: Option<SharedString>,
    /// Viewport of the last automatic fit, until the user changes the view or selection.
    auto_fit: Option<(f32, f32)>,
    /// Demo only: a captured scene keeps its hover whatever the real pointer does.
    pinned: bool,
    ceremonies: Ceremonies,
    /// Demo only: holds a ceremony at this point (0..1) so a still can be captured.
    frozen: Option<(Moment, f32)>,
    scene_view: Option<SceneView>,
    /// Each node's state at the last layout, and the nodes changing state now: from what, since when.
    node_states: HashMap<Node, UnitState>,
    blends: HashMap<Node, (UnitState, Instant)>,
    /// Nodes that appeared while the graph was on screen, and when; they fade in.
    arrivals: HashMap<Node, Instant>,
    blend_session: Option<SharedString>,
}

fn glyph_side(r: Rect, zoom: f32) -> f32 {
    (24.0 * zoom).clamp(8.0, 40.0).min(r.h - 4.0).max(4.0)
}

fn unit_chip_label(unit: &Unit, title: &plain_text::Plain) -> SharedString {
    if unit.title.is_empty() { unit.id.clone() } else { title.shown.clone() }
}

fn unit_tooltip_lines(unit: &Unit, title: plain_text::Plain) -> Vec<SharedString> {
    let mut lines = Vec::new();
    if unit.title != unit.id || title.original.is_some() {
        lines.push(title.shown);
    }
    lines.extend(title.original);
    lines
}

impl Mesh {
    /// Notes nodes whose state changed since the last layout, so they blend into the new state.
    /// A run seen for the first time, or reduce motion, just shows the states as they are.
    fn track_blends(&mut self, graph: &Graph, session: &SharedString, reduce: bool, now: Instant) {
        let fresh = self.blend_session.as_ref() != Some(session);
        let states: HashMap<Node, UnitState> = graph
            .modules
            .iter()
            .map(|m| (Node::Module(m.index), m.state))
            .chain(graph.chips.iter().map(|c| (c.node, c.state)))
            .collect();
        if fresh || reduce {
            self.blends.clear();
            self.arrivals.clear();
        } else {
            for (node, state) in &states {
                match self.node_states.get(node) {
                    Some(&before) if before != *state => {
                        self.blends.insert(*node, (before, now));
                    }
                    None => {
                        self.arrivals.insert(*node, now);
                    }
                    _ => {}
                }
            }
            self.blends.retain(|_, (_, at)| now.saturating_duration_since(*at) < Pace::Change.duration());
            self.arrivals.retain(|_, at| now.saturating_duration_since(*at) < Pace::Row.duration());
        }
        self.blend_session = Some(session.clone());
        self.node_states = states;
    }

    /// How visible a node is: 1, or less while it fades in after appearing.
    fn presence_of(&self, node: Node, cx: &App) -> f32 {
        let Some(&at) = self.arrivals.get(&node) else { return 1.0 };
        let reduce = self.state.read(cx).reduce_motion;
        let mut arrive = Tween::rest(0.0, Pace::Row, Curve::EaseOut);
        arrive.set(1.0, at, reduce);
        arrive.value(motion::now(cx), reduce)
    }

    /// How far a node is through its change of state, while it is changing.
    fn blend_of(&self, node: Node, cx: &App) -> Option<(UnitState, f32)> {
        let &(from, at) = self.blends.get(&node)?;
        let reduce = self.state.read(cx).reduce_motion;
        let mut change = Tween::rest(0.0, Pace::Change, Curve::Smooth);
        change.set(1.0, at, reduce);
        let k = change.value(motion::now(cx), reduce);
        (k < 1.0).then_some((from, k))
    }

    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Runs already on screen when the app opens are the baseline: they never get a ceremony.
        let mut ceremonies = Ceremonies::default();
        {
            let s = state.read(cx);
            ceremonies.watch.observe(&s.runs, &s.renames);
        }
        cx.observe(&state, |this, state, cx| {
            let s = state.read(cx);
            if s.view != Screen::Run {
                this.seen_session = None;
            }
            this.ceremonies.observe(&s.runs, &s.renames, s.reduce_motion, Instant::now());
            cx.notify();
        })
        .detach();
        let mut this = Self {
            state,
            focus: cx.focus_handle(),
            menu: None,
            view: View::default(),
            drag: None,
            dragged: false,
            cursor: None,
            graph: Rc::new(Graph {
                width: 0.0,
                height: 0.0,
                core_x: 0.0,
                core_r: 0.0,
                knee: 0.0,
                live: false,
                modules: vec![],
                chips: vec![],
            }),
            origin: Point::default(),
            view_w: 0.0,
            view_h: 0.0,
            inside: false,
            hover: None,
            select_glows: motion::Glows::new(Pace::Glow),
            hover_glows: motion::Glows::new(Pace::Glow),
            clock: Instant::now(),
            seen_selection: Selection::None,
            seen_session: None,
            auto_fit: None,
            pinned: false,
            ceremonies,
            frozen: None,
            scene_view: None,
            node_states: HashMap::new(),
            blends: HashMap::new(),
            arrivals: HashMap::new(),
            blend_session: None,
        };
        let scene = this.state.read(cx).scene.clone();
        if let Some(scene) = scene {
            this.set_scene(&scene, window, cx);
        }
        this
    }

    /// Demo only: puts the graph into a named state so it can be captured.
    fn set_scene(&mut self, scene: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (name, window_size) = scene.split_once('@').unwrap_or((scene, ""));
        if let Some((w, h)) = window_size.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
            window.resize(size(px(w), px(h)));
        }
        self.pinned = true;
        if self.state.read(cx).runs.len() < scene_runs_needed(name) {
            return;
        }
        self.state.update(cx, |s, cx| {
            let first_with = |run: &Run, state| run.units.iter().position(|u| u.state == state);
            match name {
                "mesh-large" => s.runs[0] = large_run(s.runs[0].clone(), 14, 320),
                "mesh-huge" | "mesh-huge-fit" => s.runs[0] = large_run(s.runs[0].clone(), 40, 900),
                "mesh-planning" => {
                    let run = &mut s.runs[0];
                    run.modules.clear();
                    run.units.clear();
                    run.goals.clear();
                    for (i, stage) in run.stages.iter_mut().enumerate() {
                        stage.phase = if i == 0 { Phase::Running } else { Phase::Waiting };
                        stage.note = None;
                    }
                }
                "mesh-waiting" => s.current = Some(1),
                "mesh-blocked" => s.current = Some(2),
                "mesh-done" => s.current = Some(3),
                "mesh-partial" => {
                    s.current = Some(3);
                    let run = &mut s.runs[3];
                    run.units[1].state = UnitState::Failed;
                    run.units[3].state = UnitState::Built;
                    run.stages[4].phase = Phase::Failed;
                }
                "mesh-select-unit" => {
                    let unit = first_with(&s.runs[0], UnitState::Failed).unwrap_or(0);
                    s.selection = Selection::Unit(unit);
                }
                "mesh-select-module" => s.selection = Selection::Module(7),
                "mesh-reduced" => s.reduce_motion = true,
                "mesh-orbit-fade" => self.frozen = Some((Moment::Started(s.runs[0].session.clone()), 0.45)),
                "mesh-hourglass-turn" => self.frozen = Some((Moment::Ended(s.runs[0].session.clone()), 0.4)),
                _ => {}
            }
            cx.notify();
        });
        let run = &self.state.read(cx).runs[0];
        match name {
            "mesh-hover-unit" => self.hover = run.units.iter().position(|u| u.state == UnitState::Running).map(Node::Unit),
            "mesh-hover-module" => self.hover = Some(Node::Module(7)),
            "mesh-hover-core" => self.hover = Some(Node::Core),
            // Zoomed in on the failed unit, which the arrow keys have reached.
            "mesh-zoom-keys" => {
                let unit = run.units.iter().position(|u| u.state == UnitState::Failed).unwrap_or(0);
                self.scene_view = Some(SceneView::ZoomAt(1.8, Node::Unit(unit)));
            }
            "mesh-fit" | "mesh-huge-fit" => self.scene_view = Some(SceneView::Fit),
            _ => {}
        }
    }

    /// Demo only: applies a scene's view once the first layout exists.
    fn apply_scene_view(&mut self) {
        let Some(scene) = self.scene_view.take() else {
            return;
        };
        match scene {
            SceneView::ZoomAt(factor, node) => {
                if let Some(rect) = self.graph.rect_of(node) {
                    self.auto_fit = None;
                    let at = self.view.rect(rect);
                    self.view = self.view.zoomed_about(Pt { x: at.x + at.w / 2.0, y: at.cy() }, factor);
                    self.view = reveal_view(self.view, rect, self.view_w, self.view_h);
                    self.cursor = Some(node);
                }
            }
            SceneView::Fit => self.fit(),
        }
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.auto_fit = None;
        let view = clamp_view(view, self.graph.content(), self.view_w, self.view_h);
        if view != self.view {
            self.view = view;
            cx.notify();
        }
    }

    fn fit(&mut self) {
        self.auto_fit = None;
        if let Some(content) = self.graph.content() {
            self.view = fit_view(content, self.view_w, self.view_h);
        }
    }

    /// Zooms about the middle of the panel, for the buttons and keys.
    fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let middle = Pt { x: self.view_w / 2.0, y: self.view_h / 2.0 };
        self.set_view(self.view.zoomed_about(middle, factor), cx);
    }

    fn scrolled(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(WHEEL_LINE));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let view = if event.modifiers.control || event.modifiers.platform {
            self.view.zoomed_about(self.panel_point(event.position), wheel_zoom(dy))
        } else {
            self.view.panned(dx, dy)
        };
        self.set_view(view, cx);
        cx.stop_propagation();
    }

    fn pressed(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.dragged = false;
        let empty = self.hit(self.panel_point(event.position)).is_none();
        self.drag = empty.then_some(Drag { start: event.position, pan: self.view.pan, moved: false });
    }

    fn open_menu(&mut self, node: Node, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.state.read(cx).run().and_then(|run| menu_target(run, node)) else {
            if self.menu.take().is_some() { cx.notify(); }
            return;
        };
        let weak = cx.weak_entity();
        let runner: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)> = Rc::new(move |action, target, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                let resolved = this.state.read(cx).run().and_then(|run| match &target {
                    MenuTarget::Unit { session, unit } if &run.session == session => run.units.iter().position(|u| &u.id == unit)
                        .map(|i| (Node::Unit(i), Selection::Unit(i), format!("{} — {}", run.units[i].id, run.units[i].title))),
                    MenuTarget::Module { session, module } if &run.session == session => run.modules.iter().position(|m| &m.id == module)
                        .map(|i| (Node::Module(i), Selection::Module(i), format!("{} — {}", run.modules[i].id, run.modules[i].purpose))),
                    _ => None,
                });
                if let Some((node, selection, text)) = resolved {
                    match action {
                        MenuAction::CopyText => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                        MenuAction::OpenDetail | MenuAction::CommentOn => {
                            this.cursor = Some(node);
                            this.state.update(cx, |state, cx| state.select(selection, cx));
                            cx.notify();
                        }
                        _ => {}
                    }
                }
            });
        });
        let state = self.state.clone();
        let menu = PopupMenu::build(window, cx, move |menu, _, cx| palette::menu::context_menu(menu, target, state, runner, cx));
        let subscription = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            window.focus(&this.focus, cx);
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.menu = Some((menu, position, subscription));
        cx.notify();
    }

    fn right_pressed(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let node = self.hit(self.panel_point(event.position));
        if let Some(node @ (Node::Unit(_) | Node::Module(_))) = node {
            self.cursor = Some(node);
            self.open_menu(node, event.position, window, cx);
        } else if self.menu.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn open_cursor_menu(&mut self, _: &palette::ContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) { return; }
        let Some(node) = menu_anchor_node(self.cursor, selected_node(&self.state.read(cx).selection)) else { return };
        let Some(target) = self.state.read(cx).run().and_then(|run| menu_target(run, node)) else { return };
        let Some(rect) = self.graph.rect_of(node) else { return };
        cx.stop_propagation();
        let screen = self.view.rect(rect);
        if screen.x < 0.0 || screen.y < 0.0 || screen.right() > self.view_w || screen.bottom() > self.view_h {
            self.set_view(reveal_view(self.view, rect, self.view_w, self.view_h), cx);
            cx.on_next_frame(window, move |this, window, cx| {
                if this.state.read(cx).run().and_then(|run| menu_target(run, node)).as_ref() == Some(&target) {
                    this.open_menu_at_node(node, window, cx);
                }
            });
        } else {
            self.open_menu_at_node(node, window, cx);
        }
    }

    fn open_menu_at_node(&mut self, node: Node, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rect) = self.graph.rect_of(node).map(|rect| self.view.rect(rect)) else { return };
        let (x, y) = node_center((f32::from(self.origin.x), f32::from(self.origin.y)), (rect.x, rect.y, rect.w, rect.h));
        self.open_menu(node, point(px(x), px(y)), window, cx);
    }

    fn moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let (Some(drag), Some(MouseButton::Left)) = (self.drag.as_mut(), event.pressed_button) {
            let (dx, dy) = (f32::from(event.position.x - drag.start.x), f32::from(event.position.y - drag.start.y));
            if !drag.moved && dx.hypot(dy) < DRAG_SLOP {
                return;
            }
            drag.moved = true;
            self.dragged = true;
            let view = View { zoom: self.view.zoom, pan: Pt { x: drag.pan.x + dx, y: drag.pan.y + dy } };
            self.set_view(view, cx);
            return;
        }
        if self.pinned {
            return;
        }
        let node = self.hit(self.panel_point(event.position));
        self.set_hover(node, cx);
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let m = &event.keystroke.modifiers;
        let command = m.control || m.platform;
        if m.alt || (m.shift && !command) {
            return;
        }
        let step = match event.keystroke.key.as_str() {
            "up" if !command => Some(Step::Up),
            "down" if !command => Some(Step::Down),
            "left" if !command => Some(Step::Left),
            "right" if !command => Some(Step::Right),
            _ => None,
        };
        if let Some(step) = step {
            self.cursor = self.graph.step(self.cursor.or_else(|| selected_node(&self.state.read(cx).selection)), step);
            if let Some(rect) = self.cursor.and_then(|n| self.graph.rect_of(n)) {
                let view = reveal_view(self.view, rect, self.view_w, self.view_h);
                self.set_view(view, cx);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        match (event.keystroke.key.as_str(), command) {
            ("enter", false) => {
                let Some(node) = self.cursor else { return };
                self.auto_fit = None;
                self.state.update(cx, |s, cx| s.select(selection_of(Some(node)), cx));
            }
            ("escape", false) => {
                self.cursor = None;
                self.auto_fit = None;
                self.state.update(cx, |s, cx| s.select(Selection::None, cx));
            }
            ("f", false) => {
                self.fit();
                cx.notify();
            }
            ("=" | "+", true) => self.zoom_by(ZOOM_STEP, cx),
            ("-", true) => self.zoom_by(1.0 / ZOOM_STEP, cx),
            ("0", true) => self.zoom_by(1.0 / self.view.zoom, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn panel_point(&self, position: Point<Pixels>) -> Pt {
        Pt {
            x: f32::from(position.x - self.origin.x),
            y: f32::from(position.y - self.origin.y),
        }
    }

    fn hit(&self, point: Pt) -> Option<Node> {
        (point.y < self.view_h - CONTROLS_BAND).then(|| self.graph.hit(point, self.view_h, &self.view)).flatten()
    }

    fn set_hover(&mut self, node: Option<Node>, cx: &mut Context<Self>) {
        if node != self.hover {
            self.hover = node;
            cx.notify();
        }
    }

    fn clicked(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.dragged) {
            return;
        }
        let node = self.hit(self.panel_point(position));
        self.auto_fit = None;
        self.cursor = node.filter(|&n| n != Node::Core);
        self.state.update(cx, |s, cx| s.select(selection_of(node), cx));
    }

    /// A gold ring just outside a node, marking the keyboard cursor.
    fn cursor_ring(&self, r: Rect, t: &Tokens) -> AnyElement {
        div()
            .absolute()
            .left(px(r.x - 4.0))
            .top(px(r.y - 4.0))
            .w(px(r.w + 8.0))
            .h(px(r.h + 8.0))
            .rounded(theme::RADIUS)
            .border(px(1.5))
            .border_color(t.gold.opacity(0.9))
            .into_any_element()
    }

    fn card_view(&self, m: &ModuleBox, r: Rect, run: &Run, t: &Tokens, dark: bool, breath: f32, levels: &HashMap<Node, (f32, f32)>, cx: &App) -> AnyElement {
        let (select, hover) = levels.get(&Node::Module(m.index)).copied().unwrap_or_default();
        let blend = self.blend_of(Node::Module(m.index), cx);
        let look = node_look(t, m.state, t.surface, dark);
        let look = blend.map_or(look, |(from, k)| look_between(&node_look(t, from, t.surface, dark), &look, k));
        let border = mix(mix(look.border, t.text.opacity(0.7), hover), t.gold, select);
        let mut shadow = look.glow.map_or_else(Vec::new, |color| glow(color, 12.));
        shadow.extend(theme::selection_glow(t, select));
        let body_breath = if m.running { breath } else { 0.0 };
        let side = glyph_side(r, self.view.zoom);
        let body: AnyElement = match blend {
            None => ornament::body(m.state, side, t.unit(m.state), t.orbit, look.fill, body_breath).into_any_element(),
            Some((from, k)) => div()
                .relative()
                .flex_none()
                .size(px(side))
                .child(div().absolute().top_0().left_0().child(ornament::body(from, side, t.unit(from).opacity(1.0 - k), t.orbit, look.fill, 0.0)))
                .child(div().absolute().top_0().left_0().child(ornament::body(
                    m.state,
                    side,
                    t.unit(m.state).opacity(k),
                    t.orbit.opacity(k),
                    look.fill.opacity(k),
                    body_breath,
                )))
                .into_any_element(),
        };
        let share = |n: usize| if m.total == 0 { 0.0 } else { n as f32 / m.total as f32 };
        let zoom = self.view.zoom;
        let worded = r.h >= 12.0;
        let text_size = px((13.0 * zoom).clamp(12.0, 20.0));
        let count_size = px((12.0 * zoom).clamp(12.0, 18.0));
        let run_ref = &run.modules[m.index];
        let count = |state| run_ref.units.iter().filter(|&&u| run.units[u].state == state).count();
        let segments = [
            (m.verified, t.ok),
            (count(UnitState::Built), t.review),
            (count(UnitState::Running), t.run),
            (count(UnitState::Failed), t.fail),
        ];
        v_flex()
            .id(("mesh-module-node", m.index))
            .opacity(self.presence_of(Node::Module(m.index), cx))
            .absolute()
            .left(px(r.x))
            .top(px(r.y))
            .w(px(r.w))
            .h(px(r.h))
            .justify_center()
            .gap(px((3.0 * zoom).min(5.0)))
            .pl(px((4.0 * zoom).min(8.0)))
            .pr(px((8.0 * zoom).min(14.0)))
            .overflow_hidden()
            .rounded(theme::RADIUS)
            .border_1()
            .border_color(border)
            .bg(look.fill)
            .shadow(shadow)
            .child(
                h_flex()
                    .gap(px((5.0 * zoom).min(8.0)))
                    .child(body)
                    .when(worded, |this| {
                        this.child(div().id(("mesh-module-label", m.index)).flex_1().min_w_0().truncate().text_size(text_size).line_height(text_size * 1.15).text_color(if m.state == UnitState::Verified { t.muted } else { t.text }).child(run_ref.id.clone().test_text(run_ref.id.clone())).test_support())
                    })
                    .when(worded && r.w >= 120.0, |this| {
                        this.child(div().flex_none().text_size(count_size).text_color(t.muted).child(format!("{}/{}", m.verified, m.total)))
                    }),
            )
            .child(
                h_flex()
                    .h(px((2.0 * zoom).clamp(1.0, 4.0)))
                    .w_full()
                    .overflow_hidden()
                    .bg(t.orbit.opacity(0.6))
                    .children(segments.into_iter().filter(|(n, _)| *n > 0).map(|(n, color)| {
                        let quiet = m.state == UnitState::Verified;
                        div().h_full().w(relative(share(n))).bg(if quiet { color.opacity(0.55) } else { color })
                    })),
            )
            .test_support().into_any_element()
    }

    fn chip_view(&self, index: usize, chip: &ChipBox, r: Rect, run: &Run, t: &Tokens, dark: bool, breath: f32, levels: &HashMap<Node, (f32, f32)>, cx: &App) -> AnyElement {
        let (select, hover) = levels.get(&chip.node).copied().unwrap_or_default();
        let blend = self.blend_of(chip.node, cx);
        let look = node_look(t, chip.state, t.surface, dark);
        let look = blend.map_or(look, |(from, k)| look_between(&node_look(t, from, t.surface, dark), &look, k));
        let border = mix(mix(look.border, t.text.opacity(0.7), hover), t.gold, select);
        let mut shadow = look.glow.map_or_else(Vec::new, |color| glow(color, 9.));
        shadow.extend(theme::selection_glow(t, select));
        let zoom = self.view.zoom;
        if r.h < LABEL_MIN_CHIP_H {
            // Too small for words: a tinted body in the state's color, its shape kept when it fits.
            let color = blend.map_or(t.unit(chip.state), |(from, k)| mix(t.unit(from), t.unit(chip.state), k));
            let radius = px((3.0 * zoom).min(3.0));
            return div()
                .id(("mesh-chip-node", index))
                .opacity(self.presence_of(chip.node, cx))
                .absolute()
                .left(px(r.x))
                .top(px(r.y))
                .w(px(r.w))
                .h(px(r.h))
                .flex()
                .items_center()
                .px(px((6.0 * zoom).min(4.0)))
                .rounded(radius)
                .border_1()
                .border_color(border)
                .shadow(shadow)
                .bg(look.fill.blend(color.opacity(0.18)))
                .when(r.h >= 10.0, |this| this.child(mark_between(blend, chip.state, r.h - 4.0, t, breath)))
                .test_support().into_any_element();
        }
        let text_size = px((12.0 * zoom).clamp(12.0, 18.0));
        let label: SharedString = match chip.node {
            Node::Unit(u) => {
                let unit = &run.units[u];
                unit_chip_label(unit, &plain_text::plain(cx, &unit.title))
            }
            Node::Folded(_, state) => {
                let word = if state == UnitState::Verified { "verified" } else { "pending" };
                format!("+{} {}", chip.count, tr(cx, TABLE, word)).into()
            }
            _ => SharedString::default(),
        };
        let retries: Option<SharedString> = match chip.node {
            Node::Unit(u) if r.w >= CHIP_MIN_W && run.units[u].attempts > 1 => Some(format!("×{}", run.units[u].attempts).into()),
            _ => None,
        };
        h_flex()
            .id(("mesh-chip-node", index))
            .opacity(self.presence_of(chip.node, cx))
            .absolute()
            .left(px(r.x))
            .top(px(r.y))
            .w(px(r.w))
            .h(px(r.h))
            .gap(px((4.0 * zoom).min(7.0)))
            .px(px((6.0 * zoom).min(10.0)))
            .overflow_hidden()
            .rounded(theme::RADIUS_SMALL)
            .border_1()
            .border_color(border)
            .bg(look.fill)
            .shadow(shadow)
            .child(mark_between(blend, chip.state, (13.0 * zoom).clamp(10.0, 22.0), t, breath))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(text_size)
                    .text_color(look.text)
                    .child(label),
            )
            .when_some(retries, |this, text| this.child(div().flex_none().text_size(text_size).text_color(t.muted).child(text)))
            .test_support().into_any_element()
    }

    fn tooltip_body(&self, node: Node, run: &Run, t: &Tokens, cx: &App) -> Option<AnyElement> {
        let d = theme::density(cx);
        let heading = |text: SharedString, mono: bool| {
            div()
                .text_size(theme::font(d, TypeSize::Body))
                .text_color(t.text)
                .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
                .child(text)
        };
        let note = |text: SharedString| div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted).child(text);
        let state_line = |state: UnitState, extra: Option<SharedString>| {
            h_flex()
                .gap(theme::space(d, SpaceStep::Sm))
                .text_size(theme::font(d, TypeSize::Caption))
                .child(state_mark(state, 12., t.unit(state), 0.))
                .child(div().text_color(t.unit(state)).child(tr(cx, TABLE, state.label())))
                .when_some(extra, |this, text| this.child(div().text_color(t.muted).child(text)))
        };
        let body = v_flex().gap(theme::space(d, SpaceStep::Xs));
        let body = match node {
            Node::Core => {
                let stage = current_stage(run).map(|s| fmt_a(tr(cx, TABLE, "Stage: {a}"), stage_label(cx, &s.name)));
                let role = current_stage(run).and_then(|s| stage_role(&s.name)).map(|id| i18n::role_tooltip(*cx.global::<Lang>(), id));
                body.child(heading("Chronos".into(), false))
                    .child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.status(run.status)).child(tr(cx, TABLE, run.status.label())))
                    .when_some(stage, |this, line| this.child(note(line.into())))
                    .when_some(role, |this, line| this.child(note(line)))
                    .child(note(tr(cx, TABLE, "Click to return to the overview").into()))
            }
            Node::Module(m) => {
                let module = run.modules.get(m)?;
                let state = module_state(run, module);
                let verified = fmt_ab(tr(cx, TABLE, "{a} of {b} verified"), run.module_done(module), module.units.len());
                let depends: Vec<&str> = module.depends.iter().filter_map(|&d| run.modules.get(d)).map(|d| d.id.as_ref()).collect();
                let depends = (!depends.is_empty()).then(|| fmt_a(tr(cx, TABLE, "Depends on {a}"), depends.join(", ")));
                body.child(heading(module.id.clone(), false))
                    .child(note(module.purpose.clone()))
                    .child(state_line(state, Some(verified.into())))
                    .when_some(depends, |this, line| this.child(note(line.into())))
            }
            Node::Unit(u) => {
                let unit = run.units.get(u)?;
                let lines = unit_tooltip_lines(unit, plain_text::plain(cx, &unit.title));
                let attempts = (unit.attempts > 0).then(|| fmt_a(tr(cx, TABLE, "Attempt {a}"), unit.attempts).into());
                let module = run.modules.get(unit.module).map(|m| fmt_a(tr(cx, TABLE, "Module {a}"), m.id.as_ref()));
                body.child(heading(unit.id.clone(), true))
                    .children(lines.into_iter().map(note))
                    .child(state_line(unit.state, attempts))
                    .when_some(module, |this, line| this.child(note(line.into())))
            }
            Node::Folded(m, state) => {
                let module = run.modules.get(m)?;
                let n = module.units.iter().filter(|&&u| run.units[u].state == state).count();
                let key = if state == UnitState::Verified { "{a} verified units of {b}" } else { "{a} pending units of {b}" };
                let names: Vec<&str> = module
                    .units
                    .iter()
                    .filter(|&&u| run.units[u].state == state)
                    .take(6)
                    .map(|&u| run.units[u].id.as_ref())
                    .collect();
                let more = (n > names.len()).then(|| fmt_a(tr(cx, TABLE, "and {a} more"), n - names.len()));
                body.child(state_line(state, None))
                    .child(heading(fmt_ab(tr(cx, TABLE, key), n, module.id.as_ref()).into(), false))
                    .child(note(names.join(", ").into()))
                    .when_some(more, |this, line| this.child(note(line.into())))
                    .child(note(tr(cx, TABLE, "Click to list them in the graph").into()))
            }
        };
        Some(body.into_any_element())
    }

    fn tooltip_view(&self, node: Node, run: &Run, t: &Tokens, cx: &App) -> Option<AnyElement> {
        let d = theme::density(cx);
        let body = self.tooltip_body(node, run, t, cx)?;
        let core = self.graph.core(self.view_h, &self.view);
        let at = match node {
            Node::Core => Pt { x: core.x + self.graph.core_radius(&self.view) + 10.0, y: core.y - 24.0 },
            Node::Module(_) => {
                let r = self.view.rect(self.graph.rect_of(node)?);
                Pt { x: r.x, y: r.bottom() + 6.0 }
            }
            _ => {
                let r = self.view.rect(self.graph.rect_of(node)?);
                Pt { x: r.right() + 8.0, y: r.y - 2.0 }
            }
        };
        let node_rect = self.view.rect(self.graph.rect_of(node).unwrap_or(Rect { x: core.x, y: core.y, w: 0.0, h: 0.0 }));
        let place = place_tip(at, node_rect, self.view_w, self.view_h);
        Some(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(place.at.x), px(place.at.y)))
                    .anchor(if place.from_bottom { Anchor::BottomLeft } else { Anchor::TopLeft })
                    .snap_to_window_with_margin(px(TIP_EDGE))
                    .child(
                        div()
                            .w(px(TIP_W))
                            .max_h(px(TIP_MAX_H))
                            .overflow_hidden()
                            .p(theme::space(d, SpaceStep::Md))
                            .rounded(theme::RADIUS)
                            .border_1()
                            .border_color(t.border)
                            .bg(t.raised)
                            .shadow_lg()
                            .child(body),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Builds the panel once its size is known, so the layout and the drawing use the same frame.
    fn graph_view(&mut self, panel: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let dark = cx.theme().is_dark();
        let d = theme::density(cx);
        let (width, view_h) = (f32::from(panel.width), f32::from(panel.height));
        if width <= 0.0 || view_h <= 0.0 {
            return div().into_any_element();
        }
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            return div().into_any_element();
        };
        let selection = state.selection.clone();
        let moving = animates(run, state.reduce_motion);
        // A narrow overflowing panel gets wider cards and two chip columns before fitting.
        let graph = Rc::new(panel_layout(run, width, view_h, &selection));
        self.view_w = width;
        self.view_h = view_h;
        self.graph = graph.clone();

        if self.seen_session.as_ref() != Some(&run.session) {
            self.seen_session = Some(run.session.clone());
            self.seen_selection = selection.clone();
            self.cursor = None;
            self.view = initial_view(&graph, width, view_h);
            self.auto_fit = Some((width, view_h));
            self.apply_scene_view();
        } else if selection != self.seen_selection {
            self.seen_selection = selection.clone();
            self.auto_fit = None;
            if let Some(rect) = selected_node(&selection).and_then(|n| graph.rect_of(n)) {
                self.view = reveal_view(self.view, rect, width, view_h);
            }
        } else if should_auto_refit(self.auto_fit, (width, view_h)) {
            self.view = initial_view(&graph, width, view_h);
            self.auto_fit = Some((width, view_h));
        }
        self.view = clamp_view(self.view, graph.content(), width, view_h);
        self.track_blends(&graph, &run.session, state.reduce_motion, motion::now(cx));
        let mut frames = Driver::new(state.reduce_motion, cx);
        frames.keep(!self.blends.is_empty() || !self.arrivals.is_empty());
        let view = self.view;
        if self.cursor.is_some_and(|n| n != Node::Core && graph.rect_of(n).is_none()) {
            self.cursor = None;
        }

        if self.inside && self.drag.is_none() {
            let p = self.panel_point(window.mouse_position());
            self.hover = self.hit(p);
        }
        let hover = self.hover.filter(|&node| node == Node::Core || graph.rect_of(node).is_some());
        // The cursor shows while the graph is being driven by keys, or in a captured scene.
        let ring = self
            .cursor
            .filter(|_| self.pinned || (self.focus.is_focused(window) && window.last_input_was_keyboard()));

        let now = Instant::now();
        let reduced = state.reduce_motion;
        let (mut orbit_p, mut turn_p) = (self.ceremonies.orbit(&run.session, reduced, now), self.ceremonies.turn(&run.session, reduced, now));
        if let Some((moment, p)) = &self.frozen {
            match moment {
                Moment::Started(owner) if *owner == run.session => orbit_p = Some(*p),
                Moment::Ended(owner) if *owner == run.session => turn_p = Some(*p),
                _ => {}
            }
        }
        let time = moving.then(|| self.clock.elapsed().as_secs_f32());
        let breath = time.map_or(0.0, ornament::breath);
        let reach = Rect { x: -OVERSCAN, y: -OVERSCAN, w: width + OVERSCAN * 2.0, h: view_h + OVERSCAN * 2.0 };
        let nodes: std::collections::HashSet<Node> = graph.modules.iter().map(|m| Node::Module(m.index))
            .chain(graph.chips.iter().map(|c| c.node)).collect();
        self.select_glows.retain(|node| nodes.contains(node));
        self.hover_glows.retain(|node| nodes.contains(node));
        let mut levels = HashMap::new();
        let visible = graph.modules.iter().map(|m| (Node::Module(m.index), m.rect))
            .chain(graph.chips.iter().map(|c| (c.node, c.rect)))
            .filter(|(_, rect)| view.rect(*rect).overlaps(&reach));
        for (node, _) in visible {
            let (select, over) = glow_targets(selected_node(&selection), hover, node);
            self.select_glows.set(&node, select, frames.now(), reduced);
            self.hover_glows.set(&node, over, frames.now(), reduced);
            levels.insert(node, (self.select_glows.value(&node, &mut frames), self.hover_glows.value(&node, &mut frames)));
        }
        frames.finish(window);
        let cards: Vec<AnyElement> = graph
            .modules
            .iter()
            .map(|m| (m, view.rect(m.rect)))
            .filter(|(_, r)| r.overlaps(&reach))
            .map(|(m, r)| self.card_view(m, r, run, &t, dark, breath, &levels, cx))
            .collect();
        let chips: Vec<AnyElement> = graph
            .chips
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c, view.rect(c.rect)))
            .filter(|(_, _, r)| r.overlaps(&reach))
            .map(|(i, c, r)| self.chip_view(i, c, r, run, &t, dark, breath, &levels, cx))
            .collect();
        let ring = ring.and_then(|n| graph.rect_of(n)).map(|r| self.cursor_ring(view.rect(r), &t));

        let backdrop = Backdrop {
            graph: graph.clone(),
            view,
            t,
            dark,
            time,
            selected: selected_node(&selection),
            hovered: hover,
            orbit_in: orbit_visibility(orbit_p),
            turn: hourglass_turn(turn_p),
        };
        let this = cx.entity();
        // Keyboard users see what the cursor is on, as a pointer would by hovering.
        let tip_node = hover.or(self.cursor.filter(|_| ring.is_some()));
        let tooltip = tip_node.and_then(|node| self.tooltip_view(node, run, &t, cx));
        let status = run.status;
        let core = graph.core(view_h, &view);
        let core_r = graph.core_radius(&view);
        let label_w = (core_r * 2.0 + 40.0).max(92.0);
        let shown = graph.content().map(|c| view.rect(c));
        let above = shown.is_some_and(|c| c.y < -1.0);
        let below = shown.is_some_and(|c| c.bottom() > view_h + 1.0);
        let zoom_label: SharedString = format!("{:.0}%", view.zoom * 100.0).into();
        let stage = current_stage(run).map(|s| stage_label(cx, &s.name));
        let sub: SharedString = match (status, stage) {
            (RunStatus::Active, Some(stage)) => stage,
            _ => tr(cx, TABLE, status.label()).into(),
        };
        // The core's name needs room: a narrow panel or a far-out zoom would lay it over the cards.
        let wide = width >= 420.0 && view.zoom >= 0.6;

        let dragging = self.drag.is_some_and(|d| d.moved);
        div()
            .id("mesh-graph")
            .test_support()
            .track_focus(&self.focus.clone().tab_stop(true))
            .size_full()
            .relative()
            .overflow_hidden()
            .when(hover.is_some() && !dragging, |this| this.cursor_pointer())
            .when(dragging, |this| this.cursor_grabbing())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .on_action(cx.listener(Self::open_cursor_menu))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| this.scrolled(event, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| this.pressed(event, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_pressed))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| {
                if this.drag.take().is_some_and(|d| d.moved) {
                    cx.notify();
                }
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| {
                if this.drag.take().is_some_and(|d| d.moved) {
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.moved(event, cx)))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                let view = this.view.zoomed_about(this.panel_point(event.position), 1.0 + event.delta);
                this.set_view(view, cx);
            }))
            .on_hover(cx.listener(|this, inside: &bool, _, cx| {
                this.inside = *inside && !this.pinned;
                if !*inside && !this.pinned {
                    this.set_hover(None, cx);
                }
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| this.clicked(event.position(), cx)))
            .child(
                div().absolute().top_0().left_0().right_0().bottom(px(CONTROLS_BAND)).overflow_hidden()
                    .child(canvas(
                        move |bounds, _, cx| this.update(cx, |this, _| this.origin = bounds.origin),
                        move |bounds, _, window, _| backdrop.paint(bounds, window),
                    ).absolute().w_full().h(px(view_h)))
                    .child(div().absolute().w_full().h(px(view_h)).children(cards).children(chips).children(ring))
                    .when(above, |this| this.child(edge_fade(t, true)))
                    .when(below, |this| this.child(edge_fade(t, false)))
                    .when(wide, |this| this.child(
                        v_flex().absolute().left(px(core.x - label_w / 2.0)).top(px(core.y + core_r + 22.0))
                            .w(px(label_w)).items_center()
                            .child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.gold).child("C H R O N O S"))
                            .child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.status(status)).child(sub)),
                    )),
            )
            .child(self.view_controls(zoom_label, &t, cx))
            .when_some(tooltip, |this, tip| this.child(tip))
            .when_some(self.menu.as_ref(), |this, (menu, position, _)| {
                this.child(deferred(anchored().position(*position).snap_to_window_with_margin(px(8.)).child(menu.clone())).with_priority(2))
            })
            .into_any_element()
    }

    /// Zoom out, the zoom (which returns to true size), zoom in, and fit, at the panel's corner.
    fn view_controls(&self, zoom_label: SharedString, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let d = theme::density(cx);
        let button = |id: &'static str| Button::new(id).ghost().small().h(theme::TARGET_MIN).min_w(theme::TARGET_MIN);
        h_flex()
            .id("mesh-view-controls")
            .absolute()
            .right(theme::space(d, SpaceStep::Md))
            .bottom(theme::space(d, SpaceStep::Md))
            .gap(theme::space(d, SpaceStep::Xs))
            .p(theme::space(d, SpaceStep::Xs))
            .rounded(theme::RADIUS)
            .border_1()
            .border_color(t.border)
            .bg(t.surface.opacity(0.92))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                button("mesh-zoom-out")
                    .icon(IconName::Minus)
                    .tooltip(tr(cx, TABLE, "Zoom out (Ctrl and the wheel)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.zoom_by(1.0 / ZOOM_STEP, cx);
                    })),
            )
            .child(
                button("mesh-zoom-reset")
                    .label(zoom_label)
                    .tooltip(tr(cx, TABLE, "Back to true size"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.zoom_by(1.0 / this.view.zoom, cx);
                    })),
            )
            .child(
                button("mesh-zoom-in")
                    .icon(IconName::Plus)
                    .tooltip(tr(cx, TABLE, "Zoom in (Ctrl and the wheel)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.zoom_by(ZOOM_STEP, cx);
                    })),
            )
            .child(
                button("mesh-fit")
                    .icon(IconName::Maximize)
                    .label(tr(cx, TABLE, "Fit"))
                    .tooltip(tr(cx, TABLE, "Frame everything (F)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.fit();
                        cx.notify();
                    })),
            )
            .test_support()
    }
}

/// A soft edge that says the graph continues past the top or bottom of the panel.
fn edge_fade(t: Tokens, top: bool) -> impl IntoElement {
    let angle = if top { 180.0 } else { 0.0 };
    div()
        .absolute()
        .left_0()
        .right_0()
        .when(top, |this| this.top_0())
        .when(!top, |this| this.bottom_0())
        .h(px(FADE_H))
        .bg(linear_gradient(angle, linear_color_stop(t.canvas, 0.0), linear_color_stop(t.canvas.opacity(0.0), 1.0)))
}

fn legend(run: &Run, t: &Tokens, cx: &App) -> impl IntoElement {
    let d = theme::density(cx);
    let order = [UnitState::Running, UnitState::Built, UnitState::Failed, UnitState::Verified, UnitState::Pending];
    h_flex()
        .flex_wrap()
        .flex_none()
        .gap(theme::space(d, SpaceStep::Md))
        .px(theme::space(d, SpaceStep::Md))
        .py(theme::space(d, SpaceStep::Sm))
        .border_b_1()
        .border_color(t.border)
        .children(order.into_iter().enumerate().map(|(i, state)| {
            let tip = match state {
                UnitState::Running => "Running: a unit is being worked on now",
                UnitState::Built => "Built: a unit is ready for verification",
                UnitState::Failed => "Failed: a unit needs attention",
                UnitState::Verified => "Verified: a unit has passed verification",
                UnitState::Pending => "Pending: a unit is waiting to start",
            };
            let tip: SharedString = tr(cx, TABLE, tip).into();
            let n = run.count(state);
            h_flex()
                .id(("mesh-legend", i))
                .tooltip(move |window, cx| Tooltip::element({ let tip = tip.clone(); move |_, _| div().child(tip.clone()) }).build(window, cx))
                .gap(theme::space(d, SpaceStep::Sm))
                .text_size(theme::font(d, TypeSize::Caption))
                .text_color(if n > 0 { t.text } else { t.muted })
                .child(state_mark(state, 12., t.unit(state), 0.))
                .child(tr(cx, TABLE, state.label()))
                .child(div().text_color(t.muted).child(n.to_string()))
        }))
}

fn summary_fact(d: theme::Density, t: &Tokens, label: &'static str, value: SharedString, color: Hsla) -> impl IntoElement {
    v_flex()
        .child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted).child(label))
        .child(div().text_size(theme::font(d, TypeSize::Body)).text_color(color).child(value))
}

/// What a finished run produced. Units that are not verified are named, never counted as done.
fn summary(run: &Run, t: &Tokens, cx: &App) -> impl IntoElement {
    let d = theme::density(cx);
    let total = run.units.len();
    let verified = run.count(UnitState::Verified);
    let headline = if total > 0 && verified == total {
        fmt_a(tr(cx, TABLE, "All {a} units verified"), total)
    } else {
        fmt_ab(tr(cx, TABLE, "{a} of {b} units verified"), verified, total)
    };
    let complete = total > 0 && verified == total;
    let (test, test_color) = match run.stages.iter().find(|s| s.name.as_ref() == "Full test").map(|s| s.phase) {
        Some(Phase::Done) => (tr(cx, TABLE, "Passed"), t.ok),
        Some(Phase::Failed) => (tr(cx, TABLE, "Failed"), t.fail),
        Some(Phase::Running) => (tr(cx, TABLE, "Running"), t.run),
        _ => (tr(cx, TABLE, "Not run"), t.muted),
    };
    let cost: SharedString = run.usage.total_usd.map_or("—".into(), |usd| format!("${usd:.2}").into());
    let time: SharedString = run.elapsed.clone().unwrap_or_else(|| "—".into());
    let open: Vec<&Unit> = run.units.iter().filter(|u| u.state != UnitState::Verified).collect();
    let shown = open.len().min(5);
    let report = match &run.report {
        Some(TextSource::Inline(text)) => Some(text.clone()),
        _ => None,
    };
    let report_path = match &run.report {
        Some(TextSource::Path(path)) => Some(path.clone()),
        _ => None,
    };
    ornament::frame(t, if complete { t.ok.opacity(0.55) } else { t.review.opacity(0.6) }, t.surface)
        .flex()
        .flex_col()
        .flex_none()
        .m(theme::space(d, SpaceStep::Md))
        .px(theme::space(d, SpaceStep::Lg))
        .py(theme::space(d, SpaceStep::Md))
        .gap(theme::space(d, SpaceStep::Sm))
        .child(
            h_flex()
                .gap(theme::space(d, SpaceStep::Sm))
                .child(state_mark(
                    if complete { UnitState::Verified } else { UnitState::Built },
                    15.,
                    if complete { t.ok } else { t.review },
                    0.,
                ))
                .child(div().text_size(theme::font(d, TypeSize::Title)).text_color(t.text).child(headline)),
        )
        .child(ornament::divider(t, 0.0))
        .child(
            h_flex()
                .flex_wrap()
                .gap(theme::space(d, SpaceStep::Xl))
                .child(summary_fact(d, t, tr(cx, TABLE, "Full test"), test.into(), test_color))
                .child(summary_fact(d, t, tr(cx, TABLE, "Cost"), cost, t.text))
                .child(summary_fact(d, t, tr(cx, TABLE, "Time"), time, t.text)),
        )
        .when_some(report, |this, text| this.child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted).line_clamp(2).child(text)))
        .when_some(report_path, |this, path| {
            this.child(
                Button::new("mesh-open-report")
                    .ghost()
                    .xsmall()
                    .icon(IconName::FileText)
                    .label(tr(cx, TABLE, "Open report"))
                    .on_click(move |_, _, cx| cx.open_with_system(&path)),
            )
        })
        .when(!open.is_empty(), |this| {
            this.child(
                v_flex()
                    .gap(theme::space(d, SpaceStep::Xs))
                    .child(div().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted).child(tr(cx, TABLE, "Not verified")))
                    .children(open.iter().take(shown).map(|u| {
                        h_flex()
                            .gap(theme::space(d, SpaceStep::Sm))
                            .text_size(theme::font(d, TypeSize::Caption))
                            .child(state_mark(u.state, 12., t.unit(u.state), 0.))
                            .child(div().text_color(t.text).child(u.id.clone()))
                            .child(div().text_color(t.muted).child(tr(cx, TABLE, u.state.label())))
                    }))
                    .when(open.len() > shown, |this| {
                        this.child(
                            div()
                                .text_size(theme::font(d, TypeSize::Caption))
                                .text_color(t.muted)
                                .child(fmt_a(tr(cx, TABLE, "and {a} more"), open.len() - shown)),
                        )
                    }),
            )
        })
}

impl Render for Mesh {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let d = theme::density(cx);
        let state = self.state.read(cx);
        let run = state.run();
        let panel = panel_state(&state.feeds.runs, run.is_none());
        let view = mesh_view(&panel, run.is_some(), run.is_some_and(|r| !r.modules.is_empty()));
        let state_entity = self.state.clone();
        let retry = || {
            let state = state_entity.clone();
            Button::new("mesh-retry").label(tr(cx, TABLE, "Try again"))
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.retry(FeedKind::Runs, cx)))
        };
        match view {
            MeshView::Loading => return v_flex().size_full().items_center().justify_center()
                .text_size(theme::font(d, TypeSize::Body)).text_color(t.muted)
                .child(tr(cx, TABLE, "Loading the plan…")).into_any_element(),
            MeshView::Empty => return v_flex().size_full().items_center().justify_center().gap(theme::space(d, SpaceStep::Md))
                .child(ornament::empty_state(&t, tr(cx, TABLE, "No wish to show").into(), tr(cx, TABLE, "Make a wish and its work appears here as a live graph.").into()))
                .child(theme::gold_leaf(Button::new("mesh-make-wish").label(tr(cx, TABLE, "Make a wish"))
                    .on_click(move |_, _, cx| state_entity.update(cx, |s, cx| s.set_composing(true, cx))), &t))
                .into_any_element(),
            MeshView::Planning => return v_flex().size_full().items_center().justify_center()
                .child(ornament::empty_state(&t, tr(cx, TABLE, planning_line(run.unwrap())).into(), tr(cx, TABLE, "Modules and units appear here once the plan is ready.").into()))
                .into_any_element(),
            MeshView::Failed(message) => return v_flex().size_full().items_center().justify_center().gap(theme::space(d, SpaceStep::Md))
                .child(div().text_size(theme::font(d, TypeSize::Body)).text_color(t.text).child(message))
                .child(retry()).into_any_element(),
            MeshView::Graph { stale } => {
                let run = run.unwrap();
                let mut frames = Driver::new(state.reduce_motion, cx).capped();
                frames.keep(animates(run, state.reduce_motion) || self.ceremonies.playing(&run.session, state.reduce_motion, Instant::now()));
                frames.keep(!self.blends.is_empty() || !self.arrivals.is_empty());
                frames.finish(window);
                let finished = (run.status == RunStatus::Done).then(|| summary(run, &t, cx));
                let legend = legend(run, &t, cx);
                let this = cx.entity();
                v_flex().size_full()
                    .when_some(stale, |this, text| this.child(h_flex().gap(theme::space(d, SpaceStep::Sm))
                        .px(theme::space(d, SpaceStep::Md)).child(ornament::stale_note(&t, text).flex_1().min_w_0()).child(retry())))
                    .child(legend)
                    .when_some(finished, |this, card| this.child(card))
                    .child(div().flex_1().min_h_0().relative().child(container_query(move |panel, window, cx| {
                        this.update(cx, |this, cx| this.graph_view(panel, window, cx))
                    })))
                    .into_any_element()
            }
        }
    }
}

/// A plausible large run, for the demo scene and the tests.
fn large_run(mut run: Run, modules: usize, units: usize) -> Run {
    const STATES: [UnitState; 19] = [
        UnitState::Verified,
        UnitState::Verified,
        UnitState::Pending,
        UnitState::Verified,
        UnitState::Running,
        UnitState::Verified,
        UnitState::Pending,
        UnitState::Built,
        UnitState::Verified,
        UnitState::Pending,
        UnitState::Verified,
        UnitState::Failed,
        UnitState::Verified,
        UnitState::Pending,
        UnitState::Verified,
        UnitState::Running,
        UnitState::Verified,
        UnitState::Pending,
        UnitState::Built,
    ];
    run.modules.clear();
    run.units.clear();
    run.goals.clear();
    let per = units / modules.max(1);
    for m in 0..modules {
        let count = if m + 1 == modules { units - per * (modules - 1) } else { per };
        let mut indices = vec![];
        for u in 0..count {
            let state = STATES[(m * 7 + u) % STATES.len()];
            indices.push(run.units.len());
            run.units.push(Unit {
                id: format!("m{m:02}_task_{u:03}").into(),
                title: format!("Build part {u} of module {m}").into(),
                module: m,
                state,
                raw_state: state.label().to_lowercase().into(),
                attempts: u32::from(state != UnitState::Pending),
                files: UnitFiles::default(),
            });
        }
        run.modules.push(Module {
            id: format!("module_{m:02}").into(),
            purpose: format!("The purpose of module {m}").into(),
            depends: m.checked_sub(1).into_iter().collect(),
            units: indices,
            contract: None,
        });
    }
    run
}

// walk list: gpui-only, verify in a window: selection and hover glow fade, Empty/Loading/Failed/Stale panels, legend and Core tooltips, graph fills the window in focus mode; right-click and shift-f10 open the menu on a unit and a module in a window; a menu open while the run list reorders acts on the right id; Escape/click-away closes it and focus returns to the graph.
#[cfg(test)]
mod accept;

#[cfg(test)]
mod tests {
    use super::super::demo::sample_run;
    use super::*;
    // The kit's glob import brings gpui's `test` macro, which would otherwise shadow the standard one.
    use std::prelude::v1::test;
    use UnitState::*;

    const WIDTHS: [f32; 4] = [300.0, 640.0, 1120.0, 1900.0];

    #[test]
    fn unit_title_uses_plain_words_and_keeps_original_in_tooltip() {
        let mut unit = sample_run().units[0].clone();
        unit.id = "unit-1".into();
        unit.title = "Technical title".into();
        let title = plain_text::Plain { shown: "Plain title".into(), original: Some("Technical title".into()) };

        assert_eq!(unit_chip_label(&unit, &title).as_ref(), "Plain title");
        assert_eq!(unit_tooltip_lines(&unit, title), vec![SharedString::from("Plain title"), "Technical title".into()]);

        // Even a title matching the ID needs both lines when it was translated.
        unit.title = unit.id.clone();
        let title = plain_text::Plain { shown: "Plain ID".into(), original: Some(unit.id.clone()) };
        assert_eq!(unit_tooltip_lines(&unit, title), vec![SharedString::from("Plain ID"), unit.id.clone()]);
    }

    #[test]
    fn unit_title_without_translation_keeps_existing_tooltip_fallback() {
        let mut unit = sample_run().units[0].clone();
        unit.id = "unit-1".into();
        unit.title = "Technical title".into();
        let title = plain_text::Plain { shown: unit.title.clone(), original: None };

        assert_eq!(unit_chip_label(&unit, &title).as_ref(), "Technical title");
        assert_eq!(unit_tooltip_lines(&unit, title), vec![SharedString::from("Technical title")]);

        let title = plain_text::Plain { shown: "Supplied text".into(), original: None };
        assert_eq!(unit_chip_label(&unit, &title).as_ref(), "Supplied text");
        assert_eq!(unit_tooltip_lines(&unit, title), vec![SharedString::from("Supplied text")]);

        unit.title = unit.id.clone();
        let title = plain_text::Plain { shown: unit.title.clone(), original: None };
        assert!(unit_tooltip_lines(&unit, title).is_empty());
    }

    #[test]
    fn empty_unit_title_uses_id_for_chip() {
        let mut unit = sample_run().units[0].clone();
        unit.id = "unit-1".into();
        unit.title = "".into();
        let title = plain_text::Plain { shown: "".into(), original: None };
        assert_eq!(unit_chip_label(&unit, &title).as_ref(), "unit-1");

        unit.title = "Technical title".into();
        let title = plain_text::Plain { shown: "Plain title".into(), original: Some(unit.title.clone()) };
        assert_eq!(unit_chip_label(&unit, &title).as_ref(), "Plain title");
    }

    fn run_of(lanes: &[&[UnitState]]) -> Run {
        let mut run = sample_run();
        run.modules.clear();
        run.units.clear();
        for (m, states) in lanes.iter().enumerate() {
            let mut units = vec![];
            for (u, state) in states.iter().enumerate() {
                units.push(run.units.len());
                run.units.push(Unit {
                    id: format!("m{m}u{u}").into(),
                    title: format!("Title {m} {u}").into(),
                    module: m,
                    state: *state,
                    raw_state: "x".into(),
                    attempts: 1,
                    files: UnitFiles::default(),
                });
            }
            run.modules.push(Module {
                id: format!("m{m}").into(),
                purpose: "p".into(),
                depends: vec![],
                units,
                contract: None,
            });
        }
        run
    }

    fn assert_no_overlaps(graph: &Graph) {
        let mut rects: Vec<Rect> = graph.modules.iter().map(|m| m.rect).collect();
        rects.extend(graph.chips.iter().map(|c| c.rect));
        for (i, a) in rects.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= graph.width + 0.01, "{a:?} leaves the panel at width {}", graph.width);
            assert!(a.y >= 0.0 && a.bottom() <= graph.height + 0.01, "{a:?} leaves the content");
            for b in &rects[i + 1..] {
                assert!(!a.overlaps(b), "{a:?} overlaps {b:?} at width {}", graph.width);
            }
        }
    }

    #[test]
    fn nothing_overlaps_or_leaves_the_panel_at_any_scale() {
        for width in WIDTHS {
            assert_no_overlaps(&layout(&sample_run(), width, 600.0, &Selection::None));
            assert_no_overlaps(&layout(&large_run(sample_run(), 14, 320), width, 600.0, &Selection::None));
            assert_no_overlaps(&layout(&large_run(sample_run(), 60, 900), width, 480.0, &Selection::None));
            assert_no_overlaps(&layout(&large_run(sample_run(), 14, 320), width, 600.0, &Selection::Module(3)));
        }
    }

    #[test]
    fn every_unit_is_shown_or_counted_in_a_fold() {
        for run in [sample_run(), large_run(sample_run(), 14, 320), large_run(sample_run(), 3, 500)] {
            let graph = layout(&run, 640.0, 600.0, &Selection::None);
            for (index, module) in run.modules.iter().enumerate() {
                let total: usize = graph.chips.iter().filter(|c| c.module == index).map(|c| c.count).sum();
                assert_eq!(total, module.units.len(), "module {}", module.id);
            }
        }
    }

    #[test]
    fn a_small_module_shows_every_unit() {
        let run = run_of(&[&[Verified, Verified, Verified, Pending, Pending, Running]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert_eq!(graph.chips.len(), 6);
        assert!(graph.chips.iter().all(|c| matches!(c.node, Node::Unit(_))));
    }

    #[test]
    fn a_large_module_folds_quiet_units_and_keeps_active_work_and_failures() {
        let mut lane = vec![Verified; 12];
        lane.extend([Running, Failed, Built]);
        lane.extend([Pending; 5]);
        let run = run_of(&[lane.as_slice()]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let shown: Vec<Node> = graph.chips.iter().map(|c| c.node).collect();
        assert_eq!(
            shown,
            vec![
                Node::Unit(12),
                Node::Unit(13),
                Node::Unit(14),
                Node::Folded(0, Verified),
                Node::Folded(0, Pending),
            ]
        );
        let fold = graph.chips.iter().find(|c| c.node == Node::Folded(0, Verified)).unwrap();
        assert_eq!(fold.count, 12);
    }

    #[test]
    fn one_quiet_unit_is_not_folded_alone() {
        let mut lane = vec![Running; 8];
        lane.push(Verified);
        let run = run_of(&[lane.as_slice()]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert!(graph.chips.iter().any(|c| c.node == Node::Unit(8)));
        assert!(!graph.chips.iter().any(|c| matches!(c.node, Node::Folded(..))));
    }

    #[test]
    fn a_unit_selected_elsewhere_comes_out_of_its_fold() {
        let run = run_of(&[&[Verified; 12]]);
        let folded = layout(&run, 640.0, 600.0, &Selection::None);
        assert_eq!(folded.chips.len(), 1);
        assert_eq!(folded.rect_of(Node::Unit(5)), None);
        let graph = layout(&run, 640.0, 600.0, &Selection::Unit(5));
        assert!(graph.rect_of(Node::Unit(5)).is_some());
        let fold = graph.chips.iter().find(|c| matches!(c.node, Node::Folded(..))).unwrap();
        assert_eq!(fold.count, 11);
    }

    #[test]
    fn selecting_a_module_lists_all_its_units() {
        let run = run_of(&[&[Verified; 12], &[Verified; 12]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::Module(1));
        assert_eq!(graph.chips.iter().filter(|c| c.module == 1).count(), 12);
        assert_eq!(graph.chips.iter().filter(|c| c.module == 0).count(), 1);
    }

    #[test]
    fn module_state_prefers_work_in_progress() {
        let run = run_of(&[
            &[Running, Failed],
            &[Failed, Built],
            &[Built, Pending],
            &[Verified, Pending],
            &[Verified, Verified],
            &[],
        ]);
        let states: Vec<UnitState> = run.modules.iter().map(|m| module_state(&run, m)).collect();
        assert_eq!(states, vec![Running, Failed, Built, Pending, Verified, Pending]);
    }

    #[test]
    fn a_run_without_modules_lays_out_nothing() {
        let run = run_of(&[]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert!(graph.modules.is_empty() && graph.chips.is_empty());
        assert_eq!(graph.height, 600.0);
    }

    #[test]
    fn layout_handles_degenerate_panels() {
        for (w, h) in [(0.0, 0.0), (10.0, 10.0), (300.0, 1.0)] {
            layout(&large_run(sample_run(), 14, 320), w, h, &Selection::None);
        }
    }

    #[test]
    fn layout_is_deterministic() {
        let run = large_run(sample_run(), 14, 320);
        assert_eq!(layout(&run, 640.0, 600.0, &Selection::None), layout(&run, 640.0, 600.0, &Selection::None));
    }

    #[test]
    fn a_short_graph_is_centred_and_a_tall_one_scrolls() {
        let run = run_of(&[&[Pending], &[Pending]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert_eq!(graph.height, 600.0);
        let top = graph.modules[0].rect.y;
        let bottom = 600.0 - graph.modules[1].rect.bottom();
        assert!((top - bottom).abs() < 0.5, "top {top}, bottom {bottom}");

        let tall = layout(&large_run(sample_run(), 60, 900), 640.0, 600.0, &Selection::None);
        assert!(tall.height > 600.0);
    }

    #[test]
    fn thirteen_modules_fit_the_default_window_without_scrolling() {
        let graph = layout(&sample_run(), 640.0, 660.0, &Selection::None);
        assert_eq!(graph.height, 660.0);
    }

    #[test]
    fn wires_start_at_the_card_and_end_at_the_chip() {
        for width in WIDTHS {
            let run = sample_run();
            let graph = layout(&run, width, 600.0, &Selection::None);
            for chip in &graph.chips {
                let wire = graph.chip_wire(chip, &View::default());
                let card = graph.modules[chip.module].rect;
                assert_eq!(wire.at(0.0), Pt { x: card.right(), y: card.cy() });
                let end = wire.at(1.0);
                assert!((end.x - chip.rect.x).abs() < 0.01 && (end.y - chip.rect.cy()).abs() < 0.01);
            }
        }
    }

    fn panned(dy: f32) -> View {
        View { zoom: 1.0, pan: Pt { x: 0.0, y: dy } }
    }

    #[test]
    fn panning_moves_wires_with_the_content() {
        let run = sample_run();
        let graph = layout(&run, 640.0, 300.0, &Selection::None);
        let chip = &graph.chips[3];
        assert_eq!(graph.chip_wire(chip, &panned(-40.0)).to.y, chip.rect.cy() - 40.0);
        let m = &graph.modules[2];
        assert_eq!(graph.core_wire(m, 300.0, &panned(-40.0)).from.y, 150.0);
        assert_eq!(graph.core_wire(m, 300.0, &panned(-40.0)).to.y, m.rect.cy() - 40.0);
    }

    #[test]
    fn zoomed_wires_still_join_card_and_chip() {
        let graph = layout(&sample_run(), 640.0, 600.0, &Selection::None);
        let view = View { zoom: 1.7, pan: Pt { x: -130.0, y: -260.0 } };
        for chip in &graph.chips {
            let wire = graph.chip_wire(chip, &view);
            let card = view.rect(graph.modules[chip.module].rect);
            let target = view.rect(chip.rect);
            assert!((wire.at(0.0).x - card.right()).abs() < 0.01 && (wire.at(0.0).y - card.cy()).abs() < 0.01);
            let end = wire.at(1.0);
            assert!((end.x - target.x).abs() < 0.01 && (end.y - target.cy()).abs() < 0.01);
        }
        for m in &graph.modules {
            let wire = graph.core_wire(m, 600.0, &view);
            let card = view.rect(m.rect);
            assert_eq!(wire.to, Pt { x: card.x, y: card.cy() });
        }
    }

    #[test]
    fn zooming_about_a_point_keeps_that_point_still_and_stays_in_range() {
        let start = View { zoom: 0.8, pan: Pt { x: 30.0, y: -120.0 } };
        let anchor = Pt { x: 412.0, y: 233.0 };
        let under = start.to_graph(anchor);
        for factor in [1.25, 0.5, wheel_zoom(120.0), wheel_zoom(-120.0)] {
            let zoomed = start.zoomed_about(anchor, factor);
            let back = zoomed.to_screen(under);
            assert!((back.x - anchor.x).abs() < 1e-3 && (back.y - anchor.y).abs() < 1e-3, "{factor}");
        }
        assert!(wheel_zoom(20.0) > 1.0 && wheel_zoom(-20.0) < 1.0);
        assert!((wheel_zoom(20.0) * wheel_zoom(-20.0) - 1.0).abs() < 1e-6, "in and out by the same amount");
        assert_eq!(start.zoomed_about(anchor, 1000.0).zoom, MAX_ZOOM);
        assert_eq!(start.zoomed_about(anchor, 0.0001).zoom, MIN_ZOOM);
        let p = Pt { x: 17.0, y: -3.5 };
        let round = start.to_graph(start.to_screen(p));
        assert!((round.x - p.x).abs() < 1e-4 && (round.y - p.y).abs() < 1e-4);
    }

    #[test]
    fn hit_testing_stays_exact_at_any_zoom_and_pan() {
        let run = large_run(sample_run(), 14, 320);
        let graph = layout(&run, 900.0, 600.0, &Selection::None);
        for view in [
            View::default(),
            View { zoom: 0.35, pan: Pt { x: 40.0, y: 10.0 } },
            View { zoom: 2.6, pan: Pt { x: -700.0, y: -900.0 } },
            View::default().zoomed_about(Pt { x: 300.0, y: 200.0 }, 1.9).panned(-55.0, 80.0),
        ] {
            for chip in &graph.chips {
                let r = view.rect(chip.rect);
                // Just inside each corner hits the chip; just outside its left edge does not.
                for p in [Pt { x: r.x + 0.5, y: r.y + 0.5 }, Pt { x: r.right() - 0.5, y: r.bottom() - 0.5 }] {
                    assert_eq!(graph.hit(p, 600.0, &view), Some(chip.node), "{view:?}");
                }
                let left = Pt { x: r.x - 0.5, y: r.cy() };
                assert_ne!(graph.hit(left, 600.0, &view), Some(chip.node), "{view:?}");
            }
            for m in &graph.modules {
                let r = view.rect(m.rect);
                assert_eq!(graph.hit(Pt { x: r.x + 1.0, y: r.cy() }, 600.0, &view), Some(Node::Module(m.index)));
            }
            assert_eq!(graph.hit(graph.core(600.0, &view), 600.0, &view), Some(Node::Core));
        }
    }

    #[test]
    fn panning_keeps_the_graph_in_reach() {
        let graph = layout(&sample_run(), 640.0, 600.0, &Selection::None);
        let content = graph.content().unwrap();
        for view in [View::default().panned(-90_000.0, 90_000.0), View::default().panned(90_000.0, -90_000.0)] {
            let kept = clamp_view(view, Some(content), 640.0, 600.0);
            let shown = kept.rect(content);
            let panel = Rect { x: 0.0, y: 0.0, w: 640.0, h: 600.0 };
            assert!(shown.overlaps(&panel), "{shown:?}");
            assert!(shown.right().min(640.0) - shown.x.max(0.0) >= KEEP_VISIBLE - 0.01);
        }
        let inside = View { zoom: 1.0, pan: Pt { x: 5.0, y: -5.0 } };
        assert_eq!(clamp_view(inside, Some(content), 640.0, 600.0), inside, "a reasonable view is left alone");
        assert_eq!(clamp_view(View { zoom: 99.0, ..inside }, None, 640.0, 600.0).zoom, MAX_ZOOM);
    }

    #[test]
    fn fit_frames_everything_and_never_enlarges() {
        for run in [sample_run(), large_run(sample_run(), 14, 320), large_run(sample_run(), 60, 900)] {
            let graph = layout(&run, 640.0, 480.0, &Selection::None);
            let content = graph.content().unwrap();
            let view = fit_view(content, 640.0, 480.0);
            assert!(view.zoom <= 1.0 && view.zoom >= MIN_ZOOM);
            let shown = view.rect(content);
            if view.zoom > MIN_ZOOM {
                assert!(shown.x >= -0.01 && shown.right() <= 640.01, "{shown:?}");
                assert!(shown.y >= -0.01 && shown.bottom() <= 480.01 - CONTROLS_BAND, "Fit leaves the controls' strip free: {shown:?}");
            }
            assert!((shown.x + shown.w / 2.0 - 320.0).abs() < 0.01, "centred across");
            assert!((shown.cy() - (480.0 - CONTROLS_BAND) / 2.0).abs() < 0.01, "centred above the controls' strip");
            assert_eq!(clamp_view(view, Some(content), 640.0, 480.0), view, "a fitted view needs no clamping");
        }
        let tall = layout(&large_run(sample_run(), 60, 900), 640.0, 480.0, &Selection::None);
        assert!(fit_view(tall.content().unwrap(), 640.0, 480.0).zoom < 0.5, "a tall run is shrunk to fit");
    }

    #[test]
    fn a_fitted_graph_never_reaches_the_zoom_controls() {
        let controls = controls_rect(640.0, 480.0);
        for run in [sample_run(), large_run(sample_run(), 14, 320), large_run(sample_run(), 60, 900)] {
            let graph = layout(&run, 640.0, 480.0, &Selection::None);
            let view = fit_view(graph.content().unwrap(), 640.0, 480.0);
            if view.zoom <= MIN_ZOOM {
                continue; // too big to frame at the smallest zoom
            }
            let shown = view.rect(graph.content().unwrap());
            assert!(!shown.overlaps(&controls), "{shown:?} over {controls:?}");
        }
    }

    #[test]
    fn a_tooltip_stays_clear_of_the_zoom_controls() {
        let (w, h) = (900.0, 600.0);
        let c = controls_rect(w, h);
        let rect = |p: TipPlace| Rect { x: p.at.x, y: if p.from_bottom { p.at.y - TIP_MAX_H } else { p.at.y }, w: TIP_W, h: TIP_MAX_H };
        // Beside a node near the bottom right, it would cover the controls.
        let node = Rect { x: 560.0, y: 520.0, w: 90.0, h: 24.0 };
        let wanted = Pt { x: node.right() + 8.0, y: node.y - 2.0 };
        assert!(Rect { x: wanted.x, y: wanted.y, w: TIP_W, h: TIP_MAX_H }.overlaps(&c), "the case needs a clash");
        let placed = place_tip(wanted, node, w, h);
        assert!(!rect(placed).overlaps(&c), "{placed:?}");
        assert!(!rect(placed).overlaps(&node), "{placed:?}");
        // A tooltip that is nowhere near the controls is left exactly where it was.
        let far = Pt { x: 40.0, y: 60.0 };
        assert_eq!(place_tip(far, Rect { x: 20.0, y: 58.0, w: 10.0, h: 20.0 }, w, h), TipPlace { at: far, from_bottom: false });
        // Under a module whose card sits above the controls, it goes beside them instead of over the card.
        let card = Rect { x: 700.0, y: 400.0, w: 150.0, h: 40.0 };
        let below = Pt { x: card.x, y: card.bottom() + 6.0 };
        let placed = place_tip(below, card, w, h);
        assert!(!rect(placed).overlaps(&c) && !rect(placed).overlaps(&card), "{placed:?}");
    }

    #[test]
    fn a_tooltip_pushed_into_the_window_still_clears_the_controls() {
        let (w, h) = (900.0, 600.0);
        let c = controls_rect(w, h);
        // A node in the bottom-right corner: beside it, the tooltip starts right of the controls,
        // but the window pushes it back left, over them.
        let node = Rect { x: w - 100.0, y: h - 60.0, w: 90.0, h: 24.0 };
        let wanted = Pt { x: node.right() + 8.0, y: node.y - 2.0 };
        let placed = place_tip(wanted, node, w, h);
        let shown = tip_rect(placed, w, h);
        assert!(!shown.overlaps(&c), "{placed:?} lands on {shown:?}, over {c:?}");
        assert!(!shown.overlaps(&node), "{placed:?} lands on {shown:?}, over {node:?}");
        assert!(shown.right() <= w && shown.bottom() <= h);
        // Away from every edge the snapped place is the asked one.
        let mid = TipPlace { at: Pt { x: 40.0, y: 60.0 }, from_bottom: false };
        assert_eq!(tip_rect(mid, w, h), Rect { x: 40.0, y: 60.0, w: TIP_W, h: TIP_MAX_H });
    }

    #[test]
    fn revealing_moves_only_when_the_node_is_out_of_sight() {
        let rect = Rect { x: 100.0, y: 500.0, w: 10.0, h: 24.0 };
        let shown = panned(-400.0);
        assert_eq!(reveal_view(shown, rect, 640.0, 300.0), shown);
        let moved = reveal_view(View::default(), rect, 640.0, 300.0);
        assert_eq!(moved.pan.y, -(512.0 - 150.0));
        let zoomed = View { zoom: 2.0, pan: Pt { x: -900.0, y: 0.0 } };
        let back = reveal_view(zoomed, rect, 640.0, 300.0);
        let r = back.rect(rect);
        assert!(r.x >= 0.0 && r.right() <= 640.0 && r.y >= REVEAL_MARGIN && r.bottom() <= 300.0 - REVEAL_MARGIN, "{r:?}");
    }

    #[test]
    fn an_overflowing_run_opens_fitted() {
        let run = large_run(sample_run(), 60, 900);
        let graph = panel_layout(&run, 640.0, 600.0, &Selection::None);
        assert!(graph.modules[0].rect.w >= 240.0, "narrow overflow retains wide cards");
        assert!(graph.chips.iter().any(|c| c.rect.x != graph.chips[0].rect.x), "overflow uses two chip columns");
        let view = initial_view(&graph, 640.0, 600.0);
        let content = graph.content().unwrap();
        let expected = (640.0 / content.w).min((600.0 - CONTROLS_BAND) / content.h).min(1.0);
        assert!((view.zoom - expected).abs() < 0.0001, "largest fitting zoom: {view:?}, expected {expected}");
        for rect in graph.modules.iter().map(|m| m.rect).chain(graph.chips.iter().map(|c| c.rect)) {
            let shown = view.rect(rect);
            assert!(shown.x >= -0.01 && shown.right() <= 640.01 && shown.y >= -0.01 && shown.bottom() <= 600.01 - CONTROLS_BAND, "{shown:?}");
        }
    }

    #[test]
    fn fitted_zoom_below_interactive_floor_is_preserved() {
        let run = large_run(sample_run(), 60, 900);
        let graph = panel_layout(&run, 640.0, 600.0, &Selection::None);
        let fitted = initial_view(&graph, 640.0, 600.0);
        assert!(fitted.zoom < MIN_ZOOM);
        assert_eq!(clamp_view(fitted, graph.content(), 640.0, 600.0), fitted);
        let zoomed_out = fitted.zoomed_about(Pt { x: 320.0, y: 300.0 }, 0.8);
        assert!((zoomed_out.zoom - fitted.zoom).abs() < 0.000001);
        assert!((zoomed_out.pan.x - fitted.pan.x).abs() < 0.001 && (zoomed_out.pan.y - fitted.pan.y).abs() < 0.001);
        let zoomed_in = fitted.zoomed_about(Pt { x: 320.0, y: 300.0 }, 1.25);
        assert!((zoomed_in.zoom - fitted.zoom * 1.25).abs() < 0.000001);
    }

    #[test]
    fn panel_layout_keeps_regular_layout_when_wide_or_not_overflowing() {
        let small = run_of(&[&[Running]]);
        let large = large_run(sample_run(), 60, 900);
        for (run, width) in [(&small, 640.0), (&large, 720.0)] {
            let normal = layout(run, width, 600.0, &Selection::None);
            let panel = panel_layout(run, width, 600.0, &Selection::None);
            assert_eq!(panel.modules.iter().map(|m| m.rect).collect::<Vec<_>>(), normal.modules.iter().map(|m| m.rect).collect::<Vec<_>>());
            assert_eq!(panel.chips.iter().map(|c| c.rect).collect::<Vec<_>>(), normal.chips.iter().map(|c| c.rect).collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_small_run_opens_at_true_size() {
        let run = run_of(&[&[Running]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert_eq!(initial_view(&graph, 640.0, 600.0).zoom, 1.0);
    }

    #[test]
    fn arrow_keys_walk_modules_units_and_siblings_in_order() {
        let run = run_of(&[&[Running, Failed, Pending], &[Verified], &[]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let walk = |start: Option<Node>, steps: &[Step]| {
            let mut at = start;
            steps.iter().map(|&s| {
                at = graph.step(at, s);
                at.unwrap()
            }).collect::<Vec<_>>()
        };
        use Step::*;
        assert_eq!(
            walk(None, &[Down, Right, Down, Down, Down, Up, Left, Down, Right, Left, Down, Right, Down, Left, Left, Left]),
            vec![
                Node::Module(0),
                Node::Unit(0),
                Node::Unit(1),
                Node::Unit(2),
                Node::Unit(2),
                Node::Unit(1),
                Node::Module(0),
                Node::Module(1),
                Node::Unit(3),
                Node::Module(1),
                Node::Module(2),
                Node::Module(2),
                Node::Module(2),
                Node::Core,
                Node::Core,
                Node::Core,
            ]
        );
        assert_eq!(graph.step(Some(Node::Core), Right), Some(Node::Module(0)));
        assert_eq!(graph.step(Some(Node::Module(0)), Up), Some(Node::Module(0)));
        // A cursor on a unit that has been folded away starts again from the top.
        assert_eq!(graph.step(Some(Node::Unit(999)), Down), Some(Node::Module(0)));
        assert_eq!(layout(&run_of(&[]), 640.0, 600.0, &Selection::None).step(None, Down), Some(Node::Core));
    }

    #[test]
    fn arrow_keys_reach_a_fold_and_enter_opens_its_module() {
        let mut lane = vec![Verified; 10];
        lane.push(Running);
        let run = run_of(&[lane.as_slice()]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let fold = graph.step(graph.step(Some(Node::Module(0)), Step::Right), Step::Down);
        assert_eq!(fold, Some(Node::Folded(0, Verified)));
        assert_eq!(selection_of(fold), Selection::Module(0));
        assert_eq!(selection_of(Some(Node::Unit(3))), Selection::Unit(3));
        assert_eq!(selection_of(Some(Node::Core)), Selection::None);
    }

    #[test]
    fn wires_to_the_second_column_pass_between_first_column_chips() {
        let graph = layout(&large_run(sample_run(), 4, 80), 640.0, 600.0, &Selection::Module(1));
        let first_x = graph.chips.iter().map(|c| c.rect.x).fold(f32::MAX, f32::min);
        let seconds: Vec<&ChipBox> = graph.chips.iter().filter(|c| c.rect.x > first_x).collect();
        assert!(!seconds.is_empty());
        for chip in seconds {
            let y = chip.rect.cy();
            for other in graph.chips.iter().filter(|c| c.rect.x == first_x) {
                assert!(y < other.rect.y || y > other.rect.bottom(), "wire at {y} crosses {:?}", other.rect);
            }
        }
    }

    #[test]
    fn hit_testing_finds_nodes_and_empty_space() {
        let run = sample_run();
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let view = View::default();
        for chip in &graph.chips {
            let p = Pt { x: chip.rect.x + 5.0, y: chip.rect.cy() };
            assert_eq!(graph.hit(p, 600.0, &view), Some(chip.node));
        }
        for m in &graph.modules {
            let p = Pt { x: m.rect.x + 5.0, y: m.rect.cy() };
            assert_eq!(graph.hit(p, 600.0, &view), Some(Node::Module(m.index)));
        }
        let core = graph.core(600.0, &view);
        assert_eq!(graph.hit(core, 600.0, &view), Some(Node::Core));
        let gap = Pt { x: graph.modules[0].rect.x - 6.0, y: graph.modules[0].rect.cy() };
        assert_eq!(graph.hit(gap, 600.0, &view), None);
    }

    #[test]
    fn hit_testing_follows_the_pan_but_the_core_stays_mid_panel() {
        let run = large_run(sample_run(), 60, 900);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let m = &graph.modules[30];
        let view = panned(-(m.rect.y - 100.0));
        let p = Pt { x: m.rect.x + 5.0, y: 100.0 + m.rect.h / 2.0 };
        assert_eq!(graph.hit(p, 600.0, &view), Some(Node::Module(30)));
        assert_eq!(graph.core(600.0, &view).y, 300.0);
        assert_eq!(graph.hit(graph.core(600.0, &view), 600.0, &view), Some(Node::Core));
    }

    #[test]
    fn paths_cover_the_right_wires() {
        let run = run_of(&[&[Running, Verified], &[Pending]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let (a, b, c) = (&graph.chips[0], &graph.chips[1], &graph.chips[2]);
        let unit = a.node;
        assert!(on_core_wire(&graph, unit, 0) && !on_core_wire(&graph, unit, 1));
        assert!(on_chip_wire(unit, a) && !on_chip_wire(unit, b) && !on_chip_wire(unit, c));
        let module = Node::Module(0);
        assert!(on_core_wire(&graph, module, 0) && !on_core_wire(&graph, module, 1));
        assert!(on_chip_wire(module, a) && on_chip_wire(module, b) && !on_chip_wire(module, c));
    }

    #[test]
    fn only_running_work_flows() {
        let run = run_of(&[&[Running, Verified], &[Pending]]);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        assert!(graph.live);
        assert!(graph.modules[0].running && !graph.modules[1].running);
        let idle = run_of(&[&[Failed, Built], &[Pending]]);
        assert!(!layout(&idle, 640.0, 600.0, &Selection::None).live);
    }

    fn run_as(session: &'static str, status: RunStatus, hosted: bool) -> Run {
        let mut run = run_of(&[&[Verified]]);
        run.session = session.into();
        run.status = status;
        run.hosted = hosted;
        run
    }

    fn no_renames() -> HashMap<SharedString, SharedString> {
        HashMap::new()
    }

    #[test]
    fn nothing_plays_for_runs_that_were_there_when_the_app_opened() {
        let mut watch = Watch::default();
        watch.observe(&[run_as("a", RunStatus::Active, true), run_as("b", RunStatus::Done, true)], &no_renames());
        let same = [run_as("a", RunStatus::Active, true), run_as("b", RunStatus::Done, true)];
        assert!(watch.observe(&same, &no_renames()).is_empty());
    }

    #[test]
    fn a_wish_started_here_plays_the_orbit_once_and_only_when_unfinished() {
        let mut watch = Watch::default();
        watch.observe(&[], &no_renames());
        let runs = [run_as("new", RunStatus::Active, true)];
        assert_eq!(watch.observe(&runs, &no_renames()), vec![Moment::Started("new".into())]);
        assert!(watch.observe(&runs, &no_renames()).is_empty(), "the same wish never starts twice");
        let history = [run_as("new", RunStatus::Active, true), run_as("old", RunStatus::Done, true)];
        assert!(watch.observe(&history, &no_renames()).is_empty(), "a finished run found later is history");
        let elsewhere = [run_as("new", RunStatus::Active, true), run_as("term", RunStatus::Active, false)];
        assert!(watch.observe(&elsewhere, &no_renames()).is_empty(), "a run from a terminal is not a wish started here");
    }

    #[test]
    fn a_provisional_id_becoming_the_real_one_is_not_a_second_start() {
        let mut watch = Watch::default();
        watch.observe(&[], &no_renames());
        assert_eq!(watch.observe(&[run_as("temp", RunStatus::Active, true)], &no_renames()).len(), 1);
        let renames: HashMap<SharedString, SharedString> = [("real".into(), "temp".into())].into();
        assert!(watch.observe(&[run_as("real", RunStatus::Active, true)], &renames).is_empty());
    }

    #[test]
    fn the_hourglass_turns_once_when_a_run_seen_unfinished_ends() {
        let mut watch = Watch::default();
        watch.observe(&[run_as("a", RunStatus::Active, true), run_as("b", RunStatus::Done, true)], &no_renames());
        let ended = [run_as("a", RunStatus::Done, true), run_as("b", RunStatus::Done, true)];
        assert_eq!(watch.observe(&ended, &no_renames()), vec![Moment::Ended("a".into())]);
        assert!(watch.observe(&ended, &no_renames()).is_empty());
        // A run first seen already finished, even after the app has been running, never turns it.
        let late = [run_as("c", RunStatus::Done, false)];
        assert!(watch.observe(&late, &no_renames()).is_empty());
    }

    #[test]
    fn ceremonies_last_about_a_second_and_a_half_play_once_and_then_stop_asking_for_frames() {
        assert!((1.2..=2.0).contains(&CEREMONY_SECONDS));
        let session: SharedString = "a".into();
        let t0 = Instant::now();
        let at = |secs: f32| t0 + Duration::from_secs_f32(secs);
        let mut c = Ceremonies::default();
        c.observe(&[], &no_renames(), false, t0);
        c.observe(&[run_as("a", RunStatus::Active, true)], &no_renames(), false, t0);
        assert!(c.playing(&session, false, at(0.0)));
        assert_eq!(c.orbit(&session, false, at(5.0)), Some(0.0), "the clock starts when the graph is first drawn");
        assert!(c.orbit(&session, false, at(5.8)).is_some_and(|p| p > 0.4 && p < 0.6));
        assert!(c.orbit(&session, false, at(5.0 + CEREMONY_SECONDS + 0.01)).is_none());
        assert!(!c.playing(&session, false, at(9.0)), "settled: no more frames");
        assert!(c.orbit(&session, false, at(9.1)).is_none(), "it does not play again");
        c.observe(&[run_as("a", RunStatus::Done, true)], &no_renames(), false, at(10.0));
        assert!(c.turn(&session, false, at(10.8)).is_some());
        assert!(c.turn(&session, false, at(10.0 + CEREMONY_SECONDS + 0.01)).is_none());
        assert!(!c.playing(&session, false, at(13.0)));
        assert!(c.turn(&session, false, at(10.5)).is_none(), "once over it never replays");
    }

    #[test]
    fn a_rename_hands_the_ceremonies_to_the_real_session() {
        let t0 = Instant::now();
        let at = |secs: f32| t0 + Duration::from_secs_f32(secs);
        let renames: HashMap<SharedString, SharedString> = [("real".into(), "temp".into())].into();
        let (temp, real): (SharedString, SharedString) = ("temp".into(), "real".into());
        // Renamed after the wish is created and before the graph first draws.
        let mut c = Ceremonies::default();
        c.observe(&[], &no_renames(), false, t0);
        c.observe(&[run_as("temp", RunStatus::Active, true)], &no_renames(), false, t0);
        c.observe(&[run_as("real", RunStatus::Active, true)], &renames, false, at(1.0));
        assert!(c.playing(&real, false, at(1.0)), "the orbit is waiting for the real session's graph");
        assert_eq!(c.orbit(&real, false, at(2.0)), Some(0.0));
        assert!(c.orbit(&temp, false, at(2.0)).is_none());
        // Renamed in the middle of the orbit: it carries on instead of stopping.
        let mut c = Ceremonies::default();
        c.observe(&[], &no_renames(), false, t0);
        c.observe(&[run_as("temp", RunStatus::Active, true)], &no_renames(), false, t0);
        assert_eq!(c.orbit(&temp, false, at(0.0)), Some(0.0));
        c.observe(&[run_as("real", RunStatus::Active, true)], &renames, false, at(0.5));
        assert!(c.orbit(&real, false, at(0.8)).is_some_and(|p| p > 0.0));
        // The turn at the end follows a rename too.
        let mut c = Ceremonies::default();
        c.observe(&[run_as("temp", RunStatus::Active, true)], &no_renames(), false, t0);
        c.observe(&[run_as("temp", RunStatus::Done, true)], &no_renames(), false, at(1.0));
        c.observe(&[run_as("real", RunStatus::Done, true)], &renames, false, at(1.2));
        assert!(c.turn(&real, false, at(1.5)).is_some());
    }

    #[test]
    fn a_node_changing_state_blends_from_the_old_look_to_the_new() {
        for t in crate::desktop::theme::both() {
            let from = node_look(&t, UnitState::Running, t.surface, true);
            let to = node_look(&t, UnitState::Verified, t.surface, true);
            let start = look_between(&from, &to, 0.0);
            let end = look_between(&from, &to, 1.0);
            assert_eq!((Rgba::from(start.border), Rgba::from(start.fill)), (Rgba::from(from.border), Rgba::from(from.fill)));
            assert_eq!((Rgba::from(end.border), Rgba::from(end.fill)), (Rgba::from(to.border), Rgba::from(to.fill)));
            // The running glow fades out rather than vanishing at once.
            let half = look_between(&from, &to, 0.5).glow.expect("still glowing halfway");
            assert!(half.a > 0.0 && half.a < from.glow.unwrap().a);
            assert!(end.glow.is_none_or(|g| g.a < 1e-6));
        }
    }

    #[test]
    fn reduce_motion_skips_both_ceremonies() {
        let session: SharedString = "a".into();
        let t0 = Instant::now();
        let mut c = Ceremonies::default();
        c.observe(&[run_as("a", RunStatus::Active, true)], &no_renames(), true, t0);
        c.observe(&[run_as("n", RunStatus::Active, true), run_as("a", RunStatus::Active, true)], &no_renames(), true, t0);
        c.observe(&[run_as("n", RunStatus::Done, true), run_as("a", RunStatus::Done, true)], &no_renames(), true, t0);
        assert!(!c.playing(&"n".into(), true, t0) && !c.playing(&session, true, t0));
        assert!(c.orbit(&"n".into(), true, t0).is_none() && c.turn(&session, true, t0).is_none());
        // And turning reduce_motion on mid-ceremony stops it at once.
        let mut c = Ceremonies::default();
        c.observe(&[run_as("a", RunStatus::Active, true)], &no_renames(), false, t0);
        c.observe(&[run_as("a", RunStatus::Done, true)], &no_renames(), false, t0);
        assert!(!c.playing(&session, true, t0));
        assert!(c.turn(&session, true, t0).is_none());
    }

    #[test]
    fn a_ceremony_belongs_to_its_own_run() {
        let t0 = Instant::now();
        let mut c = Ceremonies::default();
        c.observe(&[run_as("a", RunStatus::Active, true)], &no_renames(), false, t0);
        c.observe(&[run_as("a", RunStatus::Done, true)], &no_renames(), false, t0);
        assert!(c.turn(&"other".into(), false, t0).is_none());
        assert!(!c.playing(&"other".into(), false, t0));
    }

    #[test]
    fn the_turn_is_one_full_turn_and_the_fade_ends_fully_visible() {
        assert_eq!(hourglass_turn(None), 0.0);
        assert!((hourglass_turn(Some(1.0)) - std::f32::consts::TAU).abs() < 1e-5);
        assert_eq!(orbit_visibility(None), 1.0);
        assert_eq!(orbit_visibility(Some(0.0)), 0.0);
        assert!((orbit_visibility(Some(1.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn nothing_animates_unless_a_unit_is_running_in_an_active_run() {
        let mut run = run_of(&[&[Running]]);
        assert!(animates(&run, false));
        assert!(!animates(&run, true));
        for status in [RunStatus::Waiting, RunStatus::Blocked, RunStatus::Done] {
            run.status = status;
            assert!(!animates(&run, false));
        }
        assert!(!animates(&run_of(&[&[Verified]]), false));
    }

    #[test]
    fn wires_are_orbit_hairlines_that_turn_gold_only_along_running_work() {
        let same = |a: Hsla, b: Hsla| a.h == b.h && a.s == b.s && a.l == b.l;
        for (t, dark) in super::super::theme::both().into_iter().zip([true, false]) {
            for state in [Pending, Verified, Built, Failed] {
                for (width, color) in wire_layers(&t, state, Lit::No, dark) {
                    assert!(same(color, t.orbit), "{state:?} wire is not an orbit line");
                    assert!(width <= 1.0, "{state:?} wire is not a hairline");
                }
            }
            assert!(wire_layers(&t, Running, Lit::No, dark).iter().all(|(_, c)| same(*c, t.gold)));
            assert!(wire_layers(&t, Pending, Lit::Selected, dark).iter().all(|(_, c)| same(*c, t.gold)));
        }
    }

    #[test]
    fn faint_orbits_circle_the_core_through_the_modules_and_the_units() {
        for width in WIDTHS {
            let graph = layout(&sample_run(), width, 600.0, &Selection::None);
            let radii = orbit_radii(&graph);
            assert!((1..=3).contains(&radii.len()), "{} orbits at width {width}", radii.len());
            assert!(radii.windows(2).all(|pair| pair[1] > pair[0]), "{radii:?}");
            let card = graph.modules[0].rect;
            let first = graph.core_x + radii[0];
            assert!(first > card.x && first < card.right(), "the first orbit misses the module bodies");
            assert!(radii[0] > graph.core_r);
            assert!(graph.core_x + radii[radii.len() - 1] < width, "an orbit leaves the panel at width {width}");
        }
        assert!(orbit_radii(&layout(&run_of(&[]), 640.0, 600.0, &Selection::None)).is_empty());
        let wide = layout(&sample_run(), 1120.0, 600.0, &Selection::None);
        assert!(orbit_radii(&wide).len() >= 2);
    }

    #[test]
    fn particles_stay_on_the_wire_and_advance() {
        let wire = Wire { from: Pt { x: 0.0, y: 0.0 }, knee: 100.0, to: Pt { x: 160.0, y: 50.0 } };
        for i in 0..4 {
            let t = particle_t(3.7, wire.length(), i, 4);
            assert!((0.0..1.0).contains(&t));
        }
        let (a, b) = (particle_t(0.0, 200.0, 0, 4), particle_t(0.5, 200.0, 0, 4));
        assert!(b > a);
        let knee_share = wire.curve_len() / wire.length();
        let at_knee = wire.at(knee_share);
        assert!((at_knee.x - 100.0).abs() < 0.01 && (at_knee.y - 50.0).abs() < 0.01);
        assert!(particle_count(40.0) >= 2 && particle_count(4000.0) <= 8);
    }

    #[test]
    fn planning_says_what_is_happening() {
        let mut run = run_of(&[]);
        for stage in &mut run.stages {
            stage.phase = Phase::Waiting;
        }
        assert_eq!(planning_line(&run), "Getting ready…");
        run.stages[0].phase = Phase::Running;
        assert_eq!(planning_line(&run), "Astra is planning…");
        run.stages[0].phase = Phase::Done;
        run.stages[2].phase = Phase::Running;
        assert_eq!(planning_line(&run), "The architect is splitting the work into modules…");
        run.status = RunStatus::Blocked;
        assert_eq!(planning_line(&run), "Stopped before a plan was made.");
    }

    #[test]
    fn every_text_key_has_a_translation() {
        for line in [
            "Astra is planning…",
            "The critic is checking the plan…",
            "The architect is splitting the work into modules…",
            "Getting ready…",
            "Waiting for the background batch…",
            "Stopped before a plan was made.",
            "This run ended without a plan.",
        ] {
            assert!(TABLE.iter().any(|(key, _)| *key == line), "{line}");
        }
    }
}
