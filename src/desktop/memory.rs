//! What Chronos has learned from verified units: filter by project, search, read, correct, delete.
use super::i18n::{fill, tr, translate, Lang, Table};
use super::model::*;
use super::{motion, ornament, palette, settings, theme};
use super::theme::{Step, TypeSize};
use super::motion::{Driver, Pace};
use logic::{correction_problem, Corrections, Problem, MAX_CORRECTION_LINES};
mod logic;
#[cfg(test)] mod accept;
use super::theme::{gold_leaf, Tokened as _};
use super::wishes::{
    bind_list_keys, empty_state, matches_terms, one_line, search_terms, select_row, started_label,
};
use gpui_kit::base::actions::{SelectFirst, SelectLast, SelectPageDown, SelectPageUp};
use gpui_kit::base::SelectableText;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::empty::Empty;
use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, Disableable as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

gpui_kit::actions!(chronos_memory, [FocusMemorySearch, DeleteSelectedMemory]);

const T: Table = &[
    ("Memory copied", "已複製記憶"),
    ("More actions", "更多操作"),
    ("Loading memories", "正在載入記憶"),
    ("Try again", "重試"),
    ("Correction saved", "更正已儲存"),
    ("Undo", "復原"),
    ("Dismiss", "關閉"),
    ("All projects", "所有專案"),
    ("Search memories", "搜尋記憶"),
    ("Stale", "已過時"),
    ("Delete", "刪除"),
    ("This memory may be out of date", "這筆記憶可能已經過時"),
    (
        "A file it was recorded from has changed or been removed since. Chronos reads the current file instead of quoting this note in a brief.",
        "記錄時引用的檔案後來改過或已移除。派工時 Chronos 會改讀目前的檔案，不會引用這段內容。",
    ),
    ("Delete this memory?", "要刪除這筆記憶嗎？"),
    (
        "Later wishes will no longer be able to find it. This cannot be undone.",
        "之後的願望就找不到這筆記憶了。此動作無法復原。",
    ),
    ("No memories yet", "還沒有記憶"),
    (
        "When a wish is verified, Chronos notes what each unit built so later wishes can reuse it.",
        "每次願望驗收後，Chronos 會記下各單元做了什麼，讓之後的願望可以沿用。",
    ),
    ("No matching memories", "沒有符合的記憶"),
    (
        "Search looks in the title, the text and the project name.",
        "搜尋範圍是標題、內容與專案名稱。",
    ),
    ("Clear filters", "清除篩選"),
    ("Select a memory to read it", "選一筆記憶來閱讀"),
    ("What Chronos has learned", "Chronos 學到的事"),
    ("From the wish", "來自這個願望"),
    ("The wish was not recorded", "沒有記錄來源願望"),
    ("Last confirmed {}", "最近確認於 {}"),
    ("Last confirmed: not recorded", "最近確認時間：沒有記錄"),
    ("Corrected", "已更正"),
    ("A person rewrote this note. The new text is what later briefs see.", "這筆記憶的內容已由人修改，之後的派工看到的是新內容。"),
    ("Correct", "更正"),
    ("Save correction", "儲存更正"),
    ("Discard", "放棄修改"),
    ("Your correction replaces this note's text and is what later briefs see.", "更正後的文字會取代這筆記憶的內容，之後的派工看到的就是它。"),
    ("The text cannot be empty.", "內容不能是空的。"),
    ("The text is longer than {} lines.", "內容不能超過 {} 行。"),
    ("The text has not changed.", "內容還沒有修改。"),
    ("Correct this memory", "更正這筆記憶"),
];

/// "Last confirmed Nov 3, 05:15", written in the interface language.
fn confirmed_line(entry: &MemoryEntry, lang: Lang) -> String {
    if entry.confirmed.is_empty() {
        return translate(lang, T, "Last confirmed: not recorded").to_owned();
    }
    fill(translate(lang, T, "Last confirmed {}"), &[&started_label(&entry.confirmed, lang)])
}

/// Projects that have memories, by name, with how many each holds.
pub fn project_counts(memory: &[MemoryEntry]) -> Vec<(SharedString, usize)> {
    let mut counts: Vec<(SharedString, usize)> = Vec::new();
    let mut names: Vec<&SharedString> = memory.iter().map(|entry| &entry.project).collect();
    names.sort_unstable();
    for name in names {
        match counts.last_mut() {
            Some((last, count)) if last == name => *count += 1,
            _ => counts.push((name.clone(), 1)),
        }
    }
    counts
}

fn entry_matches(entry: &MemoryEntry, terms: &[String]) -> bool {
    matches_terms(&[&entry.title, &entry.body, &entry.project, &entry.wish], terms)
}

/// Indices into `memory` of the rows to show, most recently recorded first.
pub fn visible_memories(
    memory: &[MemoryEntry],
    project: Option<&str>,
    terms: &[String],
) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..memory.len())
        .filter(|&i| project.is_none_or(|project| memory[i].project.as_ref() == project))
        .filter(|&i| entry_matches(&memory[i], terms))
        .collect();
    rows.sort_by_key(|&i| Reverse(memory[i].id));
    rows
}

/// A row in the project menu; an empty `project` stands for every project.
#[derive(Clone, PartialEq)]
struct ProjectChoice {
    project: SharedString,
    title: SharedString,
    count: usize,
}

impl SelectItem for ProjectChoice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &SharedString {
        &self.project
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .gap(theme::space(theme::density(cx), Step::Md))
            .child(div().truncate().child(self.title.clone()))
            .child(
                div()
                    .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                    .text_color(cx.tokens().muted)
                    .child(self.count.to_string()),
            )
    }
}

fn project_choices(memory: &[MemoryEntry], cx: &App) -> Vec<ProjectChoice> {
    let all = ProjectChoice {
        project: SharedString::default(),
        title: tr(cx, T, "All projects").into(),
        count: memory.len(),
    };
    std::iter::once(all)
        .chain(project_counts(memory).into_iter().map(|(project, count)| ProjectChoice {
            title: project.clone(),
            project,
            count,
        }))
        .collect()
}

