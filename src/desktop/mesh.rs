//! The graph on the Run screen: Chronos on the left, then modules, then their units.
//! `layout` is pure and decides every position; the rest of the file draws what it returns.
use super::i18n::{tr, Table};
use super::model::*;
use super::ornament;
use super::theme::{Tokened as _, Tokens};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const TABLE: Table = &[
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

    #[cfg(test)]
    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
}

/// A thing on the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn columns(width: f32) -> Columns {
    let pad = (width * 0.03).clamp(8.0, 20.0);
    let core_r = if width < 420.0 { 15.0 } else { 26.0 };
    let reach = (width * 0.085).clamp(14.0, 96.0);
    let card_w = (width * 0.26).clamp(92.0, 220.0);
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
    let cols = columns(width);
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
    /// The core stays at the middle of the panel while the rest scrolls.
    fn core(&self, view_h: f32) -> Pt {
        Pt { x: self.core_x, y: view_h / 2.0 }
    }

    fn core_wire(&self, m: &ModuleBox, view_h: f32, scroll: f32) -> Wire {
        let from = self.core(view_h);
        Wire {
            from: Pt { x: from.x + self.core_r, y: from.y },
            knee: m.rect.x,
            to: Pt { x: m.rect.x, y: m.rect.cy() - scroll },
        }
    }

    fn chip_wire(&self, chip: &ChipBox, scroll: f32) -> Wire {
        let card = self.modules[chip.module].rect;
        let knee = if chip.rect.x > self.knee + 8.5 { self.knee } else { chip.rect.x };
        Wire {
            from: Pt { x: card.right(), y: card.cy() - scroll },
            knee,
            to: Pt { x: chip.rect.x, y: chip.rect.cy() - scroll },
        }
    }

    /// What is under a point in panel coordinates, given how far the content is scrolled.
    fn hit(&self, p: Pt, view_h: f32, scroll: f32) -> Option<Node> {
        let core = self.core(view_h);
        if (p.x - core.x).hypot(p.y - core.y) <= self.core_r + 4.0 {
            return Some(Node::Core);
        }
        let at = Pt { x: p.x, y: p.y + scroll };
        self.chips
            .iter()
            .find(|c| c.rect.contains(at))
            .map(|c| c.node)
            .or_else(|| self.modules.iter().find(|m| m.rect.contains(at)).map(|m| Node::Module(m.index)))
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

    /// The first thing a person should see when the graph first opens.
    fn most_urgent(&self) -> Option<Rect> {
        let first = |state| self.chips.iter().find(|c| c.state == state).map(|c| c.rect);
        first(UnitState::Running).or_else(|| first(UnitState::Failed)).or_else(|| first(UnitState::Built))
    }
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

/// The scroll position that brings `rect` into view, leaving it alone when it is already there.
fn reveal(rect: Rect, scroll: f32, view_h: f32, content_h: f32) -> f32 {
    if rect.y - REVEAL_MARGIN >= scroll && rect.bottom() + REVEAL_MARGIN <= scroll + view_h {
        return scroll;
    }
    (rect.cy() - view_h / 2.0).clamp(0.0, (content_h - view_h).max(0.0))
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

    fn paint(&self, bounds: Bounds<Pixels>, scroll: f32, window: &mut Window) {
        let origin = Pt { x: f32::from(bounds.origin.x), y: f32::from(bounds.origin.y) };
        let view_h = f32::from(bounds.size.height);
        let graph = &self.graph;
        let core = graph.core(view_h);
        // The sky first: fixed stars and the faint orbits around the core, which never move.
        let stars = ornament::star_field(f32::from(bounds.size.width), view_h);
        ornament::paint_stars(window, (origin.x, origin.y), &stars, self.t.text);
        let arc_color = self.t.orbit.opacity((if self.dark { 0.55 } else { 0.7 }) * self.orbit_in);
        for r in orbit_radii(graph) {
            ornament::paint_orbit_arc(window, origin.x + core.x, origin.y + core.y, r, -88.0, 88.0, arc_color);
        }
        let mut wires: Vec<(Wire, UnitState, Lit, bool, f32)> = Vec::with_capacity(graph.modules.len() * 3);
        for m in &graph.modules {
            let wire = graph.core_wire(m, view_h, scroll);
            let fade = offscreen_fade(wire.to.y, view_h);
            if fade > 0.0 {
                wires.push((wire, m.state, self.lit(Some(m.index), None), graph.live && m.running, fade));
            }
        }
        for chip in &graph.chips {
            let y = chip.rect.y - scroll;
            let card = graph.modules[chip.module].rect.cy() - scroll;
            if (y + CHIP_H).max(card) < 0.0 || y.min(card) > view_h {
                continue;
            }
            let flows = graph.live && chip.state == UnitState::Running;
            wires.push((graph.chip_wire(chip, scroll), chip.state, self.lit(None, Some(chip)), flows, 1.0));
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
        let breath = self.time.map_or(0.5, ornament::breath);
        let halo = self.t.gold.opacity(if graph.live { 0.22 + 0.16 * breath } else { 0.24 });
        soft_glow(window, point(px(center.x), px(center.y)), graph.core_r * 0.96, graph.core_r * 1.1, halo);
        let phase = self.time.map_or(0.0, |time| time * std::f32::consts::TAU / ORBIT_SECONDS);
        paint_emblem(window, center, graph.core_r, &self.t, phase, self.orbit_in, self.turn);
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

pub struct Mesh {
    state: Entity<AppState>,
    scroll: ScrollHandle,
    /// The last layout, kept for pointer events.
    graph: Rc<Graph>,
    origin: Point<Pixels>,
    view_h: f32,
    inside: bool,
    hover: Option<Node>,
    clock: Instant,
    seen_selection: Selection,
    seen_session: Option<SharedString>,
    /// Demo only: a captured scene keeps its hover whatever the real pointer does.
    pinned: bool,
    ceremonies: Ceremonies,
    /// Demo only: holds a ceremony at this point (0..1) so a still can be captured.
    frozen: Option<(Moment, f32)>,
}

impl Mesh {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Runs already on screen when the app opens are the baseline: they never get a ceremony.
        let mut ceremonies = Ceremonies::default();
        {
            let s = state.read(cx);
            ceremonies.watch.observe(&s.runs, &s.renames);
        }
        cx.observe(&state, |this, state, cx| {
            let s = state.read(cx);
            this.ceremonies.observe(&s.runs, &s.renames, s.reduce_motion, Instant::now());
            cx.notify();
        })
        .detach();
        let mut this = Self {
            state,
            scroll: ScrollHandle::new(),
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
            view_h: 0.0,
            inside: false,
            hover: None,
            clock: Instant::now(),
            seen_selection: Selection::None,
            seen_session: None,
            pinned: false,
            ceremonies,
            frozen: None,
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
        self.state.update(cx, |s, cx| {
            let first_with = |run: &Run, state| run.units.iter().position(|u| u.state == state);
            match name {
                "mesh-large" => s.runs[0] = large_run(s.runs[0].clone(), 14, 320),
                "mesh-huge" => s.runs[0] = large_run(s.runs[0].clone(), 40, 900),
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
            _ => {}
        }
    }

    fn panel_point(&self, position: Point<Pixels>) -> Pt {
        Pt {
            x: f32::from(position.x - self.origin.x),
            y: f32::from(position.y - self.origin.y),
        }
    }

    fn scroll_y(&self) -> f32 {
        (-f32::from(self.scroll.offset().y)).max(0.0)
    }

    fn set_hover(&mut self, node: Option<Node>, cx: &mut Context<Self>) {
        if node != self.hover {
            self.hover = node;
            cx.notify();
        }
    }

    fn clicked(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let p = self.panel_point(position);
        let selection = match self.graph.hit(p, self.view_h, self.scroll_y()) {
            Some(Node::Module(m)) | Some(Node::Folded(m, _)) => Selection::Module(m),
            Some(Node::Unit(u)) => Selection::Unit(u),
            Some(Node::Core) | None => Selection::None,
        };
        self.state.update(cx, |s, cx| s.select(selection, cx));
    }

    fn card_view(&self, m: &ModuleBox, run: &Run, t: &Tokens, dark: bool, breath: f32, cx: &App) -> AnyElement {
        let selected = selected_node(&self.state.read(cx).selection) == Some(Node::Module(m.index));
        let hovered = self.hover == Some(Node::Module(m.index));
        let look = node_look(t, m.state, t.surface, dark);
        let border = if selected { t.gold } else if hovered { t.text.opacity(0.7) } else { look.border };
        let shadow = if selected { Some(t.gold.opacity(if dark { 0.35 } else { 0.22 })) } else { look.glow };
        let share = |n: usize| if m.total == 0 { 0.0 } else { n as f32 / m.total as f32 };
        let run_ref = &run.modules[m.index];
        let count = |state| run_ref.units.iter().filter(|&&u| run.units[u].state == state).count();
        let segments = [
            (m.verified, t.ok),
            (count(UnitState::Built), t.review),
            (count(UnitState::Running), t.run),
            (count(UnitState::Failed), t.fail),
        ];
        v_flex()
            .absolute()
            .left(px(m.rect.x))
            .top(px(m.rect.y))
            .w(px(m.rect.w))
            .h(px(m.rect.h))
            .justify_center()
            .gap(px(3.))
            .pl(px(4.))
            .pr(px(8.))
            .rounded(px(4.))
            .border_1()
            .border_color(border)
            .bg(look.fill)
            .when_some(shadow, |this, color| this.shadow(glow(color, 12.)))
            .child(
                h_flex()
                    .gap(px(5.))
                    .child(ornament::body(m.state, 24., t.unit(m.state), t.orbit, look.fill, if m.running { breath } else { 0.0 }))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(if m.state == UnitState::Verified { t.muted } else { t.text }).child(run_ref.id.clone()))
                    .when(m.rect.w >= 120.0, |this| {
                        this.child(div().flex_none().text_size(px(12.)).text_color(t.muted).child(format!("{}/{}", m.verified, m.total)))
                    }),
            )
            .child(
                h_flex()
                    .h(px(2.))
                    .w_full()
                    .overflow_hidden()
                    .bg(t.orbit.opacity(0.6))
                    .children(segments.into_iter().filter(|(n, _)| *n > 0).map(|(n, color)| {
                        let quiet = m.state == UnitState::Verified;
                        div().h_full().w(relative(share(n))).bg(if quiet { color.opacity(0.55) } else { color })
                    })),
            )
            .into_any_element()
    }

    fn chip_view(&self, chip: &ChipBox, run: &Run, t: &Tokens, dark: bool, breath: f32, cx: &App) -> AnyElement {
        let selected = selected_node(&self.state.read(cx).selection) == Some(chip.node);
        let hovered = self.hover == Some(chip.node);
        let look = node_look(t, chip.state, t.surface, dark);
        let border = if selected { t.gold } else if hovered { t.text.opacity(0.7) } else { look.border };
        let shadow = if selected { Some(t.gold.opacity(if dark { 0.35 } else { 0.22 })) } else { look.glow };
        let label: SharedString = match chip.node {
            Node::Unit(u) => run.units[u].id.clone(),
            Node::Folded(_, state) => {
                let word = if state == UnitState::Verified { "verified" } else { "pending" };
                format!("+{} {}", chip.count, tr(cx, TABLE, word)).into()
            }
            _ => SharedString::default(),
        };
        let mono = matches!(chip.node, Node::Unit(_));
        let retries: Option<SharedString> = match chip.node {
            Node::Unit(u) if chip.rect.w >= CHIP_MIN_W && run.units[u].attempts > 1 => Some(format!("×{}", run.units[u].attempts).into()),
            _ => None,
        };
        h_flex()
            .absolute()
            .left(px(chip.rect.x))
            .top(px(chip.rect.y))
            .w(px(chip.rect.w))
            .h(px(chip.rect.h))
            .gap(px(4.))
            .px(px(6.))
            .rounded(px(3.))
            .border_1()
            .border_color(border)
            .bg(look.fill)
            .when_some(shadow, |this, color| this.shadow(glow(color, 9.)))
            .child(state_mark(chip.state, 13., t.unit(chip.state), breath))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
                    .text_color(look.text)
                    .child(label),
            )
            .when_some(retries, |this, text| this.child(div().flex_none().text_size(px(12.)).text_color(t.muted).child(text)))
            .into_any_element()
    }

    fn tooltip_body(&self, node: Node, run: &Run, t: &Tokens, cx: &App) -> Option<AnyElement> {
        let heading = |text: SharedString, mono: bool| {
            div()
                .text_size(px(13.))
                .text_color(t.text)
                .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
                .child(text)
        };
        let note = |text: SharedString| div().text_size(px(12.)).text_color(t.muted).child(text);
        let state_line = |state: UnitState, extra: Option<SharedString>| {
            h_flex()
                .gap(px(5.))
                .text_size(px(12.))
                .child(state_mark(state, 12., t.unit(state), 0.))
                .child(div().text_color(t.unit(state)).child(tr(cx, TABLE, state.label())))
                .when_some(extra, |this, text| this.child(div().text_color(t.muted).child(text)))
        };
        let body = v_flex().gap(px(4.));
        let body = match node {
            Node::Core => {
                let stage = current_stage(run).map(|s| fmt_a(tr(cx, TABLE, "Stage: {a}"), stage_label(cx, &s.name)));
                body.child(heading("Chronos".into(), false))
                    .child(div().text_size(px(12.)).text_color(t.status(run.status)).child(tr(cx, TABLE, run.status.label())))
                    .when_some(stage, |this, line| this.child(note(line.into())))
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
                let attempts = (unit.attempts > 0).then(|| fmt_a(tr(cx, TABLE, "Attempt {a}"), unit.attempts).into());
                let module = run.modules.get(unit.module).map(|m| fmt_a(tr(cx, TABLE, "Module {a}"), m.id.as_ref()));
                body.child(heading(unit.id.clone(), true))
                    .when(unit.title != unit.id, |this| this.child(note(unit.title.clone())))
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

    fn tooltip_view(&self, node: Node, run: &Run, t: &Tokens, scroll: f32, cx: &App) -> Option<AnyElement> {
        let body = self.tooltip_body(node, run, t, cx)?;
        let core = self.graph.core(self.view_h);
        let at = match node {
            Node::Core => Pt { x: core.x + self.graph.core_r + 10.0, y: core.y - 24.0 },
            Node::Module(_) => {
                let r = self.graph.rect_of(node)?;
                Pt { x: r.x, y: r.bottom() + 6.0 - scroll }
            }
            _ => {
                let r = self.graph.rect_of(node)?;
                Pt { x: r.right() + 8.0, y: r.y - scroll - 2.0 }
            }
        };
        Some(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(at.x), px(at.y)))
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .w(px(260.))
                            .p(px(10.))
                            .rounded(px(4.))
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
        let (width, view_h) = (f32::from(panel.width), f32::from(panel.height));
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            return div().into_any_element();
        };
        let selection = state.selection.clone();
        let moving = animates(run, state.reduce_motion);
        let graph = Rc::new(layout(run, width, view_h, &selection));
        self.view_h = view_h;
        self.graph = graph.clone();

        let mut scroll = self.scroll_y();
        if self.seen_session.as_ref() != Some(&run.session) {
            self.seen_session = Some(run.session.clone());
            self.seen_selection = selection.clone();
            scroll = graph.most_urgent().map_or(0.0, |r| reveal(r, 0.0, view_h, graph.height));
            self.scroll.set_offset(point(px(0.), px(-scroll)));
        } else if selection != self.seen_selection {
            self.seen_selection = selection.clone();
            if let Some(rect) = selected_node(&selection).and_then(|n| graph.rect_of(n)) {
                scroll = reveal(rect, scroll, view_h, graph.height);
                self.scroll.set_offset(point(px(0.), px(-scroll)));
            }
        }

        if self.inside {
            let p = self.panel_point(window.mouse_position());
            self.hover = graph.hit(p, view_h, scroll);
        }
        let hover = self.hover.filter(|&node| node == Node::Core || graph.rect_of(node).is_some());

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
        let top = scroll - OVERSCAN;
        let bottom = scroll + view_h + OVERSCAN;
        let visible = |r: &Rect| r.bottom() >= top && r.y <= bottom;
        let cards: Vec<AnyElement> = graph
            .modules
            .iter()
            .filter(|m| visible(&m.rect))
            .map(|m| self.card_view(m, run, &t, dark, breath, cx))
            .collect();
        let chips: Vec<AnyElement> = graph
            .chips
            .iter()
            .filter(|c| visible(&c.rect))
            .map(|c| self.chip_view(c, run, &t, dark, breath, cx))
            .collect();

        let backdrop = Backdrop {
            graph: graph.clone(),
            t,
            dark,
            time,
            selected: selected_node(&selection),
            hovered: hover,
            orbit_in: orbit_visibility(orbit_p),
            turn: hourglass_turn(turn_p),
        };
        let handle = self.scroll.clone();
        let this = cx.entity();
        let tooltip = hover.and_then(|node| self.tooltip_view(node, run, &t, scroll, cx));
        let status = run.status;
        let core = graph.core(view_h);
        let label_w = (graph.core_r * 2.0 + 40.0).max(92.0);
        let stage = current_stage(run).map(|s| stage_label(cx, &s.name));
        let sub: SharedString = match (status, stage) {
            (RunStatus::Active, Some(stage)) => stage,
            _ => tr(cx, TABLE, status.label()).into(),
        };
        let wide = width >= 420.0;

        div()
            .id("mesh-graph")
            .size_full()
            .relative()
            .overflow_hidden()
            .when(hover.is_some(), |this| this.cursor_pointer())
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.pinned {
                    return;
                }
                let p = this.panel_point(event.position);
                let node = this.graph.hit(p, this.view_h, this.scroll_y());
                this.set_hover(node, cx);
            }))
            .on_hover(cx.listener(|this, inside: &bool, _, cx| {
                this.inside = *inside && !this.pinned;
                if !*inside && !this.pinned {
                    this.set_hover(None, cx);
                }
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| this.clicked(event.position(), cx)))
            .child(
                canvas(
                    move |bounds, _, cx| this.update(cx, |this, _| this.origin = bounds.origin),
                    move |bounds, _, window, _| backdrop.paint(bounds, (-f32::from(handle.offset().y)).max(0.0), window),
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("mesh-scroll")
                    .absolute()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().relative().w_full().h(px(graph.height)).children(cards).children(chips)),
            )
            .when(scroll > 1.0, |this| this.child(edge_fade(t, true)))
            .when(scroll + view_h + 1.0 < graph.height, |this| this.child(edge_fade(t, false)))
            .when(wide, |this| {
                this.child(
                    v_flex()
                        .absolute()
                        .left(px(core.x - label_w / 2.0))
                        .top(px(core.y + graph.core_r + 22.0))
                        .w(px(label_w))
                        .items_center()
                        .child(div().text_size(px(12.)).text_color(t.gold).child("C H R O N O S"))
                        .child(div().text_size(px(12.)).text_color(t.status(status)).child(sub)),
                )
            })
            .vertical_scrollbar(&self.scroll)
            .when_some(tooltip, |this, tip| this.child(tip))
            .into_any_element()
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
    let order = [UnitState::Running, UnitState::Built, UnitState::Failed, UnitState::Verified, UnitState::Pending];
    h_flex()
        .flex_wrap()
        .flex_none()
        .gap(px(14.))
        .px(px(14.))
        .py(px(8.))
        .border_b_1()
        .border_color(t.border)
        .children(order.into_iter().map(|state| {
            let n = run.count(state);
            h_flex()
                .gap(px(5.))
                .text_size(px(12.))
                .text_color(if n > 0 { t.text } else { t.muted })
                .child(state_mark(state, 12., t.unit(state), 0.))
                .child(tr(cx, TABLE, state.label()))
                .child(div().text_color(t.muted).child(n.to_string()))
        }))
}

fn summary_fact(t: &Tokens, label: &'static str, value: SharedString, color: Hsla) -> impl IntoElement {
    v_flex()
        .child(div().text_size(px(12.)).text_color(t.muted).child(label))
        .child(div().text_size(px(14.)).text_color(color).child(value))
}

/// What a finished run produced. Units that are not verified are named, never counted as done.
fn summary(run: &Run, t: &Tokens, cx: &App) -> impl IntoElement {
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
        .m(px(12.))
        .px(px(16.))
        .py(px(12.))
        .gap(px(8.))
        .child(
            h_flex()
                .gap(px(8.))
                .child(state_mark(
                    if complete { UnitState::Verified } else { UnitState::Built },
                    15.,
                    if complete { t.ok } else { t.review },
                    0.,
                ))
                .child(div().text_size(px(15.)).text_color(t.text).child(headline)),
        )
        .child(ornament::divider(t, 0.0))
        .child(
            h_flex()
                .flex_wrap()
                .gap(px(20.))
                .child(summary_fact(t, tr(cx, TABLE, "Full test"), test.into(), test_color))
                .child(summary_fact(t, tr(cx, TABLE, "Cost"), cost, t.text))
                .child(summary_fact(t, tr(cx, TABLE, "Time"), time, t.text)),
        )
        .when_some(report, |this, text| this.child(div().text_size(px(12.)).text_color(t.muted).line_clamp(2).child(text)))
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
                    .gap(px(3.))
                    .child(div().text_size(px(12.)).text_color(t.muted).child(tr(cx, TABLE, "Not verified")))
                    .children(open.iter().take(shown).map(|u| {
                        h_flex()
                            .gap(px(5.))
                            .text_size(px(12.))
                            .child(state_mark(u.state, 12., t.unit(u.state), 0.))
                            .child(div().text_color(t.text).child(u.id.clone()))
                            .child(div().text_color(t.muted).child(tr(cx, TABLE, u.state.label())))
                    }))
                    .when(open.len() > shown, |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.muted)
                                .child(fmt_a(tr(cx, TABLE, "and {a} more"), open.len() - shown)),
                        )
                    }),
            )
        })
}

