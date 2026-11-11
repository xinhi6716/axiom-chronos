//! Every run, newest first: filter, search, open. Filtering and sorting are pure and sit apart from drawing.
use super::i18n::{tr, Lang, Table};
use super::model::*;
use super::{motion, ornament, palette, settings, theme};
use super::theme::{Step, TypeSize};
use super::motion::{Driver, Pace};
mod logic;
#[cfg(test)] mod accept;
use super::theme::Tokened as _;
use gpui_kit::base::actions::{SelectFirst, SelectLast, SelectPageDown, SelectPageUp};
use gpui_kit::base::actions::{Cancel, Confirm};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, Disableable as _, Icon, IconName, IndexPath, Selectable as _,
    Sizable as _, StyledExt as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cmp::Reverse;
use std::rc::Rc;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use gpui_kit::base::ElementExt as _;

gpui_kit::actions!(chronos_lists, [FocusSearch]);

const T: Table = &[
    ("Wishes", "願望"),
    ("Loading wishes", "正在載入願望"),
    ("Try again", "再試一次"),
    ("All", "全部"),
    ("Status", "狀態"),
    ("Project", "專案"),
    ("Wish", "願望"),
    ("Cost", "費用"),
    ("Started", "開始時間"),
    ("Search wishes and projects", "搜尋願望或專案"),
    ("More actions", "更多動作"),
    ("Copy session id", "複製工作階段 ID"),
    ("Session id copied", "已複製工作階段 ID"),
    ("No wishes yet", "還沒有許過願"),
    (
        "Every wish you make appears here with its status, progress and cost.",
        "許下的每個願望都會列在這裡，附上狀態、進度與費用。",
    ),
    ("Make a wish", "許個願"),
    ("No matching wishes", "沒有符合的願望"),
    (
        "Search looks in the wish text and the project name.",
        "搜尋範圍是願望內容與專案名稱。",
    ),
    ("Clear search", "清除搜尋"),
    ("No wishes are running", "目前沒有進行中的願望"),
    ("Nothing needs you right now", "目前沒有需要你處理的願望"),
    ("No finished wishes yet", "還沒有完成的願望"),
    ("Show all", "顯示全部"),
    (
        "Wishes that match this filter will appear here.",
        "符合這個篩選條件的願望會出現在這裡。",
    ),
    ("This needs attention", "這筆資料需要留意"),
];

/// Which runs the Wishes screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Running,
    NeedsYou,
    Done,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Running, Filter::NeedsYou, Filter::Done];

    /// A run waiting on a batch counts as running: the work is going on and nothing is asked of the user yet.
    pub fn admits(self, status: RunStatus) -> bool {
        match self {
            Filter::All => true,
            Filter::Running => matches!(status, RunStatus::Active | RunStatus::Waiting | RunStatus::LimitWait),
            Filter::NeedsYou => matches!(status, RunStatus::Blocked | RunStatus::Interrupted),
            Filter::Done => status == RunStatus::Done,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Running => "Running",
            Filter::NeedsYou => "Needs you",
            Filter::Done => "Done",
        }
    }

    fn empty_title(self) -> &'static str {
        match self {
            Filter::All => "No wishes yet",
            Filter::Running => "No wishes are running",
            Filter::NeedsYou => "Nothing needs you right now",
            Filter::Done => "No finished wishes yet",
        }
    }
}

/// Lowercased search terms; a row must contain all of them.
pub(super) fn search_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// True when every term appears in at least one of `fields`.
pub(super) fn matches_terms(fields: &[&str], terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let fields: Vec<String> = fields.iter().map(|field| field.to_lowercase()).collect();
    terms
        .iter()
        .all(|term| fields.iter().any(|field| field.contains(term.as_str())))
}

fn run_matches(run: &Run, terms: &[String]) -> bool {
    matches_terms(&[&run.wish, &run.project], terms)
}

/// How many runs each of [`Filter::ALL`] holds, within the current search.
pub fn filter_counts(runs: &[Run], terms: &[String]) -> [usize; 4] {
    let mut counts = [0; 4];
    for run in runs.iter().filter(|run| run_matches(run, terms)) {
        for (count, filter) in counts.iter_mut().zip(Filter::ALL) {
            *count += usize::from(filter.admits(run.status));
        }
    }
    counts
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `(month, day, hour, minute)` of a start time written like "Nov 3, 05:15".
fn parse_started(started: &str) -> Option<(u8, u8, u8, u8)> {
    let (month, rest) = started.trim().split_once(' ')?;
    let (day, time) = rest.split_once(',')?;
    let (hour, minute) = time.trim().split_once(':')?;
    let month = MONTHS.iter().position(|name| *name == month)? as u8 + 1;
    Some((
        month,
        day.trim().parse().ok()?,
        hour.parse().ok()?,
        minute.parse().ok()?,
    ))
}

/// A start time carries no year, so a month later than the current one is from last year.
fn started_key(started: &str, this_month: u8) -> Option<(bool, u8, u8, u8, u8)> {
    let (month, day, hour, minute) = parse_started(started)?;
    Some((month <= this_month, month, day, hour, minute))
}

/// Indices into `runs` of the rows to show, newest first. Times that cannot be read go last, in their given order.
pub fn visible_runs(runs: &[Run], filter: Filter, terms: &[String], this_month: u8) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..runs.len())
        .filter(|&i| filter.admits(runs[i].status) && run_matches(&runs[i], terms))
        .collect();
    rows.sort_by_cached_key(|&i| Reverse(started_key(&runs[i].started, this_month)));
    rows
}

/// Runs grouped in ledger order, preserving the order supplied within each state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunGroup {
    pub status: RunStatus,
    pub runs: Vec<usize>,
}

pub const GROUP_ORDER: [RunStatus; 6] = [
    RunStatus::Blocked, RunStatus::Active, RunStatus::Waiting,
    RunStatus::LimitWait, RunStatus::Interrupted, RunStatus::Done,
];

