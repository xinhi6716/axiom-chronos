//! The checklist: what the wish has to do, what is finished and what is happening now.
use super::i18n::{tr, Table};
use super::model::*;
use super::theme::{Tokened as _, Tokens};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, v_virtual_list, ElementExt as _, Icon, IconName, Selectable as _, Sizable as _,
    VirtualListScrollHandle,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

const T: Table = &[
    ("All", "全部"),
    ("Active", "進行中"),
    ("Needs review", "待審查"),
    ("Partly verified", "部分驗收"),
    ("Modules not known yet", "尚未對應模組"),
    ("No units yet", "尚無單元"),
    ("units", "單元"),
    ("Goals", "目標"),
    ("verified", "已驗收"),
    ("Not linked to a goal", "未對應目標"),
    ("1 attempt", "嘗試 1 次"),
    ("{n} attempts", "嘗試 {n} 次"),
    ("Astra is planning…", "Astra 正在規劃…"),
    ("The critic is reviewing the plan…", "第二審正在檢查計畫…"),
    ("The architect is splitting the plan into modules…", "正在把計畫切成模組…"),
    ("Getting the work ready…", "正在準備工作…"),
    ("Planning stopped before a plan was made.", "規劃中斷，尚未產生計畫。"),
    ("Splitting the plan into modules stopped.", "切分模組時中斷。"),
    ("This wish needs you before it can go on.", "這個願望需要你處理後才能繼續。"),
    ("This wish ended before the plan was split into modules.", "這個願望在切分模組前就結束了。"),
    ("Waiting on a background batch.", "正在等待背景批次。"),
    ("This wish needs you before it can plan.", "這個願望需要你處理後才能規劃。"),
    ("This wish ended without a plan.", "這個願望結束時沒有產生計畫。"),
    ("Make a wish and its goals will be listed here.", "許願後，目標會列在這裡。"),
    ("No units are running.", "目前沒有進行中的單元。"),
    ("No units need review.", "沒有待審查的單元。"),
    ("No units have failed.", "沒有失敗的單元。"),
    ("There is nothing in this plan yet.", "這份計畫還沒有內容。"),
    ("Show all", "顯示全部"),
    ("Back to active work", "回到進行中的項目"),
];

const MAX_TITLE_LINES: usize = 2;
const RIGHT_PAD: f32 = 12.0;
const FRACTION_W: f32 = 44.0;
const META_LINE: f32 = 15.0;
const SECTION_H: f32 = 30.0;
const FALLBACK_WIDTH: f32 = 320.0;

/// Where a row's parts sit, and how its title is set.
struct Tier {
    chevron: f32,
    icon: f32,
    size: f32,
    text: f32,
    font: f32,
    line: f32,
    pad: f32,
}

const GOAL: Tier = Tier { chevron: 4.0, icon: 20.0, size: 16.0, text: 42.0, font: 13.0, line: 18.0, pad: 8.0 };
const MODULE: Tier = Tier { chevron: 18.0, icon: 35.0, size: 14.0, text: 55.0, font: 12.5, line: 17.0, pad: 6.0 };
const UNIT: Tier = Tier { chevron: 0.0, icon: 42.0, size: 13.0, text: 61.0, font: 12.0, line: 16.0, pad: 5.0 };

/// How far an item has come. Goals and modules derive it from their units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    Unknown,
    Pending,
    Partial,
    Running,
    Review,
    Failed,
    Done,
}

impl Progress {
    fn of_unit(state: UnitState) -> Self {
        match state {
            UnitState::Pending => Self::Pending,
            UnitState::Running => Self::Running,
            UnitState::Built => Self::Review,
            UnitState::Verified => Self::Done,
            UnitState::Failed => Self::Failed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Modules not known yet",
            Self::Pending => UnitState::Pending.label(),
            Self::Partial => "Partly verified",
            Self::Running => UnitState::Running.label(),
            Self::Review => UnitState::Built.label(),
            Self::Failed => UnitState::Failed.label(),
            Self::Done => UnitState::Verified.label(),
        }
    }
}

/// Unit counts by state. A unit that cannot be found counts as unknown, so it can never read as done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    verified: usize,
    review: usize,
    running: usize,
    failed: usize,
    pending: usize,
    unknown: usize,
}

impl Tally {
    fn add(&mut self, state: Option<UnitState>) {
        match state {
            Some(UnitState::Verified) => self.verified += 1,
            Some(UnitState::Built) => self.review += 1,
            Some(UnitState::Running) => self.running += 1,
            Some(UnitState::Failed) => self.failed += 1,
            Some(UnitState::Pending) => self.pending += 1,
            None => self.unknown += 1,
        }
    }

    fn of(run: &Run, units: &[usize]) -> Self {
        let mut tally = Self::default();
        for &unit in units {
            tally.add(run.units.get(unit).map(|u| u.state));
        }
        tally
    }

    fn of_run(run: &Run) -> Self {
        let mut tally = Self::default();
        for unit in &run.units {
            tally.add(Some(unit.state));
        }
        tally
    }

    fn merge(&mut self, other: Self) {
        self.verified += other.verified;
        self.review += other.review;
        self.running += other.running;
        self.failed += other.failed;
        self.pending += other.pending;
        self.unknown += other.unknown;
    }

    fn total(self) -> usize {
        self.verified + self.review + self.running + self.failed + self.pending + self.unknown
    }

    fn progress(self) -> Progress {
        if self.failed > 0 {
            Progress::Failed
        } else if self.running > 0 {
            Progress::Running
        } else if self.review > 0 {
            Progress::Review
        } else if self.total() > 0 && self.verified == self.total() {
            Progress::Done
        } else if self.verified > 0 {
            Progress::Partial
        } else {
            Progress::Pending
        }
    }

    fn needs_attention(self) -> bool {
        self.running + self.failed + self.review > 0
    }
}

fn module_tallies(run: &Run) -> Vec<Tally> {
    run.modules.iter().map(|m| Tally::of(run, &m.units)).collect()
}

/// A goal's counts and state. With no module it can name, its state is neutral.
fn goal_summary(goal: &GoalItem, tallies: &[Tally]) -> (Tally, Progress) {
    let mut tally = Tally::default();
    let mut known = 0;
    for (i, &module) in goal.modules.iter().enumerate() {
        if goal.modules[..i].contains(&module) {
            continue;
        }
        match tallies.get(module) {
            Some(counts) => {
                known += 1;
                tally.merge(*counts);
            }
            None => tally.unknown += 1,
        }
    }
    (tally, if known == 0 { Progress::Unknown } else { tally.progress() })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Active,
    Review,
    Failed,
}

impl Filter {
    const ALL: [Self; 4] = [Self::All, Self::Active, Self::Review, Self::Failed];

    fn matches(self, state: UnitState) -> bool {
        match self {
            Self::All => true,
            Self::Active => state == UnitState::Running,
            Self::Review => state == UnitState::Built,
            Self::Failed => state == UnitState::Failed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Review => "Needs review",
            Self::Failed => "Failed",
        }
    }

    fn count(self, tally: Tally) -> usize {
        match self {
            Self::All => tally.total(),
            Self::Active => tally.running,
            Self::Review => tally.review,
            Self::Failed => tally.failed,
        }
    }

    fn empty_text(self) -> &'static str {
        match self {
            Self::All => "There is nothing in this plan yet.",
            Self::Active => "No units are running.",
            Self::Review => "No units need review.",
            Self::Failed => "No units have failed.",
        }
    }
}

/// Which goals and modules the user opened or closed. Everything else follows its default.
#[derive(Debug, Default)]
struct Expansion {
    goals: HashMap<usize, bool>,
    modules: HashMap<usize, bool>,
}

