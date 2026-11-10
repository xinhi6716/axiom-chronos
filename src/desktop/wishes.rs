//! Every run, newest first: filter, search, open. Filtering and sorting are pure and sit apart from drawing.
use super::i18n::{tr, Lang, Table};
use super::model::*;
use super::{ornament, theme};
use super::theme::Tokened as _;
use gpui_kit::base::actions::{SelectFirst, SelectLast, SelectPageDown, SelectPageUp};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, Disableable as _, Icon, IconName, IndexPath, Selectable as _,
    Sizable as _, StyledExt as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cmp::Reverse;
use std::path::PathBuf;

gpui_kit::actions!(chronos_lists, [FocusSearch]);

const T: Table = &[
    ("Wishes", "願望"),
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
    let indices = groups.iter().flat_map(|group| group.runs.iter().copied());
    session.and_then(|session| indices.clone().find(|&i| runs[i].session == *session))
        .or_else(|| indices.into_iter().next())
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
const ROW_HEIGHT: Pixels = px(60.);
const PROJECT_WIDTH: Pixels = px(132.);
const COST_WIDTH: Pixels = px(72.);
const STARTED_WIDTH: Pixels = px(104.);
const MENU_WIDTH: Pixels = px(24.);

fn cell(width: Pixels) -> Div {
    div().flex_none().w(width).min_w_0()
}

fn glyph_state(status: RunStatus, verified_end: bool) -> UnitState {
    match status {
        RunStatus::Active => UnitState::Running,
        RunStatus::Done if verified_end => UnitState::Verified,
        RunStatus::Done | RunStatus::Blocked => UnitState::Built,
        RunStatus::Waiting | RunStatus::LimitWait => UnitState::Pending,
        RunStatus::Interrupted => UnitState::Failed,
    }
}

#[derive(Clone)]
struct RowActions {
    state: Entity<AppState>,
    index: usize,
    status: RunStatus,
    session: SharedString,
    cwd: Option<PathBuf>,
}

impl RowActions {
    fn add_to(&self, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let open_folder = self.cwd.clone().filter(|cwd| cwd.is_dir());
        menu.min_w(px(200.))
            .item({
                let actions = self.clone();
                PopupMenuItem::new(tr(cx, T, "Resume"))
                    .icon(IconName::Play)
                    .disabled(!can_resume(self.status))
                    .on_click(move |_, _, cx| {
                        actions.state.update(cx, |state, cx| {
                            state.open_run(actions.index, cx);
                            state.resume_current(None, cx);
                        });
                    })
            })
            .item(
                PopupMenuItem::new(tr(cx, T, "Open folder"))
                    .icon(IconName::FolderOpen)
                    .disabled(open_folder.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(cwd) = &open_folder {
                            cx.open_with_system(cwd);
                        }
                    }),
            )
            .item({
                let session = self.session.clone();
                PopupMenuItem::new(tr(cx, T, "Copy session id"))
                    .icon(IconName::Copy)
                    .disabled(session.is_empty())
                    .on_click(move |_, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(session.to_string()));
                        let done = tr(cx, T, "Session id copied");
                        window.push_notification(Notification::success(done), cx);
                    })
            })
    }
}

struct RunList {
    state: Entity<AppState>,
    groups: Vec<RunGroup>,
    selected: Option<IndexPath>,
    selected_session: Option<SharedString>,
    focus: Option<FocusHandle>,
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
        Some(h_flex().h(px(40.)).items_center().child(ornament::section_label(&cx.tokens(), &label)))
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
        let actions = RowActions {
            state: self.state.clone(),
            index,
            status,
            session: run.session.clone(),
            cwd: run.cwd.clone(),
        };
        let name = format!("{} {project} {wish}", tr(cx, T, status.label()));
        let context_actions = actions.clone();