pub fn group_runs(runs: &[Run], visible: &[usize]) -> Vec<RunGroup> {
    GROUP_ORDER.into_iter().filter_map(|status| {
        let indices: Vec<_> = visible.iter().copied().filter(|&i| runs[i].status == status).collect();
        (!indices.is_empty()).then_some(RunGroup { status, runs: indices })
    }).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerRow {
    Divider { status: RunStatus, count: usize },
    Card(usize),
}

pub fn ledger_rows(groups: &[RunGroup]) -> Vec<LedgerRow> {
    groups.iter().flat_map(|group| {
        std::iter::once(LedgerRow::Divider { status: group.status, count: group.runs.len() })
            .chain(group.runs.iter().copied().map(LedgerRow::Card))
    }).collect()
}

/// Move over cards only, clamping at either end. With no current card, start at the first.
pub fn step_cards(rows: &[LedgerRow], from: Option<usize>, step: isize) -> Option<usize> {
    let cards: Vec<_> = rows.iter().filter_map(|row| match row {
        LedgerRow::Card(index) => Some(*index),
        LedgerRow::Divider { .. } => None,
    }).collect();
    if cards.is_empty() { return None; }
    let current = from.and_then(|index| cards.iter().position(|&i| i == index)).unwrap_or(0);
    Some(cards[current.saturating_add_signed(step).min(cards.len() - 1)])
}

fn index_path(groups: &[RunGroup], index: usize) -> Option<IndexPath> {
    groups.iter().enumerate().find_map(|(section, group)| {
        group.runs.iter().position(|&i| i == index).map(|row| IndexPath::new(row).section(section))
    })
}

fn selected_index(groups: &[RunGroup], runs: &[Run], session: Option<&SharedString>) -> Option<usize> {
    logic::selected_index(groups, runs, session)
}

/// The start time as the interface language writes it; anything unreadable stays as written.
pub fn started_label(started: &SharedString, lang: Lang) -> SharedString {
    match (lang, parse_started(started)) {
        (Lang::ZhTw, Some((month, day, hour, minute))) => {
            format!("{month}月{day}日 {hour:02}:{minute:02}").into()
        }
        _ => started.clone(),
    }
}

/// `verified / total`, or "—" before there is anything to verify.
pub fn progress_label(verified: usize, total: usize) -> String {
    if total == 0 {
        "—".to_owned()
    } else {
        format!("{verified} / {total}")
    }
}

pub fn cost_label(usd: Option<f64>) -> String {
    usd.map_or_else(|| "—".to_owned(), |usd| format!("${usd:.2}"))
}

pub fn can_resume(status: RunStatus) -> bool {
    matches!(status, RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted)
}

/// `text` on a single line, for rows that have room for only one.
pub(super) fn one_line(text: &SharedString) -> SharedString {
    if text.contains(['\n', '\r', '\t']) {
        text.split_whitespace().collect::<Vec<_>>().join(" ").into()
    } else {
        text.clone()
    }
}

/// The check that stops new wishes, if any: only a missing or signed-out `claude` or `codex` does.
pub fn wish_blocker(checks: &[Check]) -> Option<&Check> {
    checks.iter().find(|check| {
        check.status == CheckStatus::Fail && matches!(check.name.as_ref(), "claude" | "codex")
    })
}

fn this_month() -> u8 {
    crate::base::today()
        .get(5..7)
        .and_then(|month| month.parse().ok())
        .unwrap_or(12)
}

/// A centered explanation with an optional action, for lists with nothing to show.
pub(super) fn empty_state(
    icon: IconName,
    title: &'static str,
    description: impl Into<SharedString>,
    action: Option<AnyElement>,
    table: Table,
    cx: &App,
) -> Empty {
    let t = cx.tokens();
    Empty::new()
        .size_full()
        .gap_5()
        .header(
            EmptyHeader::new()
                .gap_1p5()
                .media(
                    EmptyMedia::new()
                        .size_12()
                        .rounded_full()
                        .bg(t.raised)
                        .text_color(t.muted)
                        .child(Icon::new(icon).size(px(22.))),
                )
                .title(
                    EmptyTitle::new()
                        .text_base()
                        .font_semibold()
                        .child(tr(cx, table, title)),
                )
                .description(EmptyDescription::new().child(description.into())),
        )
        .when_some(action, |this, action| {
            this.content(EmptyContent::new().child(action))
        })
}

/// Home, End, Page Up and Page Down for a list inside `context`.
pub(super) fn bind_list_keys(context: &'static str, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("home", SelectFirst, Some(context)),
        KeyBinding::new("end", SelectLast, Some(context)),
        KeyBinding::new("pageup", SelectPageUp, Some(context)),
        KeyBinding::new("pagedown", SelectPageDown, Some(context)),
    ]);
}

pub(super) fn select_row<D: ListDelegate>(
    list: &Entity<ListState<D>>,
    row: usize,
    window: &mut Window,
    cx: &mut App,
) {
    list.update(cx, |list, cx| {
        list.set_selected_index(Some(IndexPath::new(row)), window, cx);
        list.scroll_to_selected_item(window, cx);
    });
}

fn select_ix(list: &Entity<ListState<RunList>>, ix: IndexPath, window: &mut Window, cx: &mut App) {
    list.update(cx, |list, cx| {
        list.set_selected_index(Some(ix), window, cx);
        list.scroll_to_selected_item(window, cx);
    });
}

const PAGE_ROWS: usize = 10;
fn text_line_height(d: theme::Density, size: TypeSize) -> Pixels {
    theme::font(d, size) + theme::space(d, Step::Xs) * 2.0
}
fn row_height(d: theme::Density) -> Pixels {
    text_line_height(d, TypeSize::Body) * 2.0 + text_line_height(d, TypeSize::Caption)
        + theme::space(d, Step::Xs) * 5.0
}
fn column_width(d: theme::Density, n: f32) -> Pixels { theme::space(d, Step::Xxl) * n }

fn cell(width: Pixels) -> Div {
    div().flex_none().w(width).min_w_0()
}

pub(in crate::desktop) fn glyph_state(status: RunStatus, verified_end: bool) -> UnitState {
    match status {
        RunStatus::Active => UnitState::Running,
        RunStatus::Done if verified_end => UnitState::Verified,
        RunStatus::Done | RunStatus::Blocked => UnitState::Built,
        RunStatus::Waiting | RunStatus::LimitWait => UnitState::Pending,
        RunStatus::Interrupted => UnitState::Failed,
    }
}