/// The portrait, large and dim, above the words of an empty state.
fn empty_state(t: &Tokens, dark: bool, height: f32) -> Div {
    let side = (height * 0.5).clamp(160.0, 340.0);
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .bg(t.canvas)
        .child(div().mb(px(-side * 0.1)).child(ornament::portrait_backdrop(t, side, dark)))
}

impl Render for Mesh {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let dark = cx.theme().is_dark();
        let height = f32::from(window.viewport_size().height) - 220.0;
        let state_entity = self.state.clone();
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            return empty_state(&t, dark, height)
                .child(div().text_size(px(15.)).text_color(t.text).child(tr(cx, TABLE, "No wish to show")))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.muted)
                        .text_center()
                        .px_4()
                        .child(tr(cx, TABLE, "Make a wish and its work appears here as a live graph.")),
                )
                .child(
                    div().mt_2().child(
                        Button::new("mesh-make-wish")
                            .primary()
                            .label(tr(cx, TABLE, "Make a wish"))
                            .on_click(move |_, _, cx| state_entity.update(cx, |s, cx| s.set_composing(true, cx))),
                    ),
                )
                .into_any_element();
        };
        if run.modules.is_empty() {
            return empty_state(&t, dark, height)
                .child(div().text_size(px(15.)).text_color(t.text).child(tr(cx, TABLE, planning_line(run))))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.muted)
                        .text_center()
                        .px_4()
                        .child(tr(cx, TABLE, "Modules and units appear here once the plan is ready.")),
                )
                .into_any_element();
        }
        if animates(run, state.reduce_motion) || self.ceremonies.playing(&run.session, state.reduce_motion, Instant::now()) {
            window.request_animation_frame();
        }
        let finished = (run.status == RunStatus::Done).then(|| summary(run, &t, cx));
        let legend = legend(run, &t, cx);
        let this = cx.entity();
        v_flex()
            .size_full()
            .bg(t.canvas)
            .child(legend)
            .when_some(finished, |this, card| this.child(card))
            .child(
                div().flex_1().min_h_0().relative().child(container_query(move |panel, window, cx| {
                    this.update(cx, |this, cx| this.graph_view(panel, window, cx))
                })),
            )
            .into_any_element()
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

