//! Every run, newest first: filter, search, open. Filtering and sorting are pure and sit apart from drawing.
use super::i18n::{tr, Lang, Table};
use super::model::*;
use super::theme::{Tokened as _, Tokens};
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
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Selectable as _,
    Sizable as _, StyledExt as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cmp::Reverse;
use std::path::PathBuf;

gpui_kit::actions!(chronos_lists, [FocusSearch]);

const T: Table = &[
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
            Filter::Running => matches!(status, RunStatus::Active | RunStatus::Waiting),
            Filter::NeedsYou => status == RunStatus::Blocked,
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
    matches!(status, RunStatus::Blocked | RunStatus::Waiting)
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

const PAGE_ROWS: usize = 10;
const ROW_HEIGHT: Pixels = px(44.);
const STATUS_WIDTH: Pixels = px(120.);
const PROJECT_WIDTH: Pixels = px(132.);
const PROGRESS_WIDTH: Pixels = px(68.);
const COST_WIDTH: Pixels = px(72.);
const STARTED_WIDTH: Pixels = px(104.);
const MENU_WIDTH: Pixels = px(24.);

fn cell(width: Pixels) -> Div {
    div().flex_none().w(width).min_w_0()
}

fn status_icon(status: RunStatus) -> IconName {
    match status {
        RunStatus::Active => IconName::LoaderCircle,
        RunStatus::Waiting => IconName::Pause,
        RunStatus::Blocked => IconName::CircleAlert,
        RunStatus::Done => IconName::CircleCheck,
    }
}

fn status_cell(status: RunStatus, cx: &App) -> impl IntoElement {
    let color = cx.tokens().status(status);
    h_flex()
        .gap_1p5()
        .text_xs()
        .text_color(color)
        .child(Icon::new(status_icon(status)).size(px(14.)))
        .child(div().truncate().child(tr(cx, T, status.label())))
}

fn progress_color(status: RunStatus, verified: usize, total: usize, t: &Tokens) -> Hsla {
    if total > 0 && verified == total {
        t.ok
    } else if total > 0 && status == RunStatus::Done {
        t.review
    } else {
        t.muted
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
    rows: Vec<usize>,
    selected: Option<usize>,
    selected_session: Option<SharedString>,
    focus: Option<FocusHandle>,
}

impl ListDelegate for RunList {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.rows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let index = *self.rows.get(ix.row)?;
        let t = cx.tokens();
        let lang = *cx.global::<Lang>();
        let run = self.state.read(cx).runs.get(index)?;
        let selected = self.selected == Some(ix.row);
        let focused = selected && self.focus.as_ref().is_some_and(|f| f.is_focused(window));
        let (verified, total) = (run.count(UnitState::Verified), run.units.len());
        let status = run.status;
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
            ListItem::new(ix.row)
                .accessibility_label(name)
                .h(ROW_HEIGHT)
                .py_0()
                .px_0()
                .border_b_1()
                .border_color(t.border.opacity(0.45))
                .cursor_pointer()
                .child(
                    h_flex()
                        .id(("wish-row", ix.row))
                        .relative()
                        .w_full()
                        .h(ROW_HEIGHT - px(1.))
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
                        .child(cell(STATUS_WIDTH).child(status_cell(status, cx)))
                        .child(
                            cell(PROJECT_WIDTH)
                                .id(("wish-project", ix.row))
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
                                        .id(("wish-text", ix.row))
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(t.text)
                                        .child(wish.clone())
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(wish.clone()).build(window, cx)
                                        }),
                                )
                                .when_some(warning, |this, warning| {
                                    this.child(
                                        div()
                                            .id(("wish-warning", ix.row))
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
                            cell(PROGRESS_WIDTH)
                                .text_right()
                                .text_sm()
                                .text_color(progress_color(status, verified, total, &t))
                                .child(progress_label(verified, total)),
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
                                Button::new(("wish-menu", ix.row))
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
        self.selected = ix.map(|ix| ix.row);
        self.selected_session = ix
            .and_then(|ix| self.rows.get(ix.row))
            .and_then(|&index| self.state.read(cx).runs.get(index))
            .map(|run| run.session.clone());
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(&index) = self.selected.and_then(|row| self.rows.get(row)) {
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
                    rows: Vec::new(),
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
        let rows = visible_runs(runs, self.filter, &self.terms, this_month());
        let keep = if reset {
            None
        } else {
            let delegate = self.list.read(cx).delegate();
            delegate.selected_session.as_ref().and_then(|session| {
                rows.iter().position(|&i| runs[i].session == *session)
            })
        };
        let selected = keep.or((!rows.is_empty()).then_some(0));
        self.list.update(cx, |list, cx| {
            list.delegate_mut().rows = rows;
            list.set_selected_index(selected.map(IndexPath::new), window, cx);
            if reset {
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
        self.list.read(cx).delegate().rows.len()
    }

    fn selected_row(&self, cx: &App) -> usize {
        self.list.read(cx).delegate().selected.unwrap_or(0)
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

    fn column_headers(&self, cx: &App) -> impl IntoElement {
        let t = cx.tokens();
        let label = |width: Pixels, en: &'static str, right: bool| {
            cell(width)
                .when(right, |this| this.text_right())
                .truncate()
                .child(tr(cx, T, en))
        };
        h_flex()
            .flex_none()
            .h(px(32.))
            .gap_3()
            .px(px(14.))
            .text_xs()
            .text_color(t.muted)
            .border_b_1()
            .border_color(t.border)
            .child(label(STATUS_WIDTH, "Status", false))
            .child(label(PROJECT_WIDTH, "Project", false))
            .child(div().flex_1().min_w_0().truncate().child(tr(cx, T, "Wish")))
            .child(label(PROGRESS_WIDTH, "Verified", true))
            .child(label(COST_WIDTH, "Cost", true))
            .child(label(STARTED_WIDTH, "Started", true))
            .child(cell(MENU_WIDTH))
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
            .child(button)
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

    fn move_selection(
        &mut self,
        row: impl FnOnce(usize, usize) -> usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.row_count(cx);
        if count > 0 {
            let target = row(self.selected_row(cx), count - 1);
            select_row(&self.list, target, window, cx);
        }
    }

    fn first_row(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(|_, _| 0, window, cx);
    }

    fn last_row(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(|_, last| last, window, cx);
    }

    fn page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(|row, _| row.saturating_sub(PAGE_ROWS), window, cx);
    }

    fn page_down(&mut self, _: &SelectPageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(|row, last| (row + PAGE_ROWS).min(last), window, cx);
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
                            .child(self.column_headers(cx))
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