fn menu_runner(state: Entity<AppState>) -> Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)> {
    Rc::new(move |action, target, window, cx| {
        let effect = { let state = state.read(cx); if !state.menu_allowed(&target, action) { return; } logic::resolve(&state.runs, &target, action) };
        match effect {
            Some(logic::RowEffect::Open(index)) => state.update(cx, |state, cx| state.open_run(index, cx)),
            Some(logic::RowEffect::Resume(index)) => state.update(cx, |state, cx| { state.open_run(index, cx); state.resume_current(None, cx); }),
            Some(logic::RowEffect::Folder(path)) if path.is_dir() => cx.open_with_system(&path),
            Some(logic::RowEffect::Copy(session)) => { cx.write_to_clipboard(ClipboardItem::new_string(session)); window.push_notification(Notification::success(tr(cx, T, "Session id copied")), cx); },
            _ => {}
        }
    })
}

struct RunList {
    state: Entity<AppState>,
    groups: Vec<RunGroup>,
    selected: Option<IndexPath>,
    selected_session: Option<SharedString>,
    focus: Option<FocusHandle>,
    hover_glows: motion::Glows<SharedString>,
    select_glows: motion::Glows<SharedString>,
    hovered: HashSet<SharedString>,
    bounds: Rc<RefCell<HashMap<SharedString, Bounds<Pixels>>>>,
    saving: bool,
}

impl ListDelegate for RunList {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.groups.len().max(1)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.groups.get(section).map_or(0, |group| group.runs.len())
    }

    fn render_section_header(&mut self, section: usize, _: &mut Window, cx: &mut Context<ListState<Self>>) -> Option<impl IntoElement> {
        let group = self.groups.get(section)?;
        let label = format!("{} {}", tr(cx, T, group.status.label()), group.runs.len());
        Some(h_flex().h(theme::row_height(theme::density(cx))).items_center().child(ornament::section_label(&cx.tokens(), &label)))
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let index = *self.groups.get(ix.section)?.runs.get(ix.row)?;
        let t = cx.tokens();
        let lang = *cx.global::<Lang>();
        let run = self.state.read(cx).runs.get(index)?;
        let selected = self.selected == Some(ix);
        let focused = selected && self.focus.as_ref().is_some_and(|f| f.is_focused(window));
        let (verified, total) = (run.count(UnitState::Verified), run.units.len());
        let status = run.status;
        let verified_end = run.verified_end();
        let full_wish = run.wish.clone();
        let wish = one_line(&run.wish);
        let project = run.project.clone();
        let place = run
            .cwd
            .as_ref()
            .map_or_else(|| project.clone(), |cwd| cwd.display().to_string().into());
        let started = started_label(&run.started, lang);
        let cost = run.usage.total_usd;
        let warning = (!run.warnings.is_empty()).then(|| run.warnings.join("\n"));
        let session = run.session.clone();
        let target = MenuTarget::Run(session.clone());
        let state = self.state.clone();
        let runner = menu_runner(state.clone());
        let context_target = target.clone();
        let context_state = state.clone();
        let context_runner = runner.clone();
        let d = theme::density(cx);
        let reduce = cx.reduce_motion();
        let hovered = self.hovered.contains(&session);
        let mut frames = Driver::new(reduce, cx);
        self.hover_glows.set(&session, if hovered { 1.0 } else { 0.0 }, frames.now(), reduce);
        self.select_glows.set(&session, if selected { 1.0 } else { 0.0 }, frames.now(), reduce);
        let (lift, glow, ring) = logic::levels(selected, hovered, focused, self.hover_glows.value(&session, &mut frames), self.select_glows.value(&session, &mut frames), reduce);
        frames.finish(window);
        let name = format!("{} {project} {wish}", tr(cx, T, status.label()));
        let hover_session = session.clone();
        let bounds_session = session.clone();
        let bounds = self.bounds.clone();
        let meta = logic::row_meta(run);
        let h = row_height(d);

        Some(
            ListItem::new(ix)
                .accessibility_label(name)
                // The enclosing List handles selection; suppress ListItem's row-wide fills.
                .disabled(true)
                .h(h)
                .py(theme::space(d, Step::Xs))
                .px(theme::space(d, Step::Xs))
                .cursor_pointer()
                .child(
                    // Keep popup state outside the card's changing array-index scope.
                    div().id(session).size_full()
                        .context_menu(move |menu, _, cx| palette::menu::context_menu(menu, context_target.clone(), context_state.clone(), context_runner.clone(), cx))
                        .child(
                            ornament::raised_card(&t, lift, glow)
                                .id(("wish-row", index))
                                .flex().items_center()
                                .relative().top(-theme::space(d, Step::Xs) * (lift * 0.5)).w_full().h(h - theme::space(d, Step::Xs) * 2.0)
                                .text_color(t.text)
                                .shadow({ let mut shadows = ornament::raised_card_shadows(&t, lift, glow); if ring { shadows.extend(theme::focus_ring(&t)); } shadows })
                                .px(theme::space(d, Step::Md)).gap(theme::space(d, Step::Sm))
                                .on_prepaint(move |rect, _, _| { bounds.borrow_mut().insert(bounds_session.clone(), rect); })
                                .on_hover(cx.listener(move |list, inside: &bool, _, cx| { if *inside { list.delegate_mut().hovered.insert(hover_session.clone()); } else { list.delegate_mut().hovered.remove(&hover_session); } cx.notify(); }))
                                .child(ornament::glyph(glyph_state(status, verified_end), f32::from(theme::font(d, TypeSize::Heading)), t.outcome(status, verified_end), 0.0))
                                .child(cell(column_width(d, 3.0)).when_some(ornament::progress_ring(&t, verified, total, f32::from(theme::space(d, Step::Xl))), |this, ring| {
                                    let count: SharedString = progress_label(verified, total).into();
                                    this.child(
                                        h_flex().flex_none().gap(theme::space(d, Step::Xs))
                                            .child(div().id(("wish-progress-ring", index)).child(ring).test_support())
                                            .child(
                                                div().id(("wish-progress-count", index))
                                                    .text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted)
                                                    .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
                                                    .role(Role::Label).aria_label(count.clone())
                                                    .child(count.clone().test_text(count))
                                                    .test_support(),
                                            ),
                                    )
                                }))
                                .child(
                                    cell(column_width(d, 4.0))
                                        .id(("wish-project", index))
                                        .px_2()
                                        .py_0p5()
                                        .rounded(theme::RADIUS_SMALL)
                                        .bg(t.surface)
                                        .text_size(theme::font(d, TypeSize::Body))
                                        .text_color(t.muted)
                                        .truncate()
                                        .child(project)
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(place.clone()).build(window, cx)
                                        }),
                                )
                                .child(
                                    v_flex().flex_1().min_w_0().gap(theme::space(d, Step::Xs))
                                        .child(h_flex().min_w_0().items_start().gap(theme::space(d, Step::Xs))
                                            .child(div().id(("wish-text", index)).flex_1().min_w_0().line_clamp(2).text_size(theme::font(d, TypeSize::Body)).line_height(text_line_height(d, TypeSize::Body)).text_color(t.text).child(wish.clone().test_text(wish.clone()))
                                                .tooltip(move |window, cx| Tooltip::new(full_wish.clone()).build(window, cx)))
                                            .when_some(warning, |this, warning| this.child(div().id(("wish-warning", index)).flex_none().text_color(t.review)
                                                .child(Icon::new(IconName::TriangleAlert).size(theme::font(d, TypeSize::Body)))
                                                .tooltip(move |window, cx| Tooltip::new(warning.clone()).build(window, cx)))))
                                        .child(div().w_full().truncate().text_size(theme::font(d, TypeSize::Caption)).line_height(text_line_height(d, TypeSize::Caption)).text_color(t.muted).child(meta.clone().test_text(meta))),
                                )
                                .child(
                                    cell(column_width(d, 2.25))
                                        .text_right()
                                        .text_size(theme::font(d, TypeSize::Body))
                                        .text_color(t.muted)
                                        .child(cost_label(cost)),
                                )
                                .child(
                                    cell(column_width(d, 3.25))
                                        .text_right()
                                        .text_size(theme::font(d, TypeSize::Caption))
                                        .text_color(t.muted)
                                        .truncate()
                                        .child(started.clone()),
                                )
                                .child(
                                    cell(theme::TARGET_MIN).child(
                                        Button::new(("wish-menu", index))
                                            .ghost()
                                            .small()
                                            .icon(IconName::Ellipsis)
                                            .tab_stop(selected)
                                            .accessibility_label(tr(cx, T, "More actions"))
                                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                                                palette::menu::context_menu(menu, target.clone(), state.clone(), runner.clone(), cx)
                                            }),
                                    ),
                                ),
                        ),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let previous = self.selected_session.clone();
        self.selected = ix;
        self.selected_session = ix
            .and_then(|ix| self.groups.get(ix.section)?.runs.get(ix.row))
            .and_then(|&index| self.state.read(cx).runs.get(index))
            .map(|run| run.session.clone());
        if self.saving && previous != self.selected_session {
            let selected = self.selected_session.as_ref().map(ToString::to_string);
            if settings::view_memory(cx, "wishes").selected != selected {
                settings::remember_view("wishes", |v| v.selected = selected, cx);
            }
        }
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(&index) = self.selected.and_then(|ix| self.groups.get(ix.section)?.runs.get(ix.row)) {
            self.state.update(cx, |state, cx| state.open_run(index, cx));
        }
    }
}