#[cfg(test)]
mod tests {
    use super::super::demo::sample_run;
    use super::*;
    // The kit's glob import brings gpui's `test` macro, which would otherwise shadow the standard one.
    use std::prelude::v1::test;
    use UnitState::*;

    const WIDTHS: [f32; 4] = [300.0, 640.0, 1120.0, 1900.0];

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
                let wire = graph.chip_wire(chip, 0.0);
                let card = graph.modules[chip.module].rect;
                assert_eq!(wire.at(0.0), Pt { x: card.right(), y: card.cy() });
                let end = wire.at(1.0);
                assert!((end.x - chip.rect.x).abs() < 0.01 && (end.y - chip.rect.cy()).abs() < 0.01);
            }
        }
    }

    #[test]
    fn scrolling_moves_wires_with_the_content() {
        let run = sample_run();
        let graph = layout(&run, 640.0, 300.0, &Selection::None);
        let chip = &graph.chips[3];
        assert_eq!(graph.chip_wire(chip, 40.0).to.y, chip.rect.cy() - 40.0);
        let m = &graph.modules[2];
        assert_eq!(graph.core_wire(m, 300.0, 40.0).from.y, 150.0);
        assert_eq!(graph.core_wire(m, 300.0, 40.0).to.y, m.rect.cy() - 40.0);
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
        for chip in &graph.chips {
            let p = Pt { x: chip.rect.x + 5.0, y: chip.rect.cy() };
            assert_eq!(graph.hit(p, 600.0, 0.0), Some(chip.node));
        }
        for m in &graph.modules {
            let p = Pt { x: m.rect.x + 5.0, y: m.rect.cy() };
            assert_eq!(graph.hit(p, 600.0, 0.0), Some(Node::Module(m.index)));
        }
        let core = graph.core(600.0);
        assert_eq!(graph.hit(core, 600.0, 0.0), Some(Node::Core));
        let gap = Pt { x: graph.modules[0].rect.x - 6.0, y: graph.modules[0].rect.cy() };
        assert_eq!(graph.hit(gap, 600.0, 0.0), None);
    }

    #[test]
    fn hit_testing_follows_the_scroll_but_the_core_does_not_move() {
        let run = large_run(sample_run(), 60, 900);
        let graph = layout(&run, 640.0, 600.0, &Selection::None);
        let m = &graph.modules[30];
        let scroll = m.rect.y - 100.0;
        let p = Pt { x: m.rect.x + 5.0, y: m.rect.cy() - scroll };
        assert_eq!(graph.hit(p, 600.0, scroll), Some(Node::Module(30)));
        assert_eq!(graph.hit(graph.core(600.0), 600.0, scroll), Some(Node::Core));
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
    fn reveal_only_scrolls_when_needed() {
        let rect = Rect { x: 0.0, y: 500.0, w: 10.0, h: 24.0 };
        assert_eq!(reveal(rect, 400.0, 300.0, 2000.0), 400.0);
        assert_eq!(reveal(rect, 0.0, 300.0, 2000.0), 512.0 - 150.0);
        let high = Rect { x: 0.0, y: 40.0, w: 10.0, h: 24.0 };
        assert_eq!(reveal(high, 900.0, 300.0, 2000.0), 0.0);
        let low = Rect { x: 0.0, y: 1990.0, w: 10.0, h: 10.0 };
        assert_eq!(reveal(low, 0.0, 300.0, 2000.0), 1700.0);
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