const PAGE_ROWS: usize = 8;

fn stale_tag(cx: &App) -> Tag {
    let t = cx.tokens();
    Tag::custom(t.review.opacity(0.14), t.review, t.review.opacity(0.4))
        .xsmall()
        .text_size(theme::font(theme::density(cx), TypeSize::Caption))
        .px_2()
        .flex_none()
        .rounded_full()
        .child(tr(cx, T, "Stale"))
}

struct MemoryList {
    state: Entity<AppState>,
    rows: Vec<usize>,
    selected: Option<usize>,
    selected_id: Option<i64>,
    focus: Option<FocusHandle>,
    hover_glows: motion::Glows<i64>,
    select_glows: motion::Glows<i64>,
    hovered: HashSet<i64>,
    bounds: Rc<RefCell<HashMap<i64, Bounds<Pixels>>>>,
    runner: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)>,
    saving: bool,
}

impl ListDelegate for MemoryList {
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
        let t = cx.tokens();
        let entry = self.state.read(cx).memory.get(*self.rows.get(ix.row)?)?;
        let id = entry.id;
        let selected = self.selected == Some(ix.row);
        let focused = self.focus.as_ref().is_some_and(|f| f.is_focused(window));
        let source = if entry.wish.trim().is_empty() { &entry.body } else { &entry.wish };
        let summary = format!("{} · {}", entry.project, one_line(source));
        let title = one_line(&entry.title);
        let stale = entry.stale;
        let d = theme::density(cx);
        let height = theme::row_height(d) * 2.0;
        let reduce = cx.reduce_motion();
        let hovered = self.hovered.contains(&id);
        let mut frames = Driver::new(reduce, cx);
        self.hover_glows.set(&id, if hovered { 1.0 } else { 0.0 }, frames.now(), reduce);
        self.select_glows.set(&id, if selected { 1.0 } else { 0.0 }, frames.now(), reduce);
        let (lift, glow, ring) = logic::levels(selected, hovered, focused, self.hover_glows.value(&id, &mut frames), self.select_glows.value(&id, &mut frames), reduce);
        frames.finish(window);
        let bounds = self.bounds.clone();
        let target = MenuTarget::Memory(id);
        let state = self.state.clone();
        let runner = self.runner.clone();
        Some(ListItem::new(ix.row).accessibility_label(title.clone()).disabled(true)
            .h(height).py(theme::space(d, Step::Xs)).px(theme::space(d, Step::Xs)).cursor_pointer()
            .child(ornament::raised_card(&t, lift, glow)
                .id(("memory-row", id as u64)).relative().top(-theme::space(d, Step::Xs) * (lift * 0.5))
                .w_full().h(height - theme::space(d, Step::Xs) * 2.0)
                .shadow({ let mut shadows = ornament::raised_card_shadows(&t, lift, glow); if ring { shadows.extend(theme::focus_ring(&t)); } shadows })
                .px(theme::space(d, Step::Md))
                .on_prepaint(move |rect, _, _| { bounds.borrow_mut().insert(id, rect); })
                .on_hover(cx.listener(move |list, inside: &bool, _, cx| { if *inside { list.delegate_mut().hovered.insert(id); } else { list.delegate_mut().hovered.remove(&id); } cx.notify(); }))
                .context_menu(move |menu, _, cx| palette::menu::context_menu(menu, target.clone(), state.clone(), runner.clone(), cx))
                .child(v_flex().w_full().h_full().justify_center().gap(theme::space(d, Step::Xs))
                    .child(h_flex().min_w_0().gap(theme::space(d, Step::Sm)).justify_between()
                        .child(div().min_w_0().truncate().text_size(theme::font(d, TypeSize::Body)).child(title))
                        .when(stale, |this| this.child(stale_tag(cx))))
                    .child(div().w_full().truncate().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted).child(summary)))))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let previous = self.selected_id;
        self.selected = ix.map(|ix| ix.row);
        self.selected_id = ix
            .and_then(|ix| self.rows.get(ix.row))
            .and_then(|&index| self.state.read(cx).memory.get(index))
            .map(|entry| entry.id);
        if self.saving && previous != self.selected_id {
            let selected = self.selected_id.map(|id| id.to_string());
            if settings::view_memory(cx, "memory").selected != selected { settings::remember_view("memory", |v| v.selected = selected, cx); }
        }
    }
}

pub struct MemoryView {
    state: Entity<AppState>,
    search: Entity<InputState>,
    placeholder: &'static str,
    project: Entity<SelectState<Vec<ProjectChoice>>>,
    choices: Vec<ProjectChoice>,
    project_filter: Option<SharedString>,
    list: Entity<ListState<MemoryList>>,
    query: SharedString,
    terms: Vec<String>,
    on_screen: bool,
    /// The correction box, shared by whichever entry is being corrected.
    edit: Entity<TextareaState>,
    editing: Option<i64>,
    /// Corrections not yet saved: set aside, or waiting for the engine.
    corrections: Corrections,
    restore: logic::Restore,
    saved_selected: Option<i64>,
    saved_scroll: f32,
    last_scroll: Rc<Cell<f32>>,
    scroll_ready: Rc<Cell<bool>>,
    restore_pending: Rc<Cell<bool>>,
    bounds: Rc<RefCell<HashMap<i64, Bounds<Pixels>>>>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    painted_scroll: Rc<Cell<f32>>,
    offer: logic::UndoOffer,
    undo_len: usize,
    _subscriptions: Vec<Subscription>,
}