pub struct WishesView {
    state: Entity<AppState>,
    search: Entity<InputState>,
    placeholder: &'static str,
    list: Entity<ListState<RunList>>,
    filter: Filter,
    query: SharedString,
    terms: Vec<String>,
    counts: [usize; 4],
    on_screen: bool,
    restore: logic::Restore,
    saved_selected: Option<String>,
    saved_scroll: f32,
    last_scroll: Rc<Cell<f32>>,
    scroll_ready: Rc<Cell<bool>>,
    bounds: Rc<RefCell<HashMap<SharedString, Bounds<Pixels>>>>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    painted_scroll: Rc<Cell<f32>>,
    restore_pending: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl WishesView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if state.read(cx).scene.as_deref() == Some("many") {
            state.update(cx, |state, _| fill_for_stress(state));
        }
        bind_list_keys("WishList", cx);
        cx.bind_keys([KeyBinding::new("ctrl-f", FocusSearch, Some("WishesScreen"))]);

        let memory = settings::view_memory(cx, "wishes");
        if memory.sort.is_some() { settings::remember_view("wishes", |v| v.sort = None, cx); }
        let (filter, query) = logic::decode(memory.filter.as_deref());
        let bounds = Rc::new(RefCell::new(HashMap::new()));
        let placeholder = tr(cx, T, "Search wishes and projects");
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .clean_on_escape()
        });
        search.update(cx, |input, cx| input.set_value(query.clone(), window, cx));
        let list = cx.new(|cx| {
            ListState::new(
                RunList {
                    state: state.clone(),
                    groups: Vec::new(),
                    selected: None,
                    selected_session: None,
                    focus: None,
                    hover_glows: motion::Glows::new(Pace::Glow),
                    select_glows: motion::Glows::new(Pace::Glow),
                    hovered: HashSet::new(),
                    bounds: bounds.clone(),
                    saving: false,
                },
                window,
                cx,
            )
        });
        let focus = list.read(cx).focus_handle(cx);
        list.update(cx, |list, _| list.delegate_mut().focus = Some(focus));

        let _subscriptions = vec![
            cx.observe_in(&state, window, Self::on_state_changed),
            cx.subscribe_in(&search, window, Self::on_search_event),
        ];
        let on_screen = state.read(cx).view == Screen::Wishes;
        let mut this = Self {
            state,
            search,
            placeholder,
            list,
            filter,
            terms: search_terms(&query),
            query: query.into(),
            counts: [0; 4],
            on_screen,
            restore: logic::Restore::default(),
            saved_selected: memory.selected,
            saved_scroll: logic::scroll(memory.scroll),
            last_scroll: Rc::new(Cell::new(logic::scroll(memory.scroll))),
            scroll_ready: Rc::new(Cell::new(false)),
            bounds,
            viewport: Rc::new(Cell::new(Bounds::default())),
            painted_scroll: Rc::new(Cell::new(0.0)),
            restore_pending: Rc::new(Cell::new(false)),
            _subscriptions,
        };
        this.apply(false, window, cx);
        if on_screen {
            this.focus_list(window, cx);
        }
        this
    }

    fn on_state_changed(
        &mut self,
        _: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_screen = self.state.read(cx).view == Screen::Wishes;
        if on_screen {
            self.apply(false, window, cx);
            if !self.on_screen {
                self.focus_list(window, cx);
            }
        }
        self.on_screen = on_screen;
        cx.notify();
    }

    fn on_search_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The input reports Change only for committed text, so a word still being composed in an IME never filters.
            InputEvent::Change => {
                let value = input.read(cx).value();
                let capped: String = value.chars().take(120).collect();
                if value.as_ref() != capped { self.search.update(cx, |search, cx| search.set_value(capped.clone(), window, cx)); }
                if logic::cancels_restore(self.filter, self.filter, &self.query, &capped) {
                    self.terms = search_terms(&capped);
                    self.query = capped.into();
                    self.restore.cancel(); self.restore_pending.set(false); self.scroll_ready.set(true);
                    self.remember_filter(cx);
                    self.apply(true, window, cx);
                }
            }
            InputEvent::PressEnter { .. } => self.focus_list(window, cx),
            _ => {}
        }
    }

    fn apply(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let runs = &state.runs;
        let panel = panel_state(&state.feeds.runs, runs.is_empty());
        self.counts = filter_counts(runs, &self.terms);
        let visible = visible_runs(runs, self.filter, &self.terms, this_month());
        let groups = group_runs(runs, &visible);
        let restoring = self.restore.take(&panel);
        let old = self.list.read(cx).delegate().selected_session.clone();
        let desired = if restoring { self.saved_selected.as_ref().map(|s| SharedString::from(s.clone())) } else if reset { None } else { old };
        let selected = selected_index(&groups, runs, if reset { None } else { desired.as_ref() })
            .and_then(|index| index_path(&groups, index));
        let valid: HashSet<_> = runs.iter().map(|r| r.session.clone()).collect();
        let next_session = selected.and_then(|ix| groups.get(ix.section)?.runs.get(ix.row)).and_then(|&i| runs.get(i)).map(|run| run.session.clone());
        let restored = restoring && desired.is_some() && desired == next_session;
        let previous = self.list.read(cx).delegate().selected_session.clone();
        self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            delegate.saving = false;
            delegate.groups = groups;
            delegate.hover_glows.retain(|key| valid.contains(key));
            delegate.select_glows.retain(|key| valid.contains(key));
            delegate.hovered.retain(|key| valid.contains(key));
            list.set_selected_index(selected, window, cx);
            list.delegate_mut().saving = self.restore.done();
            if reset && selected.is_some() { list.scroll_to_item(IndexPath::default(), ScrollStrategy::Top, window, cx); }
            cx.notify();
        });
        if !reset && !restoring && self.restore.done() && previous != next_session {
            let selected = next_session.as_ref().map(ToString::to_string);
            if settings::view_memory(cx, "wishes").selected != selected { settings::remember_view("wishes", |v| v.selected = selected, cx); }
        }
        if restoring {
            if !restored {
                if self.saved_selected.take().is_some() { settings::remember_view("wishes", |v| v.selected = None, cx); }
                self.scroll_ready.set(true);
            } else {
                let handle = self.list.read(cx).scroll_handle().base_handle().clone();
                let offset = self.saved_scroll;
                let ready = self.scroll_ready.clone();
                let pending = self.restore_pending.clone();
                pending.set(true);
                cx.on_next_frame(window, move |_, window, cx| {
                    cx.on_next_frame(window, move |this, _, cx| {
                        if !pending.replace(false) { return; }
                        if this.list.read(cx).delegate().selected_session != next_session { ready.set(true); return; }
                        let mut point = handle.offset(); point.y = -px(offset); handle.set_offset(point);
                        ready.set(true); cx.notify();
                    });
                });
            }
        }
        if reset {
            let selected = self.list.read(cx).delegate().selected_session.as_ref().map(ToString::to_string);
            if settings::view_memory(cx, "wishes").selected != selected { settings::remember_view("wishes", |v| v.selected = selected, cx); }
        }
        cx.notify();
    }

    fn remember_filter(&self, cx: &mut Context<Self>) {
        let encoded = logic::encode(self.filter, &self.query);
        let memory = settings::view_memory(cx, "wishes");
        if memory.filter.as_deref() != Some(&encoded) || memory.sort.is_some() {
            settings::remember_view("wishes", |v| { v.filter = Some(encoded); v.sort = None; }, cx);
        }
    }

    fn set_filter(&mut self, filter: Filter, window: &mut Window, cx: &mut Context<Self>) {
        if !logic::cancels_restore(self.filter, filter, &self.query, &self.query) { return; }
        self.filter = filter;
        self.restore.cancel(); self.restore_pending.set(false); self.scroll_ready.set(true);
        self.remember_filter(cx);
        self.apply(true, window, cx);
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !logic::cancels_restore(self.filter, self.filter, &self.query, "") {
            self.focus_search(&FocusSearch, window, cx);
            return;
        }
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.query = SharedString::default();
        self.terms.clear();
        self.restore.cancel(); self.restore_pending.set(false); self.scroll_ready.set(true); self.remember_filter(cx);
        self.apply(true, window, cx);
        self.focus_search(&FocusSearch, window, cx);
    }

    fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn row_count(&self, cx: &App) -> usize {
        self.list.read(cx).delegate().groups.iter().map(|group| group.runs.len()).sum()
    }

    fn filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let d = theme::density(cx);
        h_flex()
            .gap(theme::space(d, Step::Xs))
            .p(theme::space(d, Step::Xs))
            .rounded(theme::RADIUS)
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .children(Filter::ALL.into_iter().zip(self.counts).enumerate().map(
                |(i, (filter, count))| {
                    let active = filter == self.filter;
                    Button::new(("wish-filter", i))
                        .ghost()
                        .small()
                        .selected(active)
                        .text_color(if active { t.text } else { t.muted })
                        .when(active, |this| this.bg(t.raised))
                        .label(tr(cx, T, filter.label()))
                        .child(
                            div()
                                .text_size(theme::font(d, TypeSize::Caption))
                                .text_color(if active { t.gold } else { t.muted })
                                .child(count.to_string()),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_filter(filter, window, cx);
                        }))
                },
            ))
    }

    fn nothing_yet(&self, cx: &mut Context<Self>) -> Empty {
        let state = self.state.clone();
        let blocker = wish_blocker(&self.state.read(cx).checks)
            .map(|check| SharedString::from(format!("{}: {}", check.name, check.detail)));
        let t = cx.tokens();
        let button = Button::new("wishes-make-wish")
            .primary()
            .large()
            .icon(IconName::Plus)
            .label(tr(cx, T, "Make a wish"))
            .disabled(blocker.is_some())
            .on_click(move |_, _, cx| state.update(cx, |state, cx| state.set_composing(true, cx)));
        let action = v_flex()
            .items_center()
            .gap_2()
            .child(theme::gold_leaf(button, &t))
            .when_some(blocker, |this, reason| {
                this.child(div().text_xs().text_color(t.review).child(reason))
            })
            .into_any_element();
        empty_state(
            IconName::Inbox,
            "No wishes yet",
            tr(cx, T, "Every wish you make appears here with its status, progress and cost."),
            Some(action),
            T,
            cx,
        )
    }

    fn nothing_matches(&self, cx: &mut Context<Self>) -> Empty {
        if self.terms.is_empty() {
            let action = Button::new("wishes-show-all")
                .label(tr(cx, T, "Show all"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_filter(Filter::All, window, cx);
                    this.focus_list(window, cx);
                }))
                .into_any_element();
            let hint = tr(cx, T, "Wishes that match this filter will appear here.");
            empty_state(IconName::Inbox, self.filter.empty_title(), hint, Some(action), T, cx)
        } else {
            let action = Button::new("wishes-clear-search")
                .label(tr(cx, T, "Clear search"))
                .on_click(cx.listener(|this, _, window, cx| this.clear_search(window, cx)))
                .into_any_element();
            let hint = tr(cx, T, "Search looks in the wish text and the project name.");
            empty_state(IconName::Search, "No matching wishes", hint, Some(action), T, cx)
        }
    }

    fn cursor_menu_position(&self, session: &SharedString, cx: &App) -> Option<Point<Pixels>> {
        let scroll = -f32::from(self.list.read(cx).scroll_handle().base_handle().offset().y);
        if (scroll - self.painted_scroll.get()).abs() > 0.5 { return None; }
        let bounds = self.bounds.borrow().get(session).copied()?.intersect(&self.viewport.get());
        (bounds.size.width > px(0.) && bounds.size.height > px(0.)).then(|| point(bounds.origin.x + bounds.size.width / 2., bounds.origin.y + bounds.size.height / 2.))
    }

    fn dispatch_cursor_menu(&mut self, session: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.read(cx).focus_handle(cx).is_focused(window) || self.list.read(cx).delegate().selected_session.as_ref() != Some(&session) { return; }
        let Some(position) = self.cursor_menu_position(&session, cx) else { return };
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, click_count: 1, ..Default::default() }), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, click_count: 1, ..Default::default() }), cx);
    }

    fn open_cursor_menu(&mut self, _: &palette::ContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.read(cx).focus_handle(cx).is_focused(window) { return; }
        let Some(session) = self.list.read(cx).delegate().selected_session.clone() else { return };
        cx.stop_propagation();
        if self.cursor_menu_position(&session, cx).is_some() {
            cx.defer_in(window, move |this, window, cx| this.dispatch_cursor_menu(session, window, cx));
        } else {
            self.list.update(cx, |list, cx| list.scroll_to_selected_item(window, cx));
            cx.on_next_frame(window, move |_, window, cx| {
                cx.on_next_frame(window, move |this, window, cx| this.dispatch_cursor_menu(session, window, cx));
            });
        }
    }

    fn retry_note(&self, message: SharedString, cx: &Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        h_flex().w_full().items_center().gap(theme::space(theme::density(cx), Step::Sm))
            .child(ornament::stale_note(&cx.tokens(), message))
            .child(Button::new("wishes-retry").ghost().small().label(tr(cx, T, "Try again"))
                .on_click(move |_, _, cx| state.update(cx, |state, cx| state.retry(FeedKind::Runs, cx))))
    }

    fn move_selection(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let delegate = self.list.read(cx).delegate();
        let current = delegate.selected.and_then(|ix| delegate.groups.get(ix.section)?.runs.get(ix.row).copied());
        let target = step_cards(&ledger_rows(&delegate.groups), current, step)
            .and_then(|index| index_path(&delegate.groups, index));
        if let Some(ix) = target {
            select_ix(&self.list, ix, window, cx);
        }
    }

    fn first_row(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MIN, window, cx);
    }

    fn last_row(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MAX, window, cx);
    }

    fn page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-(PAGE_ROWS as isize), window, cx);
    }

    fn page_down(&mut self, _: &SelectPageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(PAGE_ROWS as isize, window, cx);
    }

}