impl Expansion {
    fn set(&mut self, key: RowKey, open: bool) {
        match key {
            RowKey::Goal(goal) => {
                self.goals.insert(goal, open);
            }
            RowKey::Module { module, .. } => {
                self.modules.insert(module, open);
            }
            RowKey::Section | RowKey::Unit { .. } => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKey {
    Goal(usize),
    Section,
    Module { goal: Option<usize>, module: usize },
    Unit { goal: Option<usize>, module: Option<usize>, unit: usize },
}

impl RowKey {
    fn parent(self) -> Option<Self> {
        match self {
            Self::Module { goal: Some(goal), .. } => Some(Self::Goal(goal)),
            Self::Unit { goal, module: Some(module), .. } => Some(Self::Module { goal, module }),
            _ => None,
        }
    }

    fn id(self) -> ElementId {
        let code = match self {
            Self::Goal(goal) => goal as u64,
            Self::Section => 0,
            Self::Module { goal, module } => (slot(goal) << 24) | module as u64,
            Self::Unit { goal, module, unit } => (slot(goal) << 48) | (slot(module) << 24) | unit as u64,
        };
        ElementId::NamedInteger(
            match self {
                Self::Goal(_) => "todo-goal",
                Self::Section => "todo-section",
                Self::Module { .. } => "todo-module",
                Self::Unit { .. } => "todo-unit",
            }
            .into(),
            code,
        )
    }

    fn tier(self) -> &'static Tier {
        match self {
            Self::Goal(_) | Self::Section => &GOAL,
            Self::Module { .. } => &MODULE,
            Self::Unit { .. } => &UNIT,
        }
    }

    fn title_width(self, list_width: f32) -> f32 {
        let trailing = if matches!(self, Self::Module { .. }) { FRACTION_W } else { 0.0 };
        (list_width - self.tier().text - RIGHT_PAD - trailing).max(40.0)
    }
}

fn slot(index: Option<usize>) -> u64 {
    index.map_or(0, |i| i as u64 + 1)
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    key: RowKey,
    /// For a goal, its number and title, the number taking the first `badge_len` bytes.
    title: SharedString,
    badge_len: usize,
    progress: Progress,
    tally: Tally,
    attempts: u32,
    /// `None` when there is nothing under this row to open.
    open: Option<bool>,
    selected: bool,
    lines: usize,
    clipped: bool,
}

impl Row {
    fn new(key: RowKey, title: SharedString, progress: Progress, tally: Tally) -> Self {
        Self {
            key,
            title,
            badge_len: 0,
            progress,
            tally,
            attempts: 0,
            open: None,
            selected: false,
            lines: 1,
            clipped: false,
        }
    }

    fn height(&self) -> f32 {
        let lines = self.lines.clamp(1, MAX_TITLE_LINES) as f32;
        let tier = self.key.tier();
        match self.key {
            RowKey::Goal(_) => tier.pad * 2.0 + tier.line * lines + META_LINE,
            RowKey::Section => SECTION_H,
            RowKey::Module { .. } => tier.pad * 2.0 + tier.line * lines,
            RowKey::Unit { .. } => tier.pad * 2.0 + tier.line * lines + META_LINE,
        }
    }
}

/// Flattens the plan into the rows to draw. Every module and unit appears somewhere, and a filter
/// never hides the item that is selected.
fn build_rows(run: &Run, expansion: &Expansion, filter: Filter, selection: &Selection) -> Vec<Row> {
    let tallies = module_tallies(run);
    let filtering = filter != Filter::All;
    let unit_shown = |unit: usize| {
        *selection == Selection::Unit(unit) || run.units.get(unit).is_some_and(|u| filter.matches(u.state))
    };
    let module_shown = |module: usize| {
        !filtering || *selection == Selection::Module(module) || run.modules[module].units.iter().any(|&u| unit_shown(u))
    };
    let push_unit = |rows: &mut Vec<Row>, goal: Option<usize>, module: Option<usize>, unit: usize| {
        let item = &run.units[unit];
        let mut row = Row::new(
            RowKey::Unit { goal, module, unit },
            item.title.clone(),
            Progress::of_unit(item.state),
            Tally::default(),
        );
        row.attempts = item.attempts;
        row.selected = *selection == Selection::Unit(unit);
        rows.push(row);
    };
    let push_module = |rows: &mut Vec<Row>, goal: Option<usize>, module: usize| {
        let item = &run.modules[module];
        let tally = tallies[module];
        let open = filtering
            || expansion.modules.get(&module).copied().unwrap_or_else(|| tally.needs_attention());
        let title = if item.purpose.is_empty() { item.id.clone() } else { item.purpose.clone() };
        let mut row = Row::new(RowKey::Module { goal, module }, title, tally.progress(), tally);
        row.open = (!filtering && !item.units.is_empty()).then_some(open);
        row.selected = *selection == Selection::Module(module);
        rows.push(row);
        if open {
            for &unit in item.units.iter().filter(|&&u| u < run.units.len() && unit_shown(u)) {
                push_unit(rows, goal, Some(module), unit);
            }
        }
    };

    let mut rows = Vec::with_capacity(run.goals.len() + run.modules.len() + run.units.len());
    let mut linked = vec![false; run.modules.len()];
    for (g, goal) in run.goals.iter().enumerate() {
        let mut modules: Vec<usize> = Vec::with_capacity(goal.modules.len());
        for &module in &goal.modules {
            if module < run.modules.len() && !modules.contains(&module) {
                linked[module] = true;
                modules.push(module);
            }
        }
        let shown: Vec<usize> = modules.iter().copied().filter(|&m| module_shown(m)).collect();
        if filtering && shown.is_empty() {
            continue;
        }
        let (tally, progress) = goal_summary(goal, &tallies);
        let open = filtering
            || expansion.goals.get(&g).copied().unwrap_or(progress != Progress::Done);
        let mut row = Row::new(RowKey::Goal(g), format!("{} {}", goal.id, goal.title).into(), progress, tally);
        row.badge_len = goal.id.len();
        row.open = (!filtering && !shown.is_empty()).then_some(open);
        rows.push(row);
        if open {
            for module in shown {
                push_module(&mut rows, Some(g), module);
            }
        }
    }

    let mut listed = vec![false; run.units.len()];
    for module in &run.modules {
        for &unit in &module.units {
            if let Some(seen) = listed.get_mut(unit) {
                *seen = true;
            }
        }
    }
    let loose: Vec<usize> = (0..run.modules.len()).filter(|&m| !linked[m] && module_shown(m)).collect();
    let orphans: Vec<usize> = (0..run.units.len()).filter(|&u| !listed[u] && unit_shown(u)).collect();
    if !run.goals.is_empty() && !(loose.is_empty() && orphans.is_empty()) {
        rows.push(Row::new(RowKey::Section, SharedString::default(), Progress::Unknown, Tally::default()));
    }
    for module in loose {
        push_module(&mut rows, None, module);
    }
    for unit in orphans {
        push_unit(&mut rows, None, None, unit);
    }
    rows
}

/// A goal has no selection of its own, so choosing it selects the module where it most needs a look.
fn goal_lead(run: &Run, goal: usize) -> Option<usize> {
    let rank = |progress| match progress {
        Progress::Failed => 0,
        Progress::Running => 1,
        Progress::Review => 2,
        Progress::Partial => 3,
        Progress::Pending | Progress::Unknown => 4,
        Progress::Done => 5,
    };
    run.goals
        .get(goal)?
        .modules
        .iter()
        .copied()
        .filter(|&m| m < run.modules.len())
        .min_by_key(|&m| rank(Tally::of(run, &run.modules[m].units).progress()))
}

fn selection_of(run: &Run, key: RowKey) -> Selection {
    match key {
        RowKey::Goal(goal) => goal_lead(run, goal).map_or(Selection::None, Selection::Module),
        RowKey::Section => Selection::None,
        RowKey::Module { module, .. } => Selection::Module(module),
        RowKey::Unit { unit, .. } => Selection::Unit(unit),
    }
}

/// Opens whatever hides `selection`, so a selection made elsewhere can be shown here.
fn expand_for(run: &Run, expansion: &mut Expansion, selection: &Selection) {
    let module = match *selection {
        Selection::None => return,
        Selection::Module(module) => module,
        Selection::Unit(unit) => {
            let declared = run.units.get(unit).map(|u| u.module);
            let found = declared
                .filter(|&m| run.modules.get(m).is_some_and(|x| x.units.contains(&unit)))
                .or_else(|| run.modules.iter().position(|x| x.units.contains(&unit)));
            let Some(module) = found else { return };
            expansion.modules.insert(module, true);
            module
        }
    };
    if let Some(goal) = run.goals.iter().position(|g| g.modules.contains(&module)) {
        expansion.goals.insert(goal, true);
    }
}

/// The row the list keeps in view: running work, then failures, then what waits for review.
fn follow_target(rows: &[Row]) -> Option<usize> {
    let rank = |row: &Row| match row.progress {
        Progress::Running => Some(0),
        Progress::Failed => Some(1),
        Progress::Review => Some(2),
        _ => None,
    };
    let depth = |row: &Row| match row.key {
        RowKey::Unit { .. } => 0,
        RowKey::Module { .. } => 1,
        _ => 2,
    };
    rows.iter()
        .enumerate()
        .filter_map(|(i, row)| rank(row).map(|r| ((r, depth(row), i), i)))
        .min_by_key(|(order, _)| *order)
        .map(|(_, i)| i)
}

fn row_top(rows: &[Row], index: usize) -> f32 {
    rows[..index].iter().map(Row::height).sum()
}

fn content_height(rows: &[Row]) -> f32 {
    rows.iter().map(Row::height).sum()
}

fn is_visible(top: f32, height: f32, scroll: f32, viewport: f32) -> bool {
    top >= scroll - 0.5 && top + height <= scroll + viewport + 0.5
}

fn centered_scroll(top: f32, height: f32, viewport: f32, content: f32) -> f32 {
    (top + height / 2.0 - viewport / 2.0).clamp(0.0, (content - viewport).max(0.0))
}

fn nearest_scroll(top: f32, height: f32, scroll: f32, viewport: f32) -> f32 {
    if top < scroll {
        top
    } else if top + height > scroll + viewport {
        top + height - viewport
    } else {
        scroll
    }
}

/// Where to scroll so that what was on top of the list stays there after the rows change.
fn anchored_scroll(old: &[Row], new: &[Row], scroll: f32) -> f32 {
    let mut top = 0.0;
    let mut first = None;
    for (i, row) in old.iter().enumerate() {
        if top + row.height() > scroll {
            first = Some((i, scroll - top));
            break;
        }
        top += row.height();
    }
    let Some((start, offset)) = first else { return scroll };
    for (distance, row) in old[start..].iter().enumerate() {
        if let Some(found) = new.iter().position(|n| n.key == row.key) {
            let inside = if distance == 0 { offset } else { 0.0 };
            return row_top(new, found) + inside;
        }
    }
    scroll
}

#[derive(Clone, Debug, PartialEq)]
struct PlanNote {
    text: &'static str,
    detail: Option<SharedString>,
    busy: bool,
    failed: bool,
}

/// What to say while there is no plan yet, from the stages and the run's status.
fn plan_note(run: &Run) -> PlanNote {
    let say = |text: &'static str| PlanNote { text, detail: None, busy: false, failed: false };
    let planned = !run.goals.is_empty();
    if let Some(stage) = run.stages.iter().find(|s| s.phase == Phase::Failed) {
        let text = if planned { "Splitting the plan into modules stopped." } else { "Planning stopped before a plan was made." };
        return PlanNote { detail: stage.note.clone(), failed: true, ..say(text) };
    }
    match run.status {
        RunStatus::Waiting => return say("Waiting on a background batch."),
        RunStatus::Blocked => {
            return say(if planned { "This wish needs you before it can go on." } else { "This wish needs you before it can plan." })
        }
        RunStatus::Done => {
            return say(if planned { "This wish ended before the plan was split into modules." } else { "This wish ended without a plan." })
        }
        RunStatus::Active => {}
    }
    match run.stages.iter().find(|s| s.phase == Phase::Running) {
        Some(stage) => PlanNote {
            detail: stage.note.clone(),
            busy: true,
            ..say(match stage.name.as_ref() {
                "Astra" => "Astra is planning…",
                "Critic" => "The critic is reviewing the plan…",
                "Architect" => "The architect is splitting the plan into modules…",
                _ => "Getting the work ready…",
            })
        },
        None => say("Getting the work ready…"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rebuild {
    /// New data: stay put, and keep active work in view while following.
    Data,
    /// The list changed size: while following, start again from the top if the work still fits.
    Viewport,
    /// A different list: other rows, so the old position means nothing.
    Fresh,
    /// The user changed what is open: stay put and do not chase anything.
    Manual,
    /// The selection changed: bring it into view.
    Reveal,
}

fn progress_color(t: &Tokens, progress: Progress) -> Hsla {
    match progress {
        Progress::Unknown | Progress::Pending => t.idle,
        Progress::Partial => t.gold,
        Progress::Running => t.run,
        Progress::Review => t.review,
        Progress::Failed => t.fail,
        Progress::Done => t.ok,
    }
}

fn state_mark(progress: Progress, size: f32, live: bool, t: &Tokens) -> AnyElement {
    let color = progress_color(t, progress);
    let icon = |name: IconName| Icon::new(name).size(px(size)).text_color(color).into_any_element();
    let ring = |dashed: bool, dot: bool| {
        div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px((size - 2.0).max(8.0)))
                    .rounded_full()
                    .border(px(1.4))
                    .border_color(color)
                    .when(dashed, |this| this.border_dashed())
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(dot, |this| this.child(div().size(px(size * 0.34)).rounded_full().bg(color))),
            )
            .into_any_element()
    };
    match progress {
        Progress::Running if live => Spinner::new()
            .icon(IconName::LoaderCircle)
            .with_size(px(size))
            .color(color)
            .into_any_element(),
        Progress::Running => icon(IconName::LoaderCircle),
        Progress::Done => icon(IconName::CircleCheck),
        Progress::Failed => icon(IconName::CircleX),
        Progress::Review => icon(IconName::Eye),
        Progress::Pending => ring(false, false),
        Progress::Partial => ring(false, true),
        Progress::Unknown => ring(true, false),
    }
}

fn attempts_text(cx: &App, attempts: u32) -> String {
    if attempts == 1 {
        tr(cx, T, "1 attempt").to_owned()
    } else {
        tr(cx, T, "{n} attempts").replace("{n}", &attempts.to_string())
    }
}

fn fraction(tally: Tally) -> String {
    if tally.total() == 0 {
        "—".to_owned()
    } else {
        format!("{}/{}", tally.verified, tally.total())
    }
}

pub struct Todo {
    state: Entity<AppState>,
    focus: FocusHandle,
    scroll: VirtualListScrollHandle,
    filter: Filter,
    expansion: Expansion,
    rows: Rc<Vec<Row>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    tally: Tally,
    goals_done: usize,
    goals_total: usize,
    live: bool,
    cursor: Option<RowKey>,
    /// Whether the list keeps active work in view; off once the user has moved away from it.
    follow: bool,
    rebuild: Option<Rebuild>,
    session: Option<SharedString>,
    selection: Selection,
    /// The list's width and height at the last prepaint.
    viewport: Rc<Cell<(f32, f32)>>,
    measured: (f32, f32),
    /// The scroll position when the last frame was prepainted, to notice the user scrolling.
    painted_scroll: Rc<Cell<f32>>,
    forced_width: Option<f32>,
}

impl Todo {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, _, cx| {
            this.request(Rebuild::Data);
            cx.notify();
        })
        .detach();
        let mut this = Self {
            state,
            focus: cx.focus_handle().tab_stop(true),
            scroll: VirtualListScrollHandle::new(),
            filter: Filter::All,
            expansion: Expansion::default(),
            rows: Rc::default(),
            sizes: Rc::default(),
            tally: Tally::default(),
            goals_done: 0,
            goals_total: 0,
            live: false,
            cursor: None,
            follow: true,
            rebuild: Some(Rebuild::Data),
            session: None,
            selection: Selection::None,
            viewport: Rc::new(Cell::new((0.0, 0.0))),
            measured: (0.0, 0.0),
            painted_scroll: Rc::new(Cell::new(0.0)),
            forced_width: None,
        };
        this.apply_scene(cx);
        this
    }

    fn request(&mut self, rebuild: Rebuild) {
        self.rebuild = self.rebuild.max(Some(rebuild));
    }

    fn scroll_pos(&self) -> f32 {
        -f32::from(self.scroll.offset().y)
    }

    fn set_scroll(&self, position: f32) {
        self.scroll.set_offset(point(px(0.0), px(-position.max(0.0))));
        self.painted_scroll.set(position.max(0.0));
    }

    fn target_visible(&self, scroll: f32) -> Option<bool> {
        let index = follow_target(&self.rows)?;
        let viewport = self.viewport.get().1;
        Some(is_visible(row_top(&self.rows, index), self.rows[index].height(), scroll, viewport))
    }

    /// Brings the data, the rows and the scroll position up to date before drawing.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scroll = self.scroll_pos();
        if (scroll - self.painted_scroll.get()).abs() > 0.5 {
            self.painted_scroll.set(scroll);
            if let Some(visible) = self.target_visible(scroll) {
                self.follow = visible;
            }
        }

        let state = self.state.read(cx);
        let session = state.run().map(|r| r.session.clone());
        let selection = state.selection.clone();
        let live = state.run().is_some_and(Run::is_live) && !state.reduce_motion;
        if session != self.session {
            self.session = session;
            self.filter = Filter::All;
            self.expansion = Expansion::default();
            self.cursor = None;
            self.follow = true;
            self.set_scroll(0.0);
            self.request(Rebuild::Fresh);
        }
        if selection != self.selection {
            if let Some(run) = self.state.read(cx).run() {
                expand_for(run, &mut self.expansion, &selection);
            }
            self.selection = selection;
            self.request(Rebuild::Reveal);
        }
        self.live = live;

        if self.viewport.get() != self.measured {
            self.request(Rebuild::Viewport);
        }
        if let Some(rebuild) = self.rebuild.take() {
            self.rebuild_rows(rebuild, window, cx);
        }
    }