        Some(
            ListItem::new(ix)
                .accessibility_label(name)
                .h(ROW_HEIGHT)
                .py_0()
                .px_0()
                .cursor_pointer()
                .child(
                    h_flex()
                        .id(("wish-row", index))
                        .relative()
                        .w_full()
                        .h(ROW_HEIGHT - px(6.))
                        .mb(px(6.))
                        .rounded(px(8.))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .hover(|style| style.shadow(ornament::gold_glow(&t, 0.7)))
                        .px_3()
                        .gap_3()
                        .context_menu(move |menu, _, cx| context_actions.add_to(menu, cx))
                        .when(focused, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_2()
                                    .bottom_2()
                                    .w(px(3.))
                                    .rounded_full()
                                    .bg(t.gold),
                            )
                        })
                        .child(ornament::glyph(glyph_state(status, verified_end), 20.0, t.outcome(status, verified_end), 0.0))
                        .when_some(ornament::progress_ring(&t, verified, total, 24.0), |this, ring| this.child(ring))
                        .child(
                            cell(PROJECT_WIDTH)
                                .id(("wish-project", index))
                                .px_2()
                                .py_0p5()
                                .rounded(px(6.))
                                .bg(t.raised)
                                .text_sm()
                                .text_color(t.muted)
                                .truncate()
                                .child(project)
                                .tooltip(move |window, cx| {
                                    Tooltip::new(place.clone()).build(window, cx)
                                }),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1p5()
                                .child(
                                    div()
                                        .id(("wish-text", index))
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(t.text)
                                        .child(wish.clone())
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(full_wish.clone()).build(window, cx)
                                        }),
                                )
                                .when_some(warning, |this, warning| {
                                    this.child(
                                        div()
                                            .id(("wish-warning", index))
                                            .flex_none()
                                            .text_color(t.review)
                                            .child(
                                                Icon::new(IconName::TriangleAlert).size(px(14.)),
                                            )
                                            .tooltip(move |window, cx| {
                                                Tooltip::new(warning.clone()).build(window, cx)
                                            }),
                                    )
                                }),
                        )
                        .child(
                            cell(COST_WIDTH)
                                .text_right()
                                .text_sm()
                                .text_color(t.muted)
                                .child(cost_label(cost)),
                        )
                        .child(
                            cell(STARTED_WIDTH)
                                .text_right()
                                .text_xs()
                                .text_color(t.muted)
                                .truncate()
                                .child(started),
                        )
                        .child(
                            cell(MENU_WIDTH).child(
                                Button::new(("wish-menu", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Ellipsis)
                                    .tab_stop(selected)
                                    .accessibility_label(tr(cx, T, "More actions"))
                                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                                        actions.add_to(menu, cx)
                                    }),
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
        self.selected = ix;
        self.selected_session = ix
            .and_then(|ix| self.groups.get(ix.section)?.runs.get(ix.row))
            .and_then(|&index| self.state.read(cx).runs.get(index))
            .map(|run| run.session.clone());
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
    _subscriptions: Vec<Subscription>,
}

impl WishesView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if state.read(cx).scene.as_deref() == Some("many") {
            state.update(cx, |state, _| fill_for_stress(state));
        }
        bind_list_keys("WishList", cx);
        cx.bind_keys([KeyBinding::new("ctrl-f", FocusSearch, Some("WishesScreen"))]);

        let placeholder = tr(cx, T, "Search wishes and projects");
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder(placeholder)
        });
        let list = cx.new(|cx| {
            ListState::new(
                RunList {
                    state: state.clone(),
                    groups: Vec::new(),
                    selected: None,
                    selected_session: None,
                    focus: None,
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
            filter: Filter::All,
            query: SharedString::default(),
            terms: Vec::new(),
            counts: [0; 4],
            on_screen,
            _subscriptions,
        };
        this.apply(true, window, cx);
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
                if value != self.query {
                    self.terms = search_terms(&value);
                    self.query = value;
                    self.apply(true, window, cx);
                }
            }
            InputEvent::PressEnter { .. } => self.focus_list(window, cx),
            _ => {}
        }
    }

    fn apply(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let runs = &self.state.read(cx).runs;
        self.counts = filter_counts(runs, &self.terms);
        let visible = visible_runs(runs, self.filter, &self.terms, this_month());
        let groups = group_runs(runs, &visible);
        let session = if reset { None } else { self.list.read(cx).delegate().selected_session.as_ref() };
        let selected = selected_index(&groups, runs, session).and_then(|index| index_path(&groups, index));
        self.list.update(cx, |list, cx| {
            list.delegate_mut().groups = groups;
            list.set_selected_index(selected, window, cx);
            if reset && selected.is_some() {
                list.scroll_to_item(IndexPath::default(), ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn set_filter(&mut self, filter: Filter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = filter;
        self.apply(true, window, cx);
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.query = SharedString::default();
        self.terms.clear();
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
        h_flex()
            .gap_1()
            .p_0p5()
            .rounded(px(8.))
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
                                .text_xs()
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
        let no_runs = self.state.read(cx).runs.is_empty();
        let no_rows = self.row_count(cx) == 0;

        v_flex()
            .key_context("WishesScreen")
            .on_action(cx.listener(Self::focus_search))
            .size_full()
            .min_h_0()
            .pt_5()
            .px_6()
            .gap_4()
            .child(ornament::page_title(&t, tr(cx, T, "Wishes")).flex_none())
            .when(no_runs, |this| this.child(self.nothing_yet(cx)))
            .when(!no_runs, |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .justify_between()
                        .gap_4()
                        .child(self.filter_bar(cx))
                        .child(
                            div()
                                .w(px(280.))
                                .flex_shrink(1.)
                                .capture_action(cx.listener(
                                    |this, _: &MoveDown, window, cx| {
                                        this.focus_list(window, cx);
                                        cx.stop_propagation();
                                    },
                                ))
                                .child(
                                    Input::new(&self.search).small().cleanable(true).prefix(
                                        Icon::new(IconName::Search)
                                            .size(px(14.))
                                            .text_color(t.muted),
                                    ),
                                ),
                        ),
                )
                .when(no_rows, |this| this.child(self.nothing_matches(cx)))
                .when(!no_rows, |this| {
                    this.child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .key_context("WishList")
                                    .on_action(cx.listener(Self::first_row))
                                    .on_action(cx.listener(Self::last_row))
                                    .on_action(cx.listener(Self::page_up))
                                    .on_action(cx.listener(Self::page_down))
                                    .flex_1()
                                    .min_h_0()
                                    .child(List::new(&self.list).size_full()),
                            ),
                    )
                })
            })
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