fn fill_for_stress(state: &mut AppState) {
    let base = std::mem::take(&mut state.runs);
    state.runs = (0..6000)
        .map(|i| {
            let mut run = base[i % base.len()].clone();
            run.wish = format!("{} #{i}", run.wish).into();
            run.started = format!(
                "{} {}, {:02}:{:02}",
                MONTHS[i % 11],
                i % 28 + 1,
                i % 24,
                i % 60
            )
            .into();
            run
        })
        .collect();
    state.current = Some(0);
}

impl Render for WishesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let placeholder = tr(cx, T, "Search wishes and projects");
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.search
                .update(cx, |search, cx| search.set_placeholder(placeholder, window, cx));
        }
        let state = self.state.read(cx);
        let no_runs = state.runs.is_empty();
        let panel = panel_state(&state.feeds.runs, no_runs);
        let view = logic::screen_state(&panel);
        let no_rows = self.row_count(cx) == 0;
        let d = theme::density(cx);
        let handle = self.list.read(cx).scroll_handle().base_handle().clone();
        let last = self.last_scroll.clone();
        let ready = self.scroll_ready.clone();
        let viewport = self.viewport.clone();
        let painted_scroll = self.painted_scroll.clone();
        let row_bounds = self.bounds.clone();
        let body = v_flex().w_full().max_w(theme::READ_WIDTH * 2.0).flex_1().min_h_0().gap(theme::space(d, Step::Md))
            .when(view == logic::ScreenState::Loading, |this| this.child(ornament::empty_state(&t, tr(cx, T, "Loading wishes").into(), "".into()).size_full()))
            .when(matches!(view, logic::ScreenState::Failed | logic::ScreenState::Stale), |this| {
                let message = match &panel { PanelState::Failed(message) | PanelState::Stale(message) => message.clone(), _ => unreachable!() };
                this.child(self.retry_note(message, cx))
            })
            .when(matches!(view, logic::ScreenState::Empty | logic::ScreenState::Ready | logic::ScreenState::Stale), |this| {
                this.when(no_runs, |this| this.child(self.nothing_yet(cx)))
                    .when(!no_runs, |this| this.child(h_flex().flex_none().justify_between().gap(theme::space(d, Step::Md))
                        .child(self.filter_bar(cx))
                        .child(div().w(column_width(d, 9.0)).flex_shrink(1.)
                            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| { this.focus_list(window, cx); cx.stop_propagation(); }))
                            .child(Input::new(&self.search).small().cleanable(true).prefix(Icon::new(IconName::Search).size(theme::font(d, TypeSize::Body)).text_color(t.muted)))))
                        .when(no_rows, |this| this.child(self.nothing_matches(cx)))
                        .when(!no_rows, |this| this.child(div().key_context("WishList")
                            .on_action(cx.listener(Self::first_row)).on_action(cx.listener(Self::last_row))
                            .on_action(cx.listener(Self::page_up)).on_action(cx.listener(Self::page_down))
                            .on_action(cx.listener(Self::open_cursor_menu))
                            // List-only guards must not swallow a descendant popup's keys.
                            .capture_action(cx.listener(|this, _: &Cancel, window, cx| {
                                if this.list.read(cx).focus_handle(cx).is_focused(window) { cx.stop_propagation(); }
                            }))
                            .capture_action(cx.listener(|this, action: &Confirm, window, cx| {
                                if action.secondary && this.list.read(cx).focus_handle(cx).is_focused(window) { cx.stop_propagation(); }
                            }))
                            .flex_1().min_h_0()
                            // Clear before the List's row callbacks record this frame's bounds.
                            .on_prepaint(move |bounds, _, cx| {
                                viewport.set(bounds);
                                row_bounds.borrow_mut().clear();
                                painted_scroll.set(-f32::from(handle.offset().y));
                                if ready.get() {
                                    let value = logic::scroll(-f32::from(handle.offset().y));
                                    if (value - last.get()).abs() > 1.0 {
                                        last.set(value); settings::remember_view("wishes", |v| v.scroll = value, cx);
                                    }
                                }
                            }).child(List::new(&self.list).size_full()))))
            });
        v_flex().key_context("WishesScreen").on_action(cx.listener(Self::focus_search))
            .size_full().min_h_0().pt(theme::space(d, Step::Lg)).px(theme::space(d, Step::Xl)).gap(theme::space(d, Step::Lg))
            .child(ornament::page_title(&t, tr(cx, T, "Wishes")).id("wishes-page-title").flex_none())
            .child(body)

    }
}