    fn rebuild_rows(&mut self, rebuild: Rebuild, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            self.rows = Rc::default();
            self.sizes = Rc::default();
            self.tally = Tally::default();
            self.goals_total = 0;
            self.goals_done = 0;
            return;
        };
        let mut rows = build_rows(run, &self.expansion, self.filter, &state.selection);
        self.measured = self.viewport.get();
        let width = if self.measured.0 > 0.0 { self.measured.0 } else { FALLBACK_WIDTH };
        measure(&mut rows, width, window);
        self.tally = Tally::of_run(run);
        let tallies = module_tallies(run);
        self.goals_total = run.goals.len();
        self.goals_done = run
            .goals
            .iter()
            .filter(|g| goal_summary(g, &tallies).1 == Progress::Done)
            .count();

        let viewport = self.measured.1;
        let mut scroll = self.scroll_pos();
        if matches!(rebuild, Rebuild::Data | Rebuild::Manual | Rebuild::Viewport) {
            scroll = anchored_scroll(&self.rows, &rows, scroll);
        }
        if let Some(key) = self.cursor {
            if !rows.iter().any(|r| r.key == key) {
                let was = self.rows.iter().position(|r| r.key == key).unwrap_or(0);
                self.cursor = rows.get(was.min(rows.len().saturating_sub(1))).map(|r| r.key);
            }
        }
        self.sizes = Rc::new(rows.iter().map(|r| size(px(0.0), px(r.height()))).collect());
        self.rows = Rc::new(rows);