impl MemoryView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if state.read(cx).scene.as_deref() == Some("many") {
            state.update(cx, |state, _| fill_for_stress(state));
        }
        bind_list_keys("MemoryList", cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-f", FocusMemorySearch, Some("MemoryScreen")),
            KeyBinding::new("delete", DeleteSelectedMemory, Some("MemoryList")),
        ]);

        let saved = settings::view_memory(cx, "memory");
        let (saved_filter, query, saved_selected, saved_scroll) = logic::decode(&saved);
        let bounds = Rc::new(RefCell::new(HashMap::new()));
        let weak = cx.entity().downgrade();
        let runner: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)> = Rc::new(move |action, target, window, cx| {
            let _ = weak.update(cx, |view, cx| view.run_menu(action, target, window, cx));
        });
        let placeholder = tr(cx, T, "Search memories");
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder(placeholder)
        });
        search.update(cx, |input, cx| input.set_value(query.clone(), window, cx));
        let choices = project_choices(&state.read(cx).memory, cx);
        let filter_index = saved_filter.as_ref().and_then(|name| choices.iter().position(|c| c.project.as_ref() == name));
        let project_filter = saved_filter.map(SharedString::from);
        let project = cx.new({
            let choices = choices.clone();
            |cx| SelectState::new(choices, Some(IndexPath::new(filter_index.unwrap_or(0))), window, cx)
        });
        let list = cx.new(|cx| {
            ListState::new(
                MemoryList {
                    state: state.clone(),
                    rows: Vec::new(),
                    selected: None,
                    selected_id: None,
                    focus: None,
                    hover_glows: motion::Glows::new(Pace::Glow), select_glows: motion::Glows::new(Pace::Glow),
                    hovered: HashSet::new(), bounds: bounds.clone(), runner: runner.clone(), saving: false,
                },
                window,
                cx,
            )
        });
        let focus = list.read(cx).focus_handle(cx);
        list.update(cx, |list, _| list.delegate_mut().focus = Some(focus));
        let edit = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx);
            input.set_rows(6, cx);
            input
        });

        let _subscriptions = vec![
            cx.observe_in(&state, window, Self::on_state_changed),
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.subscribe_in(&project, window, Self::on_project_event),
            cx.subscribe(&list, |_, _, _: &ListEvent, cx| cx.notify()),
            cx.subscribe(&edit, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let on_screen = state.read(cx).view == Screen::Memory;
        let mut this = Self {
            state,
            search,
            placeholder,
            project,
            choices,
            project_filter,
            list,
            query: query.clone().into(),
            terms: search_terms(&query),
            on_screen,
            edit,
            editing: None,
            corrections: Corrections::default(),
            restore: logic::Restore::default(), saved_selected, saved_scroll,
            last_scroll: Rc::new(Cell::new(saved_scroll)), scroll_ready: Rc::new(Cell::new(false)),
            restore_pending: Rc::new(Cell::new(false)), bounds, viewport: Rc::new(Cell::new(Bounds::default())),
            painted_scroll: Rc::new(Cell::new(0.0)), offer: logic::UndoOffer::default(), undo_len: 0,
            _subscriptions,
        };
        this.undo_len = this.state.read(cx).undo.len();
        this.offer.remember_top(this.state.read(cx).undo.clone().pop());
        this.sync_choices(window, cx);
        this.apply(false, window, cx);
        if on_screen {
            this.focus_list(window, cx);
        }
        if this.state.read(cx).scene.as_deref() == Some("memory-edit") {
            if let Some((id, body)) = this.selected_entry(cx).map(|e| (e.id, e.body.clone())) {
                this.start_correcting(id, &body, window, cx);
            }
        }
        this
    }

    fn on_state_changed(
        &mut self,
        _: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.observe_undo(cx);
        let on_screen = self.state.read(cx).view == Screen::Memory;
        if on_screen {
            self.sync_choices(window, cx);
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
                if value.as_ref() != capped { self.search.update(cx, |input, cx| input.set_value(capped.clone(), window, cx)); }
                if capped != self.query.as_ref() {
                    self.terms = search_terms(&capped);
                    self.query = capped.into();
                    self.cancel_restore(); self.remember_filter(cx);
                    self.apply(true, window, cx);
                }
            }
            InputEvent::PressEnter { .. } => self.focus_list(window, cx),
            _ => {}
        }
    }

    fn on_project_event(
        &mut self,
        _: &Entity<SelectState<Vec<ProjectChoice>>>,
        event: &SelectEvent<Vec<ProjectChoice>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(project) = event;
        let next = project.clone().filter(|project| !project.is_empty());
        if self.project_filter == next { return; }
        self.project_filter = next;
        self.cancel_restore(); self.remember_filter(cx);
        self.apply(true, window, cx);
    }

    /// Rebuilds the project menu after entries change, falling back to every project when the chosen one is gone.
    fn sync_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = project_choices(&self.state.read(cx).memory, cx);
        let loaded = matches!(panel_state(&self.state.read(cx).feeds.memory, self.state.read(cx).memory.is_empty()), PanelState::Ready | PanelState::Stale(_) | PanelState::Empty);
        let selected_index = self.project_filter.as_ref().and_then(|project| choices.iter().position(|choice| choice.project == *project));
        let filter_gone = loaded && self.project_filter.is_some() && selected_index.is_none();
        if choices == self.choices && !filter_gone { return; }
        self.choices = choices.clone();
        self.project.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_index(Some(IndexPath::new(selected_index.unwrap_or(0))), window, cx);
        });
        if filter_gone {
            self.project_filter = None;
            self.remember_filter(cx);
        }
    }

    fn cancel_restore(&mut self) { self.restore.cancel(); self.restore_pending.set(false); self.scroll_ready.set(true); }
    fn remember_filter(&self, cx: &mut Context<Self>) {
        let encoded = logic::encode(self.project_filter.as_deref(), &self.query, None, 0.0);
        let filter = encoded.filter;
        let sort = encoded.sort;
        let saved = settings::view_memory(cx, "memory");
        if saved.filter != filter || saved.sort != sort { settings::remember_view("memory", |v| { v.filter = filter; v.sort = sort; }, cx); }
    }
    fn apply(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let memory = &state.memory;
        let panel = panel_state(&state.feeds.memory, memory.is_empty());
        let rows = visible_memories(memory, self.project_filter.as_deref(), &self.terms);
        let restoring = self.restore.take(&panel);
        let previous = self.list.read(cx).delegate().selected_id;
        let (row, next, restored, save) = logic::selection(&rows, memory, self.saved_selected, previous, reset, restoring, self.restore.done());
        let valid: HashSet<i64> = memory.iter().map(|e| e.id).collect();
        self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            delegate.saving = false; delegate.rows = rows;
            delegate.hover_glows.retain(|id| valid.contains(id)); delegate.select_glows.retain(|id| valid.contains(id)); delegate.hovered.retain(|id| valid.contains(id));
            list.set_selected_index(row.map(IndexPath::new), window, cx);
            list.delegate_mut().saving = self.restore.done();
            if reset && row.is_some() { list.scroll_to_item(IndexPath::default(), ScrollStrategy::Top, window, cx); }
            cx.notify();
        });
        if let Some(save) = save {
            let selected = save.map(|id| id.to_string());
            if settings::view_memory(cx, "memory").selected != selected { settings::remember_view("memory", |v| v.selected = selected, cx); }
        }
        if restoring {
            if !restored { self.scroll_ready.set(true); }
            else {
                let handle = self.list.read(cx).scroll_handle().base_handle().clone();
                let offset = self.saved_scroll; let ready = self.scroll_ready.clone(); let pending = self.restore_pending.clone();
                pending.set(true);
                cx.on_next_frame(window, move |_, window, cx| cx.on_next_frame(window, move |this, _, cx| {
                    if !pending.replace(false) { return; }
                    if this.list.read(cx).delegate().selected_id == next { let mut point = handle.offset(); point.y = -px(offset); handle.set_offset(point); cx.notify(); }
                    ready.set(true);
                }));
            }
        }
        cx.notify();
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_restore();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.query = SharedString::default();
        self.terms.clear();
        self.project_filter = None;
        self.project.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default()), window, cx);
        });
        self.remember_filter(cx);
        self.apply(true, window, cx);
        self.focus_list(window, cx);
    }

    fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn focus_search(&mut self, _: &FocusMemorySearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    /// Selects one entry, as the command palette asks, and puts the keyboard on the list. A filter
    /// that would hide the entry is cleared first; one that shows it is left alone.
    pub(super) fn reveal(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_restore();
        let shown = {
            let memory = &self.state.read(cx).memory;
            visible_memories(memory, self.project_filter.as_deref(), &self.terms).iter().any(|&i| memory[i].id == id)
        };
        if !shown {
            self.cancel_restore();
            self.search.update(cx, |search, cx| search.set_value("", window, cx));
            self.query = SharedString::default();
            self.terms.clear();
            self.project_filter = None;
            self.project.update(cx, |select, cx| select.set_selected_index(Some(IndexPath::default()), window, cx));
        }
        self.remember_filter(cx);
        self.list.update(cx, |list, _| list.delegate_mut().selected_id = Some(id));
        self.apply(false, window, cx);
        let selected = self.list.read(cx).delegate().selected_id.map(|id| id.to_string());
        if settings::view_memory(cx, "memory").selected != selected { settings::remember_view("memory", |v| v.selected = selected, cx); }
        self.list.update(cx, |list, cx| list.scroll_to_selected_item(window, cx));
        self.focus_list(window, cx);
    }

    fn selected_entry<'a>(&self, cx: &'a App) -> Option<&'a MemoryEntry> {
        let delegate = self.list.read(cx).delegate();
        let row = delegate.selected?;
        self.state.read(cx).memory.get(*delegate.rows.get(row)?)
    }

    fn row_count(&self, cx: &App) -> usize {
        self.list.read(cx).delegate().rows.len()
    }

    fn run_menu(&mut self, action: MenuAction, target: MenuTarget, window: &mut Window, cx: &mut Context<Self>) {
        let effect = { let state = self.state.read(cx); if !state.menu_allowed(&target, action) { return; } logic::resolve(&state.memory, &target, action) };
        match effect {
            Some(logic::MemoryEffect::Copy(text)) => { cx.write_to_clipboard(ClipboardItem::new_string(text)); window.push_notification(Notification::success(tr(cx, T, "Memory copied")), cx); }
            Some(logic::MemoryEffect::Correct(id)) => { self.reveal(id, window, cx); if let Some(body) = self.state.read(cx).memory.iter().find(|e| e.id == id).map(|e| e.body.clone()) { self.start_correcting(id, &body, window, cx); } }
            Some(logic::MemoryEffect::Delete(id, title)) => self.confirm_delete(id, title.into(), window, cx),
            None => {}
        }
    }
    fn observe_undo(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let len = state.undo.len();
        let top = state.undo.clone().pop();
        self.offer.observe(top, self.undo_len, len);
        self.undo_len = len;
        if self.offer.offered.as_ref().is_some_and(|(id, _)| !self.state.read(cx).memory.iter().any(|e| e.id == *id)) { self.offer.clear(); }
    }
    fn undo_correction(&mut self, cx: &mut Context<Self>) {
        let Some((id, prior)) = self.offer.take() else { return };
        self.state.update(cx, |state, cx| {
            let top = state.take_undo();
            let memory_top = match &top { Some(Undoable::Memory { id, prior }) => Some((*id, prior)), _ => None };
            if !logic::can_undo(id, &prior, state.memory.iter().any(|entry| entry.id == id), memory_top) {
                if let Some(top) = top { state.record_undo(top); } return;
            }
            state.undo_memory(id, prior, cx);
        });
        self.undo_len = self.state.read(cx).undo.len();
        self.offer.remember_top(self.state.read(cx).undo.clone().pop());
        cx.notify();
    }
    fn cursor_menu_position(&self, id: i64, cx: &App) -> Option<Point<Pixels>> {
        let scroll = -f32::from(self.list.read(cx).scroll_handle().base_handle().offset().y);
        if (scroll - self.painted_scroll.get()).abs() > 0.5 { return None; }
        let bounds = self.bounds.borrow().get(&id).copied()?.intersect(&self.viewport.get());
        (bounds.size.width > px(0.) && bounds.size.height > px(0.)).then(|| point(bounds.origin.x + bounds.size.width / 2., bounds.origin.y + bounds.size.height / 2.))
    }
    fn dispatch_cursor_menu(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.read(cx).focus_handle(cx).is_focused(window) || self.list.read(cx).delegate().selected_id != Some(id) { return; }
        let Some(position) = self.cursor_menu_position(id, cx) else { return };
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, click_count: 1, ..Default::default() }), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, click_count: 1, ..Default::default() }), cx);
    }
    fn open_cursor_menu(&mut self, _: &palette::ContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.read(cx).focus_handle(cx).is_focused(window) { return; }
        let Some(id) = self.list.read(cx).delegate().selected_id else { return }; cx.stop_propagation();
        if self.cursor_menu_position(id, cx).is_some() { cx.defer_in(window, move |this, window, cx| this.dispatch_cursor_menu(id, window, cx)); }
        else { self.list.update(cx, |list, cx| list.scroll_to_selected_item(window, cx)); cx.on_next_frame(window, move |_, window, cx| cx.on_next_frame(window, move |this, window, cx| this.dispatch_cursor_menu(id, window, cx))); }
    }
    fn retry_note(&self, message: SharedString, cx: &Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        h_flex().w_full().items_center().gap(theme::space(theme::density(cx), Step::Sm))
            .child(ornament::stale_note(&cx.tokens(), message))
            .child(Button::new("memory-retry").ghost().small().label(tr(cx, T, "Try again"))
                .on_click(move |_, _, cx| state.update(cx, |state, cx| state.retry(FeedKind::Memory, cx))))
    }

    fn confirm_delete(&self, id: i64, title: SharedString, window: &mut Window, cx: &mut App) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let state = state.clone();
            alert
                .confirm()
                .entrance(super::motion::DIALOG_ENTRANCE)
                .title(tr(cx, T, "Delete this memory?"))
                .description(
                    v_flex()
                        .gap(theme::space(theme::density(cx), Step::Xs))
                        .child(div().font_medium().child(title.clone()))
                        .child(tr(
                            cx,
                            T,
                            "Later wishes will no longer be able to find it. This cannot be undone.",
                        )),
                )
                .ok_text(tr(cx, T, "Delete"))
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(tr(cx, T, "Cancel"))
                .on_ok(move |_, _, cx| {
                    state.update(cx, |state, cx| state.delete_memory(id, cx));
                    true
                })
        });
    }

    fn delete_selected(&mut self, _: &DeleteSelectedMemory, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, title)) = self.selected_entry(cx).map(|e| (e.id, e.title.clone())) {
            self.confirm_delete(id, title, window, cx);
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
            let current = self.list.read(cx).delegate().selected.unwrap_or(0);
            select_row(&self.list, row(current, count - 1), window, cx);
            cx.notify();
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

    /// Keeps the correction box on the entry it belongs to: moving to another entry sets a
    /// changed text aside as a draft, and coming back to that entry opens it again.
    fn sync_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        {
            let state = self.state.read(cx);
            let memory = &state.memory;
            self.corrections.settle(|id| memory.iter().find(|m| m.id == id).map(|m| m.body.to_string()), &state.errors);
        }
        let selected = self.selected_entry(cx).map(|e| (e.id, e.body.clone()));
        if let Some(editing) = self.editing {
            if selected.as_ref().map(|(id, _)| *id) == Some(editing) {
                return;
            }
            let typed = self.edit.read(cx).value().to_string();
            let body = self.state.read(cx).memory.iter().find(|m| m.id == editing).map(|m| m.body.clone());
            if body.is_some_and(|body| typed.trim() != body.trim()) {
                self.corrections.drafts.insert(editing, typed);
            }
            self.editing = None;
        }
        if let Some((id, _)) = selected {
            if let Some(draft) = self.corrections.drafts.remove(&id) {
                self.editing = Some(id);
                self.edit.update(cx, |edit, cx| edit.set_value(draft, window, cx));
            }
        }
    }

    fn start_correcting(&mut self, id: i64, body: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Some(id);
        let text = self.corrections.start_text(id, body);
        self.edit.update(cx, |edit, cx| edit.set_value(text, window, cx));
        self.edit.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn stop_correcting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.focus_list(window, cx);
        cx.notify();
    }

    fn save_correction(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.edit.read(cx).value().to_string();
        let current = self.state.read(cx).memory.iter().find(|m| m.id == id).map(|m| m.body.clone());
        let Some(current) = current else { return };
        if correction_problem(&text, &current).is_some() {
            return;
        }
        let errors_seen = self.state.read(cx).next_error;
        self.corrections.submit(id, text.clone(), errors_seen);
        self.state.update(cx, |state, cx| state.correct_memory(id, &text, cx));
        self.stop_correcting(window, cx);
    }

    fn problem_text(&self, problem: Problem, cx: &App) -> String {
        match problem {
            Problem::Empty => tr(cx, T, "The text cannot be empty.").to_owned(),
            Problem::TooLong => fill(tr(cx, T, "The text is longer than {} lines."), &[&MAX_CORRECTION_LINES.to_string()]),
            Problem::Unchanged => tr(cx, T, "The text has not changed.").to_owned(),
        }
    }

    fn editor(&self, entry: &MemoryEntry, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let (id, body) = (entry.id, entry.body.clone());
        let typed = self.edit.read(cx).value().to_string();
        let problem = correction_problem(&typed, &body);
        // An untouched box says nothing; the reason shows once there is something to judge.
        let shown = problem.filter(|p| *p != Problem::Unchanged || typed.trim() != body.trim());
        let note = shown.filter(|p| *p != Problem::Unchanged).map(|p| self.problem_text(p, cx));
        v_flex()
            .flex_1()
            .min_h_0()
            .gap(theme::space(theme::density(cx), Step::Sm))
            .child(
                div()
                    .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                    .text_color(t.muted)
                    .child(tr(cx, T, "Your correction replaces this note's text and is what later briefs see.")),
            )
            .child(Textarea::new(&self.edit).small().h(theme::row_height(theme::density(cx)) * 5.0))
            .when_some(note, |this, note| {
                this.child(div().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.fail).child(note))
            })
            .child(
                h_flex()
                    .gap(theme::space(theme::density(cx), Step::Sm))
                    .justify_end()
                    .child(
                        Button::new("memory-discard")
                            .ghost()
                            .small()
                            .label(tr(cx, T, "Discard"))
                            .on_click(cx.listener(|this, _, window, cx| this.stop_correcting(window, cx))),
                    )
                    .child(
                        gold_leaf(
                            Button::new("memory-save")
                                .primary()
                                .small()
                                .label(tr(cx, T, "Save correction"))
                                .disabled(problem.is_some())
                                .on_click(cx.listener(move |this, _, window, cx| this.save_correction(id, window, cx))),
                            &t,
                        ),
                    ),
            )
            .into_any_element()
    }

    fn nothing_yet(&self, cx: &App) -> Empty {
        let hint = tr(
            cx,
            T,
            "When a wish is verified, Chronos notes what each unit built so later wishes can reuse it.",
        );
        empty_state(IconName::BookOpen, "No memories yet", hint, None, T, cx)
    }

    fn nothing_matches(&self, cx: &mut Context<Self>) -> Empty {
        let action = Button::new("memory-clear-filters")
            .label(tr(cx, T, "Clear filters"))
            .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx)))
            .into_any_element();
        let hint = tr(cx, T, "Search looks in the title, the text and the project name.");
        empty_state(IconName::Search, "No matching memories", hint, Some(action), T, cx)
    }

    fn detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let Some(entry) = self.selected_entry(cx).cloned() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(theme::font(theme::density(cx), TypeSize::Body))
                .text_color(t.muted)
                .child(tr(cx, T, "Select a memory to read it"))
                .into_any_element();
        };
        let (id, title) = (entry.id, entry.title.clone());
        let d = theme::density(cx);
        let target = MenuTarget::Memory(id);
        let state = self.state.clone();
        let weak = cx.entity().downgrade();
        let runner: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)> = Rc::new(move |action, target, window, cx| { let _ = weak.update(cx, |view, cx| view.run_menu(action, target, window, cx)); });
        v_flex()
            .size_full()
            .p(theme::space(d, Step::Xl))
            .gap(theme::space(d, Step::Lg))
            .child(
                v_flex()
                    .flex_none()
                    .gap(theme::space(theme::density(cx), Step::Sm))
                    .child(
                        h_flex().justify_between().gap(theme::space(d, Step::Sm))
                            .child(div().min_w_0().text_size(theme::font(d, TypeSize::Heading)).font_semibold().text_color(t.text).child(entry.title.clone()))
                            .child(Button::new("memory-menu").ghost().small().icon(IconName::Ellipsis).min_w(theme::TARGET_MIN).min_h(theme::TARGET_MIN)
                                .accessibility_label(tr(cx, T, "More actions"))
                                .tooltip(tr(cx, T, "More actions"))
                                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| palette::menu::context_menu(menu, target.clone(), state.clone(), runner.clone(), cx))),
                    )
                    .child(
                        h_flex()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                            .text_color(t.muted)
                            .child(Icon::new(IconName::Folder).size(theme::font(theme::density(cx), TypeSize::Body)))
                            .child(entry.project.clone())
                            .child(format!("#{}", entry.id)),
                    )
                    .child(
                        div()
                            .id(("memory-wish", entry.id as u64))
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                            .text_color(t.muted)
                            .line_clamp(3)
                            .child(if entry.wish.trim().is_empty() {
                                tr(cx, T, "The wish was not recorded").to_string()
                            } else {
                                format!("{}: {}", tr(cx, T, "From the wish"), one_line(&entry.wish))
                            }),
                    )
                    .child(
                        h_flex()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .items_center()
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                            .text_color(t.muted)
                            .child(Icon::new(IconName::Calendar).size(theme::font(theme::density(cx), TypeSize::Body)))
                            .child(confirmed_line(&entry, *cx.global::<Lang>()))
                            .when(entry.corrected, |this| {
                                this.child(
                                    div()
                                        .id(("memory-corrected", entry.id as u64))
                                        .px_2()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(t.border)
                                        .child(tr(cx, T, "Corrected"))
                                        .tooltip({
                                            let tip = tr(
                                                cx,
                                                T,
                                                "A person rewrote this note. The new text is what later briefs see.",
                                            );
                                            move |window, cx| Tooltip::new(tip).build(window, cx)
                                        }),
                                )
                            }),
                    ),
            )
            .when(self.offer.offered.as_ref().is_some_and(|(offered_id, _)| *offered_id == id), |this| {
                this.child(h_flex().gap(theme::space(d, Step::Sm)).items_center().text_size(theme::font(d, TypeSize::Caption)).text_color(t.muted)
                    .child(tr(cx, T, "Correction saved"))
                    .child(Button::new("memory-undo").ghost().small().min_h(theme::TARGET_MIN).label(tr(cx, T, "Undo")).on_click(cx.listener(|this, _, _, cx| this.undo_correction(cx))))
                    .child(Button::new("memory-dismiss-undo").ghost().small().min_h(theme::TARGET_MIN).label(tr(cx, T, "Dismiss")).on_click(cx.listener(|this, _, _, cx| { this.offer.clear(); cx.notify(); }))))
            })
            .when(entry.stale, |this| {
                this.child(
                    Alert::warning(
                        ("memory-stale-note", entry.id as u64),
                        tr(
                            cx,
                            T,
                            "A file it was recorded from has changed or been removed since. Chronos reads the current file instead of quoting this note in a brief.",
                        ),
                    )
                    .title(tr(cx, T, "This memory may be out of date"))
                    .flex_none(),
                )
            })
            .child(if self.editing == Some(entry.id) {
                self.editor(&entry, cx)
            } else {
                v_flex()
                    .id(("memory-body", entry.id as u64))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .text_size(theme::font(theme::density(cx), TypeSize::Body))
                            .line_height(relative(1.7))
                            .text_color(t.text)
                            .whitespace_normal()
                            .child(SelectableText::new(
                                ("memory-body-text", entry.id as u64),
                                entry.body.clone(),
                            )),
                    )
                    .into_any_element()
            })
            .child(
                h_flex()
                    .flex_none()
                    .justify_between()
                    .child({
                        let body = entry.body.clone();
                        Button::new("memory-correct")
                            .ghost()
                            .small()
                            .icon(IconName::Replace)
                            .label(tr(cx, T, "Correct"))
                            .accessibility_label(tr(cx, T, "Correct this memory"))
                            .disabled(self.editing == Some(entry.id))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_correcting(id, &body, window, cx);
                            }))
                    })
                    .child(
                        Button::new("memory-delete")
                            .ghost()
                            .small()
                            .danger()
                            .label(tr(cx, T, "Delete"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete(id, title.clone(), window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn fill_for_stress(state: &mut AppState) {
    let base = std::mem::take(&mut state.memory);
    state.memory = (0..6000)
        .map(|i| {
            let mut entry = base[i % base.len()].clone();
            entry.id = i as i64;
            entry.title = format!("{} #{i}", entry.title).into();
            entry
        })
        .collect();
}

impl Render for MemoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens(); let d = theme::density(cx);
        let placeholder = tr(cx, T, "Search memories");
        if self.placeholder != placeholder { self.placeholder = placeholder; self.search.update(cx, |search, cx| search.set_placeholder(placeholder, window, cx)); }
        let state = self.state.read(cx);
        let no_entries = state.memory.is_empty();
        let panel = panel_state(&state.feeds.memory, no_entries);
        let view = logic::screen_state(&panel);
        let no_rows = self.row_count(cx) == 0;
        self.sync_editing(window, cx);
        let handle = self.list.read(cx).scroll_handle().base_handle().clone();
        let last = self.last_scroll.clone(); let ready = self.scroll_ready.clone();
        let viewport = self.viewport.clone(); let painted_scroll = self.painted_scroll.clone(); let bounds = self.bounds.clone();
        let width = px(logic::list_width(f32::from(window.viewport_size().width)));
        v_flex().key_context("MemoryScreen").on_action(cx.listener(Self::focus_search))
            .size_full().min_h_0().pt(theme::space(d, Step::Lg)).px(theme::space(d, Step::Xl)).pb(theme::space(d, Step::Lg)).gap(theme::space(d, Step::Lg))
            .child(ornament::page_title(&t, tr(cx, T, "What Chronos has learned")).flex_none())
            .when(view == logic::ScreenState::Loading, |this| this.child(ornament::empty_state(&t, tr(cx, T, "Loading memories").into(), "".into()).size_full()))
            .when(matches!(view, logic::ScreenState::Failed | logic::ScreenState::Stale), |this| {
                let message = match &panel { PanelState::Failed(message) | PanelState::Stale(message) => message.clone(), _ => unreachable!() };
                this.child(self.retry_note(message, cx))
            })
            .when(matches!(view, logic::ScreenState::Empty | logic::ScreenState::Ready | logic::ScreenState::Stale), |this| {
                this.when(no_entries, |this| this.child(self.nothing_yet(cx)))
                    .when(!no_entries, |this| this.child(h_flex().flex_none().gap(theme::space(d, Step::Md))
                        .child(div().w(theme::row_height(d) * 6.0).flex_none().child(Select::new(&self.project).small()))
                        .child(div().w(theme::row_height(d) * 8.0).flex_shrink(1.)
                            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| { this.focus_list(window, cx); cx.stop_propagation(); }))
                            .child(Input::new(&self.search).small().cleanable(true).prefix(Icon::new(IconName::Search).size(theme::font(d, TypeSize::Body)).text_color(t.muted)))))
                        .when(no_rows, |this| this.child(self.nothing_matches(cx)))
                        .when(!no_rows, |this| this.child(h_flex().flex_1().min_h_0().items_start().gap(theme::space(d, Step::Lg))
                            .child(div().key_context("MemoryList").on_action(cx.listener(Self::first_row)).on_action(cx.listener(Self::last_row))
                                .on_action(cx.listener(Self::page_up)).on_action(cx.listener(Self::page_down)).on_action(cx.listener(Self::delete_selected)).on_action(cx.listener(Self::open_cursor_menu))
                                .w(width).flex_none().h_full()
                                // Clear before the List's row callbacks record this frame's bounds.
                                .on_prepaint(move |rect, _, cx| { viewport.set(rect); bounds.borrow_mut().clear(); painted_scroll.set(-f32::from(handle.offset().y));
                                    if ready.get() { let value = logic::scroll(-f32::from(handle.offset().y)); if (value - last.get()).abs() > 1.0 { last.set(value); settings::remember_view("memory", |v| v.scroll = value, cx); } }
                                }).child(List::new(&self.list).size_full()))
                            .child(div().flex_1().min_w_0().max_w(logic::detail_width(d)).h_full().rounded(theme::RADIUS).border_1().border_color(t.border).bg(t.surface).child(self.detail(cx))))))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    fn entry(id: i64, project: &str, title: &str, body: &str, stale: bool) -> MemoryEntry {
        MemoryEntry {
            id,
            project: project.to_owned().into(),
            title: title.to_owned().into(),
            body: body.to_owned().into(),
            stale,
            wish: SharedString::default(),
            confirmed: SharedString::default(),
            corrected: false,
        }
    }

    fn sample() -> Vec<MemoryEntry> {
        vec![
            entry(33, "chronos-r4", "CRLF breaks doc tests", "Read with CRLF tolerance", true),
            entry(48, "chronos-r5", "Stop hook blocks idle turns", "Polling wastes turns", false),
            entry(41, "ml_hfo2", "Page images beat tiles", "單頁 300 dpi 比拼貼便宜", false),
        ]
    }

    #[test]
    fn newest_recorded_first() {
        assert_eq!(visible_memories(&sample(), None, &[]), vec![1, 2, 0]);
    }

    #[test]
    fn filters_by_project() {
        assert_eq!(visible_memories(&sample(), Some("ml_hfo2"), &[]), vec![2]);
        assert!(visible_memories(&sample(), Some("nope"), &[]).is_empty());
    }

    #[test]
    fn search_covers_title_body_and_project() {
        let memory = sample();
        assert_eq!(visible_memories(&memory, None, &search_terms("crlf")), vec![0]);
        assert_eq!(visible_memories(&memory, None, &search_terms("polling")), vec![1]);
        assert_eq!(visible_memories(&memory, None, &search_terms("拼貼")), vec![2]);
        assert_eq!(visible_memories(&memory, None, &search_terms("chronos-r5")), vec![1]);
    }

    #[test]
    fn project_and_search_combine() {
        let memory = sample();
        assert_eq!(
            visible_memories(&memory, Some("chronos-r5"), &search_terms("stop")),
            vec![1]
        );
        assert!(visible_memories(&memory, Some("ml_hfo2"), &search_terms("stop")).is_empty());
    }

    #[test]
    fn project_counts_are_sorted_and_tallied() {
        let mut memory = sample();
        memory.push(entry(7, "ml_hfo2", "x", "y", false));
        let counts = project_counts(&memory);
        let names: Vec<&str> = counts.iter().map(|(name, _)| name.as_ref()).collect();
        assert_eq!(names, ["chronos-r4", "chronos-r5", "ml_hfo2"]);
        assert_eq!(counts[2].1, 2);
    }

    #[test]
    fn no_memories_give_no_projects() {
        assert!(project_counts(&[]).is_empty());
    }

    #[test]
    fn search_also_finds_the_wish_an_entry_came_from() {
        let mut memory = sample();
        memory[1].wish = "make the nightly report faster".into();
        assert_eq!(visible_memories(&memory, None, &search_terms("nightly")), vec![1]);
    }

    fn failure(id: u64) -> AppError {
        AppError { id, action: crate::desktop::model::Action::CorrectMemory, session: None, message: "disk full".into() }
    }

    #[test]
    fn a_correction_is_kept_until_saved_and_a_failure_can_be_retried() {
        let mut c = Corrections::default();
        let old = "old text";
        c.submit(7, "my edit".into(), 0);
        // Queued but not yet shown: pressing Correct again must not bring back the old text.
        c.settle(|_| Some(old.to_owned()), &[]);
        assert_eq!(c.start_text(7, old), "my edit");
        // The engine reports a failure later: the text comes back as a draft to retry.
        c.settle(|_| Some(old.to_owned()), &[failure(1)]);
        assert!(c.sent.is_empty());
        assert_eq!(c.drafts.get(&7).map(String::as_str), Some("my edit"));
        assert_eq!(c.start_text(7, old), "my edit");
        // The retry succeeds: once the entry shows it, nothing is kept.
        c.submit(7, "my edit".into(), 1);
        c.settle(|_| Some("my edit".to_owned()), &[failure(1)]);
        assert!(c.sent.is_empty() && c.drafts.is_empty());
        assert_eq!(c.start_text(7, "my edit"), "my edit");
    }

    #[test]
    fn a_correction_of_a_deleted_entry_is_forgotten() {
        let mut c = Corrections::default();
        c.submit(7, "x".into(), 0);
        c.settle(|_| None, &[]);
        assert!(c.sent.is_empty() && c.drafts.is_empty());
    }

    #[test]
    fn a_correction_must_say_something_new_and_fit_the_store() {
        assert_eq!(correction_problem("  \n ", "old"), Some(Problem::Empty));
        assert_eq!(correction_problem(" old \n", "old"), Some(Problem::Unchanged));
        assert_eq!(correction_problem("new", "old"), None);
        let twelve = vec!["line"; MAX_CORRECTION_LINES].join("\n");
        assert_eq!(correction_problem(&twelve, "old"), None);
        let thirteen = vec!["line"; MAX_CORRECTION_LINES + 1].join("\n");
        assert_eq!(correction_problem(&thirteen, "old"), Some(Problem::TooLong));
    }

    #[test]
    fn the_line_limit_is_the_one_the_memory_store_enforces() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::memory::Memory::open(dir.path()).unwrap();
        let db = rusqlite::Connection::open(dir.path().join("memory/memory.db")).unwrap();
        db.execute(
            "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('p', 'u', 't', 's', 'b', '', 'w', '[]', '/nowhere', 1000.0)",
            [],
        )
        .unwrap();
        let id = db.last_insert_rowid();
        let fits = vec!["x"; MAX_CORRECTION_LINES].join("
");
        let over = vec!["x"; MAX_CORRECTION_LINES + 1].join("
");
        assert!(store.correct(id, &fits).is_ok());
        assert!(store.correct(id, &over).is_err());
    }

    #[test]
    fn the_confirmed_time_reads_in_both_languages_and_never_guesses() {
        let mut e = entry(1, "p", "t", "b", false);
        e.confirmed = "Nov 3, 05:15".into();
        assert_eq!(confirmed_line(&e, Lang::En), "Last confirmed Nov 3, 05:15");
        assert_eq!(confirmed_line(&e, Lang::ZhTw), "最近確認於 11月3日 05:15");
        e.confirmed = SharedString::default();
        assert_eq!(confirmed_line(&e, Lang::En), "Last confirmed: not recorded");
        assert_eq!(confirmed_line(&e, Lang::ZhTw), "最近確認時間：沒有記錄");
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
