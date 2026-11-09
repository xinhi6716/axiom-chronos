//! What Chronos has learned from verified units: filter by project, search, read, delete.
use super::i18n::{tr, Table};
use super::model::*;
use super::theme::Tokened as _;
use super::wishes::{bind_list_keys, empty_state, matches_terms, one_line, search_terms, select_row};
use gpui_kit::base::actions::{SelectFirst, SelectLast, SelectPageDown, SelectPageUp};
use gpui_kit::base::SelectableText;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::empty::Empty;
use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, Icon, IconName, IndexPath, Sizable as _, StyledExt as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cmp::Reverse;

gpui_kit::actions!(chronos_memory, [FocusMemorySearch, DeleteSelectedMemory]);

const T: Table = &[
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
];

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
    matches_terms(&[&entry.title, &entry.body, &entry.project], terms)
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
            .gap_3()
            .child(div().truncate().child(self.title.clone()))
            .child(
                div()
                    .text_xs()
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

const ROW_HEIGHT: Pixels = px(60.);
const LIST_WIDTH: Pixels = px(360.);
const PAGE_ROWS: usize = 8;

fn stale_tag(cx: &App) -> Tag {
    let t = cx.tokens();
    Tag::custom(t.review.opacity(0.14), t.review, t.review.opacity(0.4))
        .xsmall()
        .text_xs()
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
        let selected = self.selected == Some(ix.row);
        let focused = selected && self.focus.as_ref().is_some_and(|f| f.is_focused(window));
        let meaning = tr(
            cx,
            T,
            "A file it was recorded from has changed or been removed since. Chronos reads the current file instead of quoting this note in a brief.",
        );
        let summary = SharedString::from(format!("{} · {}", entry.project, one_line(&entry.body)));
        let title = one_line(&entry.title);
        let stale = entry.stale;
        Some(
            ListItem::new(ix.row)
                .accessibility_label(title.clone())
                .h(ROW_HEIGHT)
                .py_0()
                .px_0()
                .rounded(px(6.))
                .child(
                    v_flex()
                        .relative()
                        .w_full()
                        .h(ROW_HEIGHT)
                        .px_3()
                        .justify_center()
                        .gap_0p5()
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
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_between()
                                .child(div().min_w_0().truncate().text_sm().child(title))
                                .when(stale, |this| {
                                    this.child(
                                        div()
                                            .id(("memory-stale", ix.row))
                                            .flex_none()
                                            .child(stale_tag(cx))
                                            .tooltip(move |window, cx| {
                                                Tooltip::new(meaning).build(window, cx)
                                            }),
                                    )
                                }),
                        )
                        .child(div().truncate().text_xs().text_color(t.muted).child(summary)),
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
        self.selected_id = ix
            .and_then(|ix| self.rows.get(ix.row))
            .and_then(|&index| self.state.read(cx).memory.get(index))
            .map(|entry| entry.id);
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

        let placeholder = tr(cx, T, "Search memories");
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder(placeholder)
        });
        let choices = project_choices(&state.read(cx).memory, cx);
        let project = cx.new({
            let choices = choices.clone();
            |cx| SelectState::new(choices, Some(IndexPath::default()), window, cx)
        });
        let list = cx.new(|cx| {
            ListState::new(
                MemoryList {
                    state: state.clone(),
                    rows: Vec::new(),
                    selected: None,
                    selected_id: None,
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
            cx.subscribe_in(&project, window, Self::on_project_event),
            cx.subscribe(&list, |_, _, _: &ListEvent, cx| cx.notify()),
        ];
        let on_screen = state.read(cx).view == Screen::Memory;
        let mut this = Self {
            state,
            search,
            placeholder,
            project,
            choices,
            project_filter: None,
            list,
            query: SharedString::default(),
            terms: Vec::new(),
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

    fn on_project_event(
        &mut self,
        _: &Entity<SelectState<Vec<ProjectChoice>>>,
        event: &SelectEvent<Vec<ProjectChoice>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(project) = event;
        self.project_filter = project.clone().filter(|project| !project.is_empty());
        self.apply(true, window, cx);
    }

    /// Rebuilds the project menu after entries change, falling back to every project when the chosen one is gone.
    fn sync_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = project_choices(&self.state.read(cx).memory, cx);
        if choices == self.choices {
            return;
        }
        self.choices = choices.clone();
        let filter_gone = self
            .project_filter
            .as_ref()
            .is_some_and(|project| !choices.iter().any(|choice| choice.project == *project));
        self.project.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            if filter_gone {
                select.set_selected_index(Some(IndexPath::default()), window, cx);
            }
        });
        if filter_gone {
            self.project_filter = None;
        }
    }

    fn apply(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let memory = &self.state.read(cx).memory;
        let rows = visible_memories(memory, self.project_filter.as_deref(), &self.terms);
        let delegate = self.list.read(cx).delegate();
        let row = if reset {
            0
        } else {
            let kept = delegate.selected_id.and_then(|id| {
                rows.iter().position(|&i| memory[i].id == id)
            });
            kept.or(delegate.selected).unwrap_or(0).min(rows.len().saturating_sub(1))
        };
        let selected = (!rows.is_empty()).then(|| IndexPath::new(row));
        self.list.update(cx, |list, cx| {
            list.delegate_mut().rows = rows;
            list.set_selected_index(selected, window, cx);
            if reset {
                list.scroll_to_item(IndexPath::default(), ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.query = SharedString::default();
        self.terms.clear();
        self.project_filter = None;
        self.project.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default()), window, cx);
        });
        self.apply(true, window, cx);
        self.focus_list(window, cx);
    }

    fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn focus_search(&mut self, _: &FocusMemorySearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn selected_entry<'a>(&self, cx: &'a App) -> Option<&'a MemoryEntry> {
        let delegate = self.list.read(cx).delegate();
        let row = delegate.selected?;
        self.state.read(cx).memory.get(*delegate.rows.get(row)?)
    }

    fn row_count(&self, cx: &App) -> usize {
        self.list.read(cx).delegate().rows.len()
    }

    fn confirm_delete(&self, id: i64, title: SharedString, window: &mut Window, cx: &mut App) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let state = state.clone();
            alert
                .confirm()
                .title(tr(cx, T, "Delete this memory?"))
                .description(
                    v_flex()
                        .gap_1()
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
                .text_sm()
                .text_color(t.muted)
                .child(tr(cx, T, "Select a memory to read it"))
                .into_any_element();
        };
        let (id, title) = (entry.id, entry.title.clone());
        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .child(
                v_flex()
                    .flex_none()
                    .gap_1p5()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(t.text)
                            .child(entry.title.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(t.muted)
                            .child(Icon::new(IconName::Folder).size(px(13.)))
                            .child(entry.project.clone())
                            .child(format!("#{}", entry.id)),
                    ),
            )
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
            .child(
                v_flex()
                    .id(("memory-body", entry.id as u64))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .text_sm()
                            .line_height(relative(1.7))
                            .text_color(t.text)
                            .whitespace_normal()
                            .child(SelectableText::new(
                                ("memory-body-text", entry.id as u64),
                                entry.body.clone(),
                            )),
                    ),
            )
            .child(
                h_flex().flex_none().justify_end().child(
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
        let t = cx.tokens();
        let placeholder = tr(cx, T, "Search memories");
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.search
                .update(cx, |search, cx| search.set_placeholder(placeholder, window, cx));
        }
        let no_entries = self.state.read(cx).memory.is_empty();
        let no_rows = self.row_count(cx) == 0;

        v_flex()
            .key_context("MemoryScreen")
            .on_action(cx.listener(Self::focus_search))
            .size_full()
            .min_h_0()
            .pt_5()
            .px_6()
            .pb_5()
            .gap_4()
            .when(no_entries, |this| this.child(self.nothing_yet(cx)))
            .when(!no_entries, |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .gap_3()
                        .child(div().w(px(200.)).flex_none().child(Select::new(&self.project).small()))
                        .child(
                            div()
                                .w(px(280.))
                                .flex_shrink(1.)
                                .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                                    this.focus_list(window, cx);
                                    cx.stop_propagation();
                                }))
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
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_start()
                            .gap_4()
                            .child(
                                div()
                                    .key_context("MemoryList")
                                    .on_action(cx.listener(Self::first_row))
                                    .on_action(cx.listener(Self::last_row))
                                    .on_action(cx.listener(Self::page_up))
                                    .on_action(cx.listener(Self::page_down))
                                    .on_action(cx.listener(Self::delete_selected))
                                    .w(LIST_WIDTH)
                                    .flex_none()
                                    .h_full()
                                    .child(List::new(&self.list).size_full()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(t.border)
                                    .bg(t.surface)
                                    .child(self.detail(cx)),
                            ),
                    )
                })
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
    fn table_keys_are_unique() {
        let mut keys: Vec<&str> = T.iter().map(|(en, _)| *en).collect();
        keys.sort_unstable();
        let total = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), total);
    }
}