        if viewport > 0.0 {
            let content = content_height(&self.rows);
            match rebuild {
                Rebuild::Reveal => {
                    if let Some(index) = self.rows.iter().position(|r| r.selected) {
                        let (top, height) = (row_top(&self.rows, index), self.rows[index].height());
                        if !is_visible(top, height, scroll, viewport) {
                            scroll = centered_scroll(top, height, viewport, content);
                        }
                        if let Some(visible) = self.target_visible(scroll) {
                            self.follow = visible;
                        }
                    }
                }
                Rebuild::Manual => {
                    if let Some(visible) = self.target_visible(scroll) {
                        self.follow = visible;
                    }
                }
                Rebuild::Data | Rebuild::Viewport | Rebuild::Fresh => {
                    if self.follow {
                        if let Some(index) = follow_target(&self.rows) {
                            let (top, height) = (row_top(&self.rows, index), self.rows[index].height());
                            if rebuild == Rebuild::Viewport && is_visible(top, height, 0.0, viewport) {
                                scroll = 0.0;
                            } else if !is_visible(top, height, scroll, viewport) {
                                scroll = centered_scroll(top, height, viewport, content);
                            }
                        }
                    }
                }
            }
        }
        self.set_scroll(scroll);
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.follow = true;
            self.set_scroll(0.0);
            self.request(Rebuild::Fresh);
            cx.notify();
        }
    }

    fn set_open(&mut self, key: RowKey, open: bool, cx: &mut Context<Self>) {
        self.expansion.set(key, open);
        self.request(Rebuild::Manual);
        cx.notify();
    }

    fn activate(&mut self, key: RowKey, cx: &mut Context<Self>) {
        self.cursor = Some(key);
        if let RowKey::Goal(_) = key {
            self.expansion.set(key, true);
            self.request(Rebuild::Manual);
        }
        let selection = match self.state.read(cx).run() {
            Some(run) => selection_of(run, key),
            None => return,
        };
        self.state.update(cx, |state, cx| state.select(selection, cx));
        cx.notify();
    }

    fn jump_to_active(&mut self, cx: &mut Context<Self>) {
        self.follow = true;
        self.request(Rebuild::Data);
        cx.notify();
    }

    fn cursor_index(&self) -> Option<usize> {
        let by_key = self.cursor.and_then(|key| self.rows.iter().position(|r| r.key == key));
        by_key
            .or_else(|| self.rows.iter().position(|r| r.selected))
            .or_else(|| self.rows.iter().position(|r| r.key != RowKey::Section))
    }

    fn step_cursor(&self, from: usize, down: bool) -> Option<usize> {
        let mut index = from;
        loop {
            index = if down { index.checked_add(1).filter(|&i| i < self.rows.len())? } else { index.checked_sub(1)? };
            if self.rows[index].key != RowKey::Section {
                return Some(index);
            }
        }
    }

    fn move_cursor(&mut self, index: usize, cx: &mut Context<Self>) {
        self.cursor = Some(self.rows[index].key);
        let viewport = self.viewport.get().1;
        let (top, height) = (row_top(&self.rows, index), self.rows[index].height());
        let scroll = nearest_scroll(top, height, self.scroll_pos(), viewport).max(0.0);
        self.set_scroll(scroll);
        if let Some(visible) = self.target_visible(scroll) {
            self.follow = visible;
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
            return;
        }
        let Some(current) = self.cursor_index() else { return };
        let row_count = self.rows.len();
        let page = |this: &Self, down: bool| {
            let viewport = this.viewport.get().1;
            let mut index = current;
            let mut travelled = 0.0;
            while travelled + this.rows[index].height() < viewport {
                travelled += this.rows[index].height();
                match this.step_cursor(index, down) {
                    Some(next) => index = next,
                    None => break,
                }
            }
            index
        };
        match event.keystroke.key.as_str() {
            "up" | "down" => {
                let down = event.keystroke.key == "down";
                let start = self.cursor.is_some() || self.rows[current].selected;
                let target = if start { self.step_cursor(current, down) } else { Some(current) };
                if let Some(index) = target {
                    self.move_cursor(index, cx);
                }
            }
            "pageup" => self.move_cursor(page(self, false), cx),
            "pagedown" => self.move_cursor(page(self, true), cx),
            "home" => {
                if let Some(index) = self.rows.iter().position(|r| r.key != RowKey::Section) {
                    self.move_cursor(index, cx);
                }
            }
            "end" => {
                if let Some(index) = (0..row_count).rev().find(|&i| self.rows[i].key != RowKey::Section) {
                    self.move_cursor(index, cx);
                }
            }
            "left" => {
                let row = &self.rows[current];
                let key = row.key;
                if row.open == Some(true) {
                    self.cursor = Some(key);
                    self.set_open(key, false, cx);
                } else if let Some(parent) = key.parent() {
                    if let Some(index) = self.rows.iter().position(|r| r.key == parent) {
                        self.move_cursor(index, cx);
                    }
                }
            }
            "right" => {
                let row = &self.rows[current];
                let key = row.key;
                match row.open {
                    Some(false) => {
                        self.cursor = Some(key);
                        self.set_open(key, true, cx);
                    }
                    Some(true) => {
                        if let Some(index) = self.step_cursor(current, true) {
                            self.move_cursor(index, cx);
                        }
                    }
                    None => {}
                }
            }
            "enter" | "space" => {
                let key = self.rows[current].key;
                self.activate(key, cx);
            }
            "escape" => {
                self.state.update(cx, |state, cx| state.select(Selection::None, cx));
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let row = self.rows[index].clone();
        let key = row.key;
        let height = row.height();
        if key == RowKey::Section {
            return div()
                .w_full()
                .h(px(height))
                .px(px(14.0))
                .pt(px(12.0))
                .text_size(px(11.0))
                .line_height(px(14.0))
                .text_color(t.muted)
                .child(tr(cx, T, "Not linked to a goal"))
                .into_any_element();
        }

        let tier = key.tier();
        let live = self.live;
        let done = row.progress == Progress::Done && !row.selected;
        let running_unit = row.progress == Progress::Running && matches!(key, RowKey::Unit { .. });
        let ring = self.focus.is_focused(window)
            && window.last_input_was_keyboard()
            && self.cursor_index() == Some(index);
        let title_width = key.title_width(self.measured.0.max(1.0));
        let title_color = if done { t.muted } else { t.text };
        let title: AnyElement = if row.badge_len == 0 {
            row.title.clone().into_any_element()
        } else {
            StyledText::new(row.title.clone())
                .with_highlights([(0..row.badge_len, HighlightStyle::color(t.gold))])
                .into_any_element()
        };
        let weight = match key {
            RowKey::Goal(_) => FontWeight::SEMIBOLD,
            RowKey::Module { .. } => FontWeight::MEDIUM,
            _ => FontWeight::NORMAL,
        };
        let meta: Option<AnyElement> = match key {
            RowKey::Goal(_) => {
                let word = tr(cx, T, row.progress.label());
                let figure = match row.progress {
                    Progress::Unknown => None,
                    _ if row.tally.total() == 0 => Some(tr(cx, T, "No units yet").to_owned()),
                    _ => Some(format!("{} {}", fraction(row.tally), tr(cx, T, "units"))),
                };
                Some(
                    h_flex()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .line_height(px(META_LINE))
                        .text_color(t.muted)
                        .child(div().text_color(meta_color(&t, row.progress)).child(word))
                        .when_some(figure, |this, figure| this.child(div().child("·")).child(figure))
                        .into_any_element(),
                )
            }
            RowKey::Unit { .. } => {
                let word = tr(cx, T, row.progress.label());
                let attempts = (row.attempts > 0).then(|| attempts_text(cx, row.attempts));
                Some(
                    h_flex()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .line_height(px(META_LINE))
                        .text_color(t.muted)
                        .child(div().text_color(meta_color(&t, row.progress)).child(word))
                        .when_some(attempts, |this, attempts| this.child(div().child("·")).child(attempts))
                        .into_any_element(),
                )
            }
            _ => None,
        };
        let tooltip_text = row.clipped.then(|| row.title.clone());
        let mark_top = tier.pad + (tier.line - tier.size) / 2.0;
        let accent = if row.selected {
            Some(t.gold)
        } else if running_unit {
            Some(t.run)
        } else {
            None
        };

        div()
            .w_full()
            .h(px(height))
            .px(px(4.0))
            .child(
                ListItem::new(key.id())
                    .selected(row.selected)
                    .w_full()
                    .h(px(height))
                    .px(px(0.0))
                    .py(px(0.0))
                    .rounded(px(6.0))
                    .accessibility_label(format!("{}: {}", row.title, tr(cx, T, row.progress.label())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate(key, cx)))
                    .when_some(tooltip_text, |this, text| {
                        this.tooltip(move |window, cx| Tooltip::new(text.clone()).max_w(px(360.0)).build(window, cx))
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(height))
                            .when(running_unit && !row.selected, |this| this.bg(t.run.opacity(0.10)).rounded(px(6.0)))
                            .when_some(accent, |this, color| {
                                this.child(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(3.0))
                                        .bottom(px(3.0))
                                        .w(px(2.5))
                                        .rounded(px(2.0))
                                        .bg(color),
                                )
                            })
                            .when_some(row.open, |this, open| {
                                this.child(
                                    div()
                                        .id(ElementId::NamedChild(Arc::new(key.id()), "chevron".into()))
                                        .absolute()
                                        .left(px(tier.chevron))
                                        .top(px(tier.pad + (tier.line - 16.0) / 2.0))
                                        .size(px(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(3.0))
                                        .cursor_pointer()
                                        .hover(|this| this.bg(t.border))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            this.set_open(key, !open, cx);
                                        }))
                                        .child(
                                            Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                                                .size(px(13.0))
                                                .text_color(t.muted),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .left(px(tier.icon))
                                    .top(px(mark_top))
                                    .when(done, |this| this.opacity(0.75))
                                    .child(state_mark(row.progress, tier.size, live, &t)),
                            )
                            .child(
                                v_flex()
                                    .absolute()
                                    .left(px(tier.text))
                                    .top(px(tier.pad))
                                    .w(px(title_width))
                                    .child(
                                        div()
                                            .w(px(title_width))
                                            .text_size(px(tier.font))
                                            .line_height(px(tier.line))
                                            .font_weight(weight)
                                            .text_color(title_color)
                                            .line_clamp(MAX_TITLE_LINES)
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .children(meta),
                            )
                            .when(matches!(key, RowKey::Module { .. }), |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .right(px(RIGHT_PAD - 4.0))
                                        .top(px(tier.pad))
                                        .text_size(px(11.5))
                                        .line_height(px(tier.line))
                                        .text_color(if done { t.muted } else { t.text })
                                        .child(fraction(row.tally)),
                                )
                            })
                            .when(ring, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(t.gold),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let tally = self.tally;
        let filter = self.filter;
        let figure = if tally.total() == 0 {
            "— / —".to_owned()
        } else {
            format!("{} / {}", tally.verified, tally.total())
        };
        let goals = (self.goals_total > 0).then(|| format!("{} {} / {}", tr(cx, T, "Goals"), self.goals_done, self.goals_total));
        let total = tally.total().max(1) as f32;
        let segment = |count: usize, color: Hsla| {
            (count > 0).then(|| {
                div()
                    .h_full()
                    .w(relative(count as f32 / total))
                    .rounded(px(1.5))
                    .bg(color)
            })
        };
        v_flex()
            .flex_none()
            .w_full()
            .gap(px(8.0))
            .px(px(12.0))
            .pt(px(12.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_baseline()
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_baseline()
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .line_height(px(20.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(figure),
                            )
                            .child(div().text_size(px(12.0)).text_color(t.muted).child(tr(cx, T, "verified"))),
                    )
                    .when_some(goals, |this, goals| {
                        this.child(div().text_size(px(12.0)).text_color(t.muted).child(goals))
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(5.0))
                    .gap(px(1.0))
                    .rounded_full()
                    .bg(t.border)
                    .children(segment(tally.verified, t.ok))
                    .children(segment(tally.review, t.review))
                    .children(segment(tally.running, t.run))
                    .children(segment(tally.failed, t.fail)),
            )
            .child(
                h_flex().w_full().flex_wrap().gap(px(2.0)).children(Filter::ALL.into_iter().map(|option| {
                    let count = option.count(tally);
                    let selected = filter == option;
                    Button::new(("todo-filter", option as usize))
                        .ghost()
                        .xsmall()
                        .compact()
                        .selected(selected)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_filter(option, cx)))
                        .child(
                            h_flex()
                                .gap(px(3.0))
                                .px(px(5.0))
                                .text_size(px(12.0))
                                .text_color(if selected { t.text } else { t.muted })
                                .child(tr(cx, T, option.label()))
                                .when(count > 0, |this| {
                                    this.child(div().text_size(px(11.0)).text_color(t.muted).child(count.to_string()))
                                }),
                        )
                })),
            )
    }

    fn frame(&self, t: &Tokens) -> Div {
        v_flex()
            .h_full()
            .bg(t.surface)
            .text_color(t.text)
            .when_some(self.forced_width, |this, width| this.w(px(width)).border_r_1().border_color(t.border))
            .when(self.forced_width.is_none(), |this| this.w_full())
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let viewport = self.viewport.clone();
        let painted = self.painted_scroll.clone();
        let handle = self.scroll.clone();
        let show_jump = !self.follow && self.target_visible(self.scroll_pos()) == Some(false);
        div()
            .id("todo-list")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                v_virtual_list(cx.entity(), "todo-rows", self.sizes.clone(), |this, range, window, cx| {
                    range.map(|index| this.render_row(index, window, cx)).collect::<Vec<_>>()
                })
                .track_scroll(&self.scroll)
                .py(px(4.0)),
            )
            .vertical_scrollbar(&self.scroll)
            .when(show_jump, |this| {
                this.child(
                    h_flex().absolute().left_0().right_0().bottom(px(10.0)).justify_center().child(
                        Button::new("todo-jump")
                            .small()
                            .outline()
                            .label(tr(cx, T, "Back to active work"))
                            .bg(t.raised)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.jump_to_active(cx))),
                    ),
                )
            })
            .on_prepaint(move |bounds, window, _| {
                painted.set(-f32::from(handle.offset().y));
                let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                if viewport.get() != size {
                    viewport.set(size);
                    window.request_animation_frame();
                }
            })
    }

    fn apply_scene(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.state.read(cx).scene.clone() else { return };
        for part in scene.split('+') {
            match part {
                "todo-sim" => {
                    let state = self.state.clone();
                    cx.spawn(async move |_, cx| {
                        loop {
                            cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                            let _ = state.update(cx, |s, cx| {
                                edit_run(s, |run| {
                                    if let Some(i) = run.units.iter().position(|u| u.state == UnitState::Running) {
                                        run.units[i].state = UnitState::Verified;
                                        if let Some(j) = (i + 1..run.units.len()).find(|&j| run.units[j].state == UnitState::Pending) {
                                            run.units[j].state = UnitState::Running;
                                        }
                                    }
                                });
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
                "todo-narrow" => self.forced_width = Some(260.0),
                "todo-filter-active" => self.filter = Filter::Active,
                "todo-filter-review" => self.filter = Filter::Review,
                "todo-filter-failed" => self.filter = Filter::Failed,
                "todo-planning" => self.state.update(cx, |state, _| edit_run(state, plan_scene)),
                "todo-goals-only" => self.state.update(cx, |state, _| edit_run(state, goals_only_scene)),
                "todo-unlinked" => self.state.update(cx, |state, _| edit_run(state, unlinked_scene)),
                "todo-big" => self.state.update(cx, |state, _| edit_run(state, big_scene)),
                "todo-finished" => self.state.update(cx, |state, _| state.current = Some(3)),
                "todo-select-unit" => self.state.update(cx, |state, _| {
                    state.selection = Selection::Unit(2);
                }),
                _ => {}
            }
        }
    }
}

fn meta_color(t: &Tokens, progress: Progress) -> Hsla {
    match progress {
        Progress::Unknown | Progress::Pending | Progress::Done => t.muted,
        other => progress_color(t, other),
    }
}

fn edit_run(state: &mut AppState, edit: impl FnOnce(&mut Run)) {
    if let Some(run) = state.current.and_then(|i| state.runs.get_mut(i)) {
        edit(run);
    }
}

fn plan_scene(run: &mut Run) {
    run.goals.clear();
    run.modules.clear();
    run.units.clear();
    for (i, stage) in run.stages.iter_mut().enumerate() {
        stage.phase = if i == 0 { Phase::Running } else { Phase::Waiting };
        stage.note = (i == 0).then(|| "14 min".into());
    }
}

fn goals_only_scene(run: &mut Run) {
    run.modules.clear();
    run.units.clear();
    for goal in &mut run.goals {
        goal.modules.clear();
    }
    for (i, stage) in run.stages.iter_mut().enumerate() {
        stage.phase = match i {
            0 | 1 => Phase::Done,
            2 => Phase::Running,
            _ => Phase::Waiting,
        };
    }
}

fn unlinked_scene(run: &mut Run) {
    run.goals[2].modules = vec![99];
    run.goals[1].modules.clear();
    run.goals.push(GoalItem {
        id: "G6".into(),
        title: "A goal whose modules were never named by the plan, with a long title that needs wrapping across lines to read".into(),
        modules: vec![],
    });
    run.goals[3].modules.retain(|&m| m != 11);
    run.units[1].title = "Retry every transient failure at the shared I/O layer with exponential backoff, keeping the attempt count visible to the caller and never retrying a change that already landed".into();
}

fn big_scene(run: &mut Run) {
    let states = [
        UnitState::Verified,
        UnitState::Verified,
        UnitState::Verified,
        UnitState::Built,
        UnitState::Pending,
        UnitState::Failed,
        UnitState::Pending,
        UnitState::Verified,
        UnitState::Running,
        UnitState::Pending,
    ];
    let template = run.units[0].clone();
    run.goals.clear();
    run.modules.clear();
    run.units.clear();
    for m in 0..64 {
        let mut units = vec![];
        for u in 0..10 {
            let state = if m < 40 { UnitState::Verified } else { states[(m + u) % states.len()] };
            units.push(run.units.len());
            let mut unit = template.clone();
            unit.id = format!("u{m}_{u}").into();
            unit.title = format!("Implement step {u} of module {m} with its tests").into();
            unit.module = m;
            unit.state = state;
            unit.attempts = (m + u) as u32 % 3;
            run.units.push(unit);
        }
        run.modules.push(Module {
            id: format!("m{m}").into(),
            purpose: format!("Module {m}: a part of the plan").into(),
            depends: vec![],
            units,
            contract: None,
        });
        if m % 8 == 0 {
            run.goals.push(GoalItem {
                id: format!("G{}", m / 8 + 1).into(),
                title: format!("Deliver the {}th slice of the plan", m / 8 + 1).into(),
                modules: vec![],
            });
        }
        let last = run.goals.len() - 1;
        run.goals[last].modules.push(m);
    }
}

fn measure(rows: &mut [Row], list_width: f32, window: &Window) {
    let system = window.text_system().clone();
    let base = window.text_style().font();
    let wrapper = |tier: &Tier, weight: FontWeight| {
        system.line_wrapper(Font { weight, ..base.clone() }, px(tier.font))
    };
    let mut goal = wrapper(&GOAL, FontWeight::SEMIBOLD);
    let mut module = wrapper(&MODULE, FontWeight::MEDIUM);
    let mut unit = wrapper(&UNIT, FontWeight::NORMAL);
    for row in rows.iter_mut() {
        let width = px(row.key.title_width(list_width));
        let fragments = [LineFragment::text(&row.title)];
        let adjust = IndentAdjustment::default();
        let lines = 1 + match row.key {
            RowKey::Goal(_) => goal.wrap_line(&fragments, width, adjust).count(),
            RowKey::Module { .. } => module.wrap_line(&fragments, width, adjust).count(),
            RowKey::Unit { .. } => unit.wrap_line(&fragments, width, adjust).count(),
            RowKey::Section => 0,
        };
        row.lines = lines.min(MAX_TITLE_LINES);
        row.clipped = lines > MAX_TITLE_LINES;
    }
}

impl Render for Todo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let t = cx.tokens();
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            return self
                .frame(&t)
                .child(placeholder(None, tr(cx, T, "Make a wish and its goals will be listed here."), None, &t))
                .into_any_element();
        };
        let planless = run.goals.is_empty() && run.modules.is_empty();
        let building_note = run.modules.is_empty().then(|| plan_note(run));
        let reduce_motion = state.reduce_motion;
        let live = !reduce_motion;
        let empty_filter = self.rows.is_empty() && !planless;
        let filter = self.filter;
        if planless {
            let note = building_note.expect("a run without modules has a note");
            let color = if note.failed { t.fail } else { t.run };
            let mark = if note.busy && live {
                Spinner::new().icon(IconName::LoaderCircle).with_size(px(20.0)).color(color).into_any_element()
            } else if note.failed {
                Icon::new(IconName::CircleX).size(px(20.0)).text_color(color).into_any_element()
            } else {
                Icon::new(IconName::LoaderCircle).size(px(20.0)).text_color(t.idle).into_any_element()
            };
            return self
                .frame(&t)
                .child(placeholder(Some(mark), tr(cx, T, note.text), note.detail, &t))
                .into_any_element();
        }
        self.frame(&t)
            .child(self.render_header(cx))
            .when_some(building_note, |this, note| {
                this.child(
                    h_flex()
                        .flex_none()
                        .gap(px(8.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .border_b_1()
                        .border_color(t.border)
                        .text_size(px(12.0))
                        .text_color(t.muted)
                        .child(if note.busy && live {
                            Spinner::new().icon(IconName::LoaderCircle).with_size(px(14.0)).color(t.run).into_any_element()
                        } else {
                            Icon::new(IconName::LoaderCircle).size(px(14.0)).text_color(t.idle).into_any_element()
                        })
                        .child(tr(cx, T, note.text)),
                )
            })
            .when(empty_filter, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .px(px(20.0))
                        .child(div().text_size(px(12.5)).text_color(t.muted).text_center().child(tr(cx, T, filter.empty_text())))
                        .when(filter != Filter::All, |this| {
                            this.child(
                                Button::new("todo-show-all")
                                    .small()
                                    .ghost()
                                    .label(tr(cx, T, "Show all"))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_filter(Filter::All, cx))),
                            )
                        }),
                )
            })
            .when(!empty_filter, |this| this.child(self.render_list(cx)))
            .into_any_element()
    }
}

fn placeholder(mark: Option<AnyElement>, text: &str, detail: Option<SharedString>, t: &Tokens) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .px(px(24.0))
        .children(mark)
        .child(
            div()
                .text_size(px(13.0))
                .line_height(px(20.0))
                .text_color(t.text)
                .text_center()
                .child(text.to_owned()),
        )
        .when_some(detail, |this, detail| {
            this.child(div().text_size(px(11.5)).text_color(t.muted).text_center().child(detail))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    fn unit(module: usize, state: UnitState) -> Unit {
        Unit {
            id: "u".into(),
            title: "title".into(),
            module,
            state,
            raw_state: "".into(),
            attempts: 1,
            files: UnitFiles::default(),
        }
    }

    fn module(units: Vec<usize>) -> Module {
        Module { id: "m".into(), purpose: "purpose".into(), depends: vec![], units, contract: None }
    }

    fn goal(modules: Vec<usize>) -> GoalItem {
        GoalItem { id: "G".into(), title: "goal".into(), modules }
    }

    fn run_of(goals: Vec<GoalItem>, modules: Vec<Module>, units: Vec<Unit>) -> Run {
        Run {
            started_at: None,
            session: "s".into(),
            project: "p".into(),
            cwd: None,
            wish: "w".into(),
            status: RunStatus::Active,
            hosted: true,
            started: "".into(),
            stages: vec![],
            goals,
            modules,
            units,
            usage: Usage::default(),
            needs: vec![],
            log: vec![],
            decisions: vec![],
            report: None,
            elapsed: None,
            warnings: vec![],
        }
    }

    fn sample() -> Run {
        run_of(
            vec![goal(vec![0, 1]), goal(vec![2])],
            vec![module(vec![0, 1]), module(vec![2]), module(vec![3, 4])],
            vec![
                unit(0, UnitState::Verified),
                unit(0, UnitState::Running),
                unit(1, UnitState::Verified),
                unit(2, UnitState::Failed),
                unit(2, UnitState::Built),
            ],
        )
    }

    fn keys(rows: &[Row]) -> Vec<RowKey> {
        rows.iter().map(|r| r.key).collect()
    }

    fn all_rows(run: &Run) -> Vec<Row> {
        build_rows(run, &Expansion::default(), Filter::All, &Selection::None)
    }

    fn stage(name: &'static str, phase: Phase) -> Stage {
        Stage { name: name.into(), phase, note: None }
    }

    fn tally_of(states: &[UnitState]) -> Tally {
        let mut tally = Tally::default();
        for state in states {
            tally.add(Some(*state));
        }
        tally
    }

    #[test]
    fn empty_module_is_not_done() {
        assert_eq!(Tally::default().progress(), Progress::Pending);
    }

    #[test]
    fn module_is_done_only_when_every_unit_is_verified() {
        assert_eq!(tally_of(&[UnitState::Verified, UnitState::Verified]).progress(), Progress::Done);
        assert_eq!(tally_of(&[UnitState::Verified, UnitState::Pending]).progress(), Progress::Partial);
        assert_eq!(tally_of(&[UnitState::Verified, UnitState::Built]).progress(), Progress::Review);
    }

    #[test]
    fn progress_prefers_what_needs_attention() {
        let failed_while_running = tally_of(&[UnitState::Failed, UnitState::Running, UnitState::Verified]);
        assert_eq!(failed_while_running.progress(), Progress::Failed);
        assert_eq!(tally_of(&[UnitState::Running, UnitState::Built]).progress(), Progress::Running);
    }

    #[test]
    fn a_unit_that_cannot_be_found_keeps_a_module_from_done() {
        let run = run_of(vec![], vec![], vec![unit(0, UnitState::Verified)]);
        let tally = Tally::of(&run, &[0, 7]);
        assert_eq!(tally.unknown, 1);
        assert_eq!(tally.progress(), Progress::Partial);
    }

    #[test]
    fn goal_without_modules_is_neutral() {
        let (_, progress) = goal_summary(&goal(vec![]), &[]);
        assert_eq!(progress, Progress::Unknown);
    }

    #[test]
    fn goal_naming_only_missing_modules_is_neutral() {
        let (_, progress) = goal_summary(&goal(vec![4, 9]), &[Tally::default()]);
        assert_eq!(progress, Progress::Unknown);
    }

    #[test]
    fn goal_with_a_missing_module_is_never_done() {
        let tallies = [tally_of(&[UnitState::Verified])];
        let (tally, progress) = goal_summary(&goal(vec![0, 9]), &tallies);
        assert_eq!(tally.unknown, 1);
        assert_eq!(progress, Progress::Partial);
    }

    #[test]
    fn goal_counts_a_repeated_module_once() {
        let tallies = [tally_of(&[UnitState::Verified, UnitState::Pending])];
        let (tally, _) = goal_summary(&goal(vec![0, 0]), &tallies);
        assert_eq!(tally.total(), 2);
    }

    #[test]
    fn goal_state_comes_from_its_modules() {
        let run = sample();
        let tallies: Vec<Tally> = run.modules.iter().map(|m| Tally::of(&run, &m.units)).collect();
        assert_eq!(goal_summary(&run.goals[0], &tallies).1, Progress::Running);
        assert_eq!(goal_summary(&run.goals[1], &tallies).1, Progress::Failed);
    }

    #[test]
    fn modules_needing_attention_open_by_default() {
        let rows = all_rows(&sample());
        assert_eq!(
            keys(&rows),
            vec![
                RowKey::Goal(0),
                RowKey::Module { goal: Some(0), module: 0 },
                RowKey::Unit { goal: Some(0), module: Some(0), unit: 0 },
                RowKey::Unit { goal: Some(0), module: Some(0), unit: 1 },
                RowKey::Module { goal: Some(0), module: 1 },
                RowKey::Goal(1),
                RowKey::Module { goal: Some(1), module: 2 },
                RowKey::Unit { goal: Some(1), module: Some(2), unit: 3 },
                RowKey::Unit { goal: Some(1), module: Some(2), unit: 4 },
            ]
        );
    }

    #[test]
    fn finished_goals_start_closed_and_a_user_can_open_them() {
        let run = run_of(vec![goal(vec![0])], vec![module(vec![0])], vec![unit(0, UnitState::Verified)]);
        assert_eq!(keys(&all_rows(&run)), vec![RowKey::Goal(0)]);
        let mut expansion = Expansion::default();
        expansion.set(RowKey::Goal(0), true);
        let rows = build_rows(&run, &expansion, Filter::All, &Selection::None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].open, Some(true));
    }

    #[test]
    fn a_user_can_close_a_module_with_active_work() {
        let run = sample();
        let mut expansion = Expansion::default();
        expansion.set(RowKey::Module { goal: Some(0), module: 0 }, false);
        let rows = build_rows(&run, &expansion, Filter::All, &Selection::None);
        assert!(!rows.iter().any(|r| matches!(r.key, RowKey::Unit { unit: 0 | 1, .. })));
    }

    #[test]
    fn rows_with_nothing_under_them_have_no_chevron() {
        let run = run_of(vec![goal(vec![])], vec![module(vec![])], vec![]);
        let rows = all_rows(&run);
        assert_eq!(rows[0].open, None);
        assert_eq!(rows.iter().find(|r| matches!(r.key, RowKey::Module { .. })).unwrap().open, None);
    }

    #[test]
    fn filter_keeps_only_matching_units_and_their_parents() {
        let run = sample();
        let rows = build_rows(&run, &Expansion::default(), Filter::Failed, &Selection::None);
        assert_eq!(
            keys(&rows),
            vec![
                RowKey::Goal(1),
                RowKey::Module { goal: Some(1), module: 2 },
                RowKey::Unit { goal: Some(1), module: Some(2), unit: 3 },
            ]
        );
    }

    #[test]
    fn filtering_ignores_what_is_closed() {
        let run = sample();
        let mut expansion = Expansion::default();
        expansion.set(RowKey::Goal(0), false);
        expansion.set(RowKey::Module { goal: Some(0), module: 0 }, false);
        let rows = build_rows(&run, &expansion, Filter::Active, &Selection::None);
        assert!(rows.iter().any(|r| matches!(r.key, RowKey::Unit { unit: 1, .. })));
        assert!(rows.iter().all(|r| r.open.is_none()));
    }

    #[test]
    fn filters_match_the_right_states() {
        assert!(Filter::Active.matches(UnitState::Running));
        assert!(!Filter::Active.matches(UnitState::Built));
        assert!(Filter::Review.matches(UnitState::Built));
        assert!(Filter::Failed.matches(UnitState::Failed));
        assert!(!Filter::Failed.matches(UnitState::Verified));
        assert!(Filter::All.matches(UnitState::Pending));
    }

    #[test]
    fn filter_counts_come_from_the_tally() {
        let tally = tally_of(&[UnitState::Running, UnitState::Built, UnitState::Built, UnitState::Failed]);
        assert_eq!(Filter::All.count(tally), 4);
        assert_eq!(Filter::Active.count(tally), 1);
        assert_eq!(Filter::Review.count(tally), 2);
        assert_eq!(Filter::Failed.count(tally), 1);
    }

    #[test]
    fn a_filter_never_hides_the_selected_unit() {
        let run = sample();
        let rows = build_rows(&run, &Expansion::default(), Filter::Failed, &Selection::Unit(2));
        let selected: Vec<_> = rows.iter().filter(|r| r.selected).map(|r| r.key).collect();
        assert_eq!(selected, vec![RowKey::Unit { goal: Some(0), module: Some(1), unit: 2 }]);
    }

    #[test]
    fn a_filter_never_hides_the_selected_module() {
        let run = sample();
        let rows = build_rows(&run, &Expansion::default(), Filter::Failed, &Selection::Module(1));
        assert!(rows.iter().any(|r| r.key == RowKey::Module { goal: Some(0), module: 1 } && r.selected));
    }

    #[test]
    fn modules_linked_to_no_goal_are_still_listed() {
        let run = run_of(
            vec![goal(vec![0])],
            vec![module(vec![0]), module(vec![1])],
            vec![unit(0, UnitState::Pending), unit(1, UnitState::Pending)],
        );
        let rows = all_rows(&run);
        let section = rows.iter().position(|r| r.key == RowKey::Section).expect("a section");
        assert_eq!(rows[section + 1].key, RowKey::Module { goal: None, module: 1 });
    }

    #[test]
    fn without_goals_modules_need_no_section() {
        let run = run_of(vec![], vec![module(vec![0])], vec![unit(0, UnitState::Running)]);
        let rows = all_rows(&run);
        assert!(rows.iter().all(|r| r.key != RowKey::Section));
        assert_eq!(rows[0].key, RowKey::Module { goal: None, module: 0 });
    }

    #[test]
    fn units_no_module_claims_are_still_listed() {
        let run = run_of(vec![goal(vec![0])], vec![module(vec![0])], vec![unit(0, UnitState::Pending), unit(9, UnitState::Failed)]);
        let rows = all_rows(&run);
        assert!(rows.iter().any(|r| r.key == RowKey::Unit { goal: None, module: None, unit: 1 }));
    }

    #[test]
    fn every_module_and_unit_appears_when_everything_is_open() {
        let run = sample();
        let mut expansion = Expansion::default();
        for g in 0..run.goals.len() {
            expansion.goals.insert(g, true);
        }
        for m in 0..run.modules.len() {
            expansion.modules.insert(m, true);
        }
        let rows = build_rows(&run, &expansion, Filter::All, &Selection::None);
        for m in 0..run.modules.len() {
            assert!(rows.iter().any(|r| matches!(r.key, RowKey::Module { module, .. } if module == m)));
        }
        for u in 0..run.units.len() {
            assert!(rows.iter().any(|r| matches!(r.key, RowKey::Unit { unit, .. } if unit == u)));
        }
    }

    #[test]
    fn indices_out_of_range_do_not_panic() {
        let run = run_of(vec![goal(vec![5])], vec![module(vec![8])], vec![]);
        let rows = all_rows(&run);
        assert_eq!(rows[0].progress, Progress::Unknown);
        assert!(rows.iter().any(|r| matches!(r.key, RowKey::Module { module: 0, .. })));
    }

    #[test]
    fn selecting_a_goal_selects_the_module_that_most_needs_a_look() {
        let run = run_of(
            vec![goal(vec![0, 1, 2])],
            vec![module(vec![0]), module(vec![1]), module(vec![2])],
            vec![unit(0, UnitState::Verified), unit(1, UnitState::Running), unit(2, UnitState::Failed)],
        );
        assert_eq!(selection_of(&run, RowKey::Goal(0)), Selection::Module(2));
    }

    #[test]
    fn a_goal_with_unknown_modules_selects_nothing() {
        let run = run_of(vec![goal(vec![])], vec![], vec![]);
        assert_eq!(selection_of(&run, RowKey::Goal(0)), Selection::None);
    }

    #[test]
    fn rows_select_their_own_item() {
        let run = sample();
        assert_eq!(selection_of(&run, RowKey::Module { goal: Some(0), module: 1 }), Selection::Module(1));
        assert_eq!(selection_of(&run, RowKey::Unit { goal: None, module: None, unit: 3 }), Selection::Unit(3));
    }

    #[test]
    fn revealing_a_unit_opens_its_module_and_goal() {
        let run = run_of(
            vec![goal(vec![0])],
            vec![module(vec![0])],
            vec![unit(0, UnitState::Verified)],
        );
        let mut expansion = Expansion::default();
        expand_for(&run, &mut expansion, &Selection::Unit(0));
        assert_eq!(keys(&build_rows(&run, &expansion, Filter::All, &Selection::Unit(0))).len(), 3);
    }

    #[test]
    fn revealing_a_module_opens_its_goal_only() {
        let run = run_of(
            vec![goal(vec![0])],
            vec![module(vec![0])],
            vec![unit(0, UnitState::Verified)],
        );
        let mut expansion = Expansion::default();
        expand_for(&run, &mut expansion, &Selection::Module(0));
        assert_eq!(expansion.goals.get(&0), Some(&true));
        assert!(expansion.modules.is_empty());
    }

    #[test]
    fn reveal_finds_a_unit_whose_declared_module_is_wrong() {
        let run = run_of(vec![goal(vec![1])], vec![module(vec![]), module(vec![0])], vec![unit(0, UnitState::Verified)]);
        let mut expansion = Expansion::default();
        expand_for(&run, &mut expansion, &Selection::Unit(0));
        assert_eq!(expansion.modules.get(&1), Some(&true));
    }

    #[test]
    fn follow_prefers_running_units() {
        let rows = all_rows(&sample());
        assert_eq!(rows[follow_target(&rows).unwrap()].key, RowKey::Unit { goal: Some(0), module: Some(0), unit: 1 });
    }

    #[test]
    fn follow_falls_back_to_failures_then_review() {
        let run = run_of(
            vec![goal(vec![0])],
            vec![module(vec![0, 1])],
            vec![unit(0, UnitState::Built), unit(0, UnitState::Failed)],
        );
        let rows = all_rows(&run);
        assert_eq!(rows[follow_target(&rows).unwrap()].key, RowKey::Unit { goal: Some(0), module: Some(0), unit: 1 });
    }

    #[test]
    fn follow_uses_the_module_when_its_units_are_closed() {
        let run = sample();
        let mut expansion = Expansion::default();
        expansion.set(RowKey::Module { goal: Some(0), module: 0 }, false);
        let rows = build_rows(&run, &expansion, Filter::All, &Selection::None);
        assert_eq!(rows[follow_target(&rows).unwrap()].key, RowKey::Module { goal: Some(0), module: 0 });
    }

    #[test]
    fn nothing_active_means_nothing_to_follow() {
        let run = run_of(vec![], vec![module(vec![0])], vec![unit(0, UnitState::Verified)]);
        assert_eq!(follow_target(&all_rows(&run)), None);
    }

    #[test]
    fn visibility_requires_the_whole_row() {
        assert!(is_visible(100.0, 40.0, 100.0, 40.0));
        assert!(!is_visible(90.0, 40.0, 100.0, 200.0));
        assert!(!is_visible(280.0, 40.0, 100.0, 200.0));
    }

    #[test]
    fn centering_stays_inside_the_content() {
        assert_eq!(centered_scroll(0.0, 40.0, 200.0, 1000.0), 0.0);
        assert_eq!(centered_scroll(500.0, 40.0, 200.0, 1000.0), 420.0);
        assert_eq!(centered_scroll(980.0, 20.0, 200.0, 1000.0), 800.0);
        assert_eq!(centered_scroll(10.0, 20.0, 200.0, 100.0), 0.0);
    }

    #[test]
    fn nearest_scroll_moves_only_as_far_as_needed() {
        assert_eq!(nearest_scroll(50.0, 40.0, 100.0, 200.0), 50.0);
        assert_eq!(nearest_scroll(280.0, 40.0, 100.0, 200.0), 120.0);
        assert_eq!(nearest_scroll(150.0, 40.0, 100.0, 200.0), 100.0);
    }

    fn row(key: RowKey, lines: usize) -> Row {
        let mut row = Row::new(key, "x".into(), Progress::Pending, Tally::default());
        row.lines = lines;
        row
    }

    #[test]
    fn row_heights_follow_their_line_count() {
        assert!(row(RowKey::Goal(0), 2).height() > row(RowKey::Goal(0), 1).height());
        assert_eq!(row(RowKey::Goal(0), 5).height(), row(RowKey::Goal(0), MAX_TITLE_LINES).height());
    }

    #[test]
    fn anchored_scroll_keeps_the_top_row_in_place() {
        let unit_key = |unit| RowKey::Unit { goal: None, module: None, unit };
        let old = vec![row(RowKey::Goal(0), 1), row(unit_key(1), 1), row(unit_key(2), 1)];
        let new = vec![row(RowKey::Goal(0), 1), row(unit_key(9), 1), row(unit_key(1), 1), row(unit_key(2), 1)];
        let top = old[0].height() + 5.0;
        assert_eq!(anchored_scroll(&old, &new, top), row_top(&new, 2) + 5.0);
    }

    #[test]
    fn anchored_scroll_falls_forward_when_the_top_row_is_gone() {
        let unit_key = |unit| RowKey::Unit { goal: None, module: None, unit };
        let old = vec![row(unit_key(1), 1), row(unit_key(2), 1), row(unit_key(3), 1)];
        let new = vec![row(unit_key(2), 1), row(unit_key(3), 1)];
        assert_eq!(anchored_scroll(&old, &new, 3.0), 0.0);
    }

    #[test]
    fn plan_note_says_who_is_working() {
        let mut run = run_of(vec![], vec![], vec![]);
        run.stages = vec![stage("Astra", Phase::Running), stage("Critic", Phase::Waiting)];
        let note = plan_note(&run);
        assert_eq!(note.text, "Astra is planning…");
        assert!(note.busy);
        run.stages = vec![stage("Astra", Phase::Done), stage("Critic", Phase::Running)];
        assert_eq!(plan_note(&run).text, "The critic is reviewing the plan…");
        run.stages = vec![stage("Critic", Phase::Done), stage("Architect", Phase::Running)];
        assert_eq!(plan_note(&run).text, "The architect is splitting the plan into modules…");
    }

    #[test]
    fn plan_note_is_quiet_when_nothing_is_running() {
        let mut run = run_of(vec![], vec![], vec![]);
        run.stages = vec![stage("Astra", Phase::Waiting)];
        let note = plan_note(&run);
        assert_eq!(note.text, "Getting the work ready…");
        assert!(!note.busy);
    }

    #[test]
    fn plan_note_reports_a_failed_stage_first() {
        let mut run = run_of(vec![], vec![], vec![]);
        run.status = RunStatus::Blocked;
        run.stages = vec![stage("Astra", Phase::Failed)];
        let note = plan_note(&run);
        assert!(note.failed);
        assert_eq!(note.text, "Planning stopped before a plan was made.");
    }

    #[test]
    fn plan_note_follows_a_run_that_is_not_active() {
        let mut run = run_of(vec![], vec![], vec![]);
        run.stages = vec![stage("Astra", Phase::Running)];
        run.status = RunStatus::Blocked;
        assert_eq!(plan_note(&run).text, "This wish needs you before it can plan.");
        assert!(!plan_note(&run).busy);
        run.status = RunStatus::Done;
        assert_eq!(plan_note(&run).text, "This wish ended without a plan.");
        run.status = RunStatus::Waiting;
        assert_eq!(plan_note(&run).text, "Waiting on a background batch.");
    }

    #[test]
    fn unknown_figures_show_a_dash() {
        assert_eq!(fraction(Tally::default()), "—");
        assert_eq!(fraction(tally_of(&[UnitState::Verified, UnitState::Failed])), "1/2");
    }

    #[test]
    fn verified_counts_nothing_else_as_done() {
        let tally = tally_of(&[UnitState::Built, UnitState::Built, UnitState::Verified]);
        assert_eq!(fraction(tally), "1/3");
    }

    #[test]
    fn building_rows_stays_quick_at_large_size() {
        let units: Vec<Unit> = (0..800).map(|i| unit(i / 10, UnitState::Verified)).collect();
        let modules: Vec<Module> = (0..80).map(|m| module((m * 10..m * 10 + 10).collect())).collect();
        let goals: Vec<GoalItem> = (0..20).map(|g| goal((g * 4..g * 4 + 4).collect())).collect();
        let run = run_of(goals, modules, units);
        let mut expansion = Expansion::default();
        for m in 0..80 {
            expansion.modules.insert(m, true);
        }
        for g in 0..20 {
            expansion.goals.insert(g, true);
        }
        let started = std::time::Instant::now();
        for _ in 0..100 {
            let rows = build_rows(&run, &expansion, Filter::All, &Selection::Unit(400));
            assert!(rows.len() > 800);
        }
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "{:?}", started.elapsed());
    }
}