#[cfg(test)]
mod tests {
    // Named, because a glob import also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use super::super::demo::sample_run;
    use super::*;

    fn run(wish: &str, project: &str, status: RunStatus, started: &str) -> Run {
        let mut run = sample_run();
        run.wish = wish.to_owned().into();
        run.project = project.to_owned().into();
        run.status = status;
        run.started = started.to_owned().into();
        run
    }

    fn sample_runs() -> Vec<Run> {
        vec![
            run("Older run", "alpha", RunStatus::Active, "Nov 3, 05:15"),
            run("把批次改掉", "rram-batch", RunStatus::Waiting, "Nov 4, 12:50"),
            run("Migrate billing", "billing", RunStatus::Blocked, "Nov 2, 19:33"),
            run("Done one", "chronos-r5", RunStatus::Done, "Nov 3, 00:42"),
        ]
    }

    #[test]
    fn grouping_covers_every_run_in_all_six_states_and_preserves_input_order() {
        let statuses = [RunStatus::Done, RunStatus::Waiting, RunStatus::Blocked,
            RunStatus::Active, RunStatus::LimitWait, RunStatus::Interrupted, RunStatus::Active];
        let runs: Vec<_> = statuses.into_iter().map(|status| run("wish", "project", status, "Nov 1, 00:00")).collect();
        let groups = group_runs(&runs, &[6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(groups.iter().map(|g| g.status).collect::<Vec<_>>(), GROUP_ORDER);
        assert_eq!(groups.iter().map(|g| g.runs.clone()).collect::<Vec<_>>(),
            vec![vec![2], vec![6, 3], vec![1], vec![4], vec![5], vec![0]]);
        let mut all: Vec<_> = groups.iter().flat_map(|g| g.runs.iter().copied()).collect();
        all.sort_unstable();
        assert_eq!(all, (0..runs.len()).collect::<Vec<_>>());
    }

    #[test]
    fn grouping_follows_filter_and_search_and_omits_empty_groups() {
        let runs = sample_runs();
        let visible = visible_runs(&runs, Filter::Running, &search_terms("rram"), 11);
        assert_eq!(group_runs(&runs, &visible), vec![RunGroup { status: RunStatus::Waiting, runs: vec![1] }]);
        assert!(group_runs(&runs, &[]).is_empty());
    }

    #[test]
    fn movement_skips_dividers_and_clamps_to_cards() {
        let groups = group_runs(&sample_runs(), &[1, 0, 3, 2]);
        let rows = ledger_rows(&groups);
        assert_eq!(rows[0], LedgerRow::Divider { status: RunStatus::Blocked, count: 1 });
        assert_eq!(step_cards(&rows, Some(2), 1), Some(0));
        assert_eq!(step_cards(&rows, Some(0), -1), Some(2));
        assert_eq!(step_cards(&rows, Some(1), isize::MIN), Some(2));
        assert_eq!(step_cards(&rows, Some(1), isize::MAX), Some(3));
        assert_eq!(step_cards(&[], None, 1), None);
    }

    #[test]
    fn selection_follows_session_across_regroup() {
        let mut runs = sample_runs();
        for (i, run) in runs.iter_mut().enumerate() { run.session = format!("session-{i}").into(); }
        let before = group_runs(&runs, &[1, 0, 3, 2]);
        let session = runs[0].session.clone();
        assert_eq!(index_path(&before, selected_index(&before, &runs, Some(&session)).unwrap()), Some(IndexPath::new(0).section(1)));
        runs[0].status = RunStatus::Done;
        let after = group_runs(&runs, &[1, 0, 3, 2]);
        let selected = selected_index(&after, &runs, Some(&session));
        assert_eq!(selected, Some(0));
        assert_eq!(index_path(&after, selected.unwrap()), Some(IndexPath::new(0).section(2)));
    }

    #[test]
    fn newest_first() {
        let runs = sample_runs();
        assert_eq!(visible_runs(&runs, Filter::All, &[], 11), vec![1, 0, 3, 2]);
    }

    #[test]
    fn ties_and_unreadable_times_keep_their_order_after_readable_ones() {
        let mut runs = sample_runs();
        runs[0].started = "".into();
        runs[1].started = "yesterday".into();
        runs[2].started = "Nov 3, 00:42".into();
        assert_eq!(visible_runs(&runs, Filter::All, &[], 11), vec![2, 3, 0, 1]);
    }

    #[test]
    fn december_sorts_below_january_early_in_the_year() {
        let runs = vec![
            run("last year", "a", RunStatus::Done, "Dec 30, 09:00"),
            run("this year", "a", RunStatus::Done, "Jan 2, 09:00"),
        ];
        assert_eq!(visible_runs(&runs, Filter::All, &[], 1), vec![1, 0]);
        assert_eq!(visible_runs(&runs, Filter::All, &[], 12), vec![0, 1]);
    }

    #[test]
    fn filters_group_statuses() {
        let runs = sample_runs();
        assert_eq!(visible_runs(&runs, Filter::Running, &[], 11), vec![1, 0]);
        assert_eq!(visible_runs(&runs, Filter::NeedsYou, &[], 11), vec![2]);
        assert_eq!(visible_runs(&runs, Filter::Done, &[], 11), vec![3]);
    }

    #[test]
    fn counts_follow_the_search() {
        let runs = sample_runs();
        assert_eq!(filter_counts(&runs, &[]), [4, 2, 1, 1]);
        assert_eq!(filter_counts(&runs, &search_terms("BILLING")), [1, 0, 1, 0]);
        assert_eq!(filter_counts(&runs, &search_terms("zzz")), [0, 0, 0, 0]);
    }

    #[test]
    fn search_covers_wish_and_project_case_insensitively() {
        let runs = sample_runs();
        assert_eq!(visible_runs(&runs, Filter::All, &search_terms("MIGRATE"), 11), vec![2]);
        assert_eq!(visible_runs(&runs, Filter::All, &search_terms("rram"), 11), vec![1]);
        assert_eq!(visible_runs(&runs, Filter::All, &search_terms("批次"), 11), vec![1]);
    }

    #[test]
    fn every_term_must_match_somewhere() {
        let runs = sample_runs();
        assert_eq!(visible_runs(&runs, Filter::All, &search_terms("billing migrate"), 11), vec![2]);
        assert!(visible_runs(&runs, Filter::All, &search_terms("billing done"), 11).is_empty());
    }

    #[test]
    fn blank_search_matches_everything() {
        let runs = sample_runs();
        assert_eq!(visible_runs(&runs, Filter::All, &search_terms("   "), 11).len(), 4);
    }

    #[test]
    fn start_times_read_naturally_in_chinese() {
        let started = SharedString::from("Nov 3, 05:15");
        assert_eq!(started_label(&started, Lang::ZhTw).as_ref(), "11月3日 05:15");
        assert_eq!(started_label(&started, Lang::En).as_ref(), "Nov 3, 05:15");
        let odd = SharedString::from("sometime");
        assert_eq!(started_label(&odd, Lang::ZhTw).as_ref(), "sometime");
    }

    #[test]
    fn unknown_figures_show_a_dash() {
        assert_eq!(progress_label(0, 0), "—");
        assert_eq!(progress_label(7, 9), "7 / 9");
        assert_eq!(cost_label(None), "—");
        assert_eq!(cost_label(Some(116.12)), "$116.12");
    }

    #[test]
    fn resume_is_offered_only_when_a_run_is_stopped_or_waiting() {
        assert!(can_resume(RunStatus::Blocked));
        assert!(can_resume(RunStatus::Waiting));
        assert!(!can_resume(RunStatus::Active));
        assert!(!can_resume(RunStatus::Done));
    }

    #[test]
    fn multi_line_wishes_flatten() {
        let wish = SharedString::from("first line\n  second\tpart");
        assert_eq!(one_line(&wish).as_ref(), "first line second part");
    }

    #[test]
    fn only_missing_claude_or_codex_blocks_new_wishes() {
        let check = |name: &'static str, status| Check {
            name: name.into(),
            status,
            detail: "".into(),
        };
        assert!(wish_blocker(&[check("codegraph", CheckStatus::Fail)]).is_none());
        assert!(wish_blocker(&[check("claude", CheckStatus::Warn)]).is_none());
        assert_eq!(
            wish_blocker(&[check("codex", CheckStatus::Fail)]).unwrap().name.as_ref(),
            "codex"
        );
    }

    #[test]
    fn table_keys_are_unique() {
        let mut keys: Vec<&str> = T.iter().map(|(en, _)| *en).collect();
        keys.sort_unstable();
        let total = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), total);
    }
}


