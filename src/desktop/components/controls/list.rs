//! Searchable wrapper for the kit list state and delegate.
use super::{
    button::ring,
    keys::{self, Confirm, First, Last, MoveDown, MoveUp, PageDown, PageUp},
};
use crate::desktop::components::tokens::Tokens;
use gpui_kit::component::{
    list::{List as KitList, ListDelegate, ListState},
    IndexPath as KitIndexPath,
};
use gpui_kit::{
    div, App, ElementId, Entity, Global, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, Styled, Window,
};

/// List position. Convert explicitly at kit list delegate and state boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexPath {
    pub section: usize,
    pub row: usize,
    pub column: usize,
}
impl IndexPath {
    pub fn new(row: usize) -> Self {
        Self {
            row,
            ..Self::default()
        }
    }
    pub fn section(mut self, section: usize) -> Self {
        self.section = section;
        self
    }
}
impl From<IndexPath> for KitIndexPath {
    fn from(path: IndexPath) -> Self {
        Self {
            section: path.section,
            row: path.row,
            column: path.column,
        }
    }
}
impl From<KitIndexPath> for IndexPath {
    fn from(path: KitIndexPath) -> Self {
        Self {
            section: path.section,
            row: path.row,
            column: path.column,
        }
    }
}
impl From<IndexPath> for ElementId {
    fn from(path: IndexPath) -> Self {
        KitIndexPath::from(path).into()
    }
}

struct Bindings;
impl Global for Bindings {}
#[derive(IntoElement)]
pub struct List<D: ListDelegate> {
    id: ElementId,
    state: Entity<ListState<D>>,
    tokens: Tokens,
    page: usize,
}
impl<D: ListDelegate> List<D> {
    pub fn new(id: impl Into<ElementId>, state: &Entity<ListState<D>>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            tokens: *tokens,
            page: page_size(10),
        }
    }
    pub fn page_size(mut self, page: usize) -> Self {
        self.page = page_size(page);
        self
    }
    pub fn set_query(&self, query: &str, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.set_query(normalize_query(query), window, cx)
        });
    }
}
fn page_size(page: usize) -> usize {
    page.max(1)
}
fn normalize_query(query: &str) -> &str {
    query.trim()
}
fn can_confirm(counts: &[usize], current: Option<KitIndexPath>) -> bool {
    navigate_index(counts, current, 0).is_some()
}
fn navigate_index(
    counts: &[usize],
    current: Option<KitIndexPath>,
    direction: isize,
) -> Option<KitIndexPath> {
    let total = counts.iter().fold(0usize, |sum, &n| sum.saturating_add(n));
    if total == 0 {
        return None;
    }
    let current = current.and_then(|ix| {
        counts.get(ix.section).filter(|&&n| ix.row < n).map(|_| {
            counts[..ix.section]
                .iter()
                .fold(0usize, |sum, &n| sum.saturating_add(n))
                .saturating_add(ix.row)
        })
    });
    let target = match (direction, current) {
        (isize::MIN, _) => 0,
        (isize::MAX, _) => total - 1,
        (0, None) => return None,
        (_, Some(i)) => i.saturating_add_signed(direction).min(total - 1),
        (d, None) if d < 0 => total - 1,
        (_, None) => 0,
    };
    let mut row = target;
    for (section, &count) in counts.iter().enumerate() {
        if row < count {
            return Some(KitIndexPath::new(row).section(section));
        }
        row -= count;
    }
    None
}
fn navigate<D: ListDelegate>(
    state: &Entity<ListState<D>>,
    direction: isize,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |s, cx| {
        let counts: Vec<_> = (0..s.delegate().sections_count(cx))
            .map(|i| s.delegate().items_count(i, cx))
            .collect();
        let selected = navigate_index(&counts, s.selected_index(), direction);
        s.set_selected_index(selected, window, cx);
        if selected.is_none() {
            return;
        }
        s.scroll_to_selected_item(window, cx);
        cx.notify();
    });
}
impl<D: ListDelegate> RenderOnce for List<D> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !cx.has_global::<Bindings>() {
            keys::bind_list("AxiomList", cx);
            cx.set_global(Bindings);
        }
        self.state
            .update(cx, |state, cx| state.set_searchable(true, cx));
        let focus = window.use_keyed_state((self.id.clone(), "list-focus"), cx, |_, cx| {
            cx.focus_handle()
        });
        let focus = focus.read(cx).clone().tab_stop(true);
        let up = self.state.clone();
        let down = self.state.clone();
        let pgup = self.state.clone();
        let pgdown = self.state.clone();
        let first = self.state.clone();
        let last = self.state.clone();
        let confirm = self.state.clone();
        let page = self.page.min(isize::MAX as usize) as isize;
        div()
            .id(self.id)
            .key_context("AxiomList")
            .track_focus(&focus)
            .bg(self.tokens.surface)
            .text_color(self.tokens.text)
            .shadow(ring(focus.contains_focused(window, cx), &self.tokens))
            .on_action(move |_: &MoveUp, w, c| {
                navigate(&up, -1, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &MoveDown, w, c| {
                navigate(&down, 1, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &PageUp, w, c| {
                navigate(&pgup, -page, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &PageDown, w, c| {
                navigate(&pgdown, page, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &First, w, c| {
                navigate(&first, isize::MIN, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &Last, w, c| {
                navigate(&last, isize::MAX, w, c);
                c.stop_propagation();
            })
            .on_action(move |_: &Confirm, w, c| {
                confirm.update(c, |s, c| {
                    let counts: Vec<_> = (0..s.delegate().sections_count(c))
                        .map(|i| s.delegate().items_count(i, c))
                        .collect();
                    if can_confirm(&counts, s.selected_index()) {
                        s.delegate_mut().confirm(false, w, c);
                    }
                });
                c.stop_propagation();
            })
            .child(KitList::new(&self.state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirm_requires_valid_selection() {
        let at = |section, row| Some(KitIndexPath::new(row).section(section));
        assert_eq!(navigate_index(&[2, 0, 3], at(2, 1), 0), at(2, 1));
        assert_eq!(navigate_index(&[2, 0, 3], at(2, 3), 0), None);
        assert_eq!(navigate_index(&[0, 0], at(0, 0), 0), None);
        assert!(can_confirm(&[2, 0, 3], at(2, 1)));
        assert!(!can_confirm(&[2, 0, 3], at(2, 3)));
        assert!(!can_confirm(&[0, 0], None));
    }
    #[test]
    fn page_steps_cross_sections_and_clamp() {
        let counts = [2, 0, 3, 2];
        let at = |section, row| Some(KitIndexPath::new(row).section(section));
        assert_eq!(page_size(0), 1);
        assert_eq!(page_size(3), 3);
        let page = page_size(3) as isize;
        assert_eq!(navigate_index(&counts, at(0, 1), page), at(2, 2));
        assert_eq!(navigate_index(&counts, at(3, 0), -page), at(2, 0));
        assert_eq!(navigate_index(&counts, at(0, 0), -page), at(0, 0));
        assert_eq!(navigate_index(&counts, at(3, 1), page), at(3, 1));
    }
    #[test]
    fn query_is_normalized_before_forwarding() {
        assert_eq!(normalize_query("  alpha beta \n"), "alpha beta");
        assert_eq!(normalize_query("beta"), "beta");
        assert_eq!(normalize_query(" \t"), "");
        assert_eq!(normalize_query(""), "");
    }
    #[test]
    fn navigation_sections_and_stale_selection() {
        for _ in crate::desktop::theme::both() {
            let counts = [2, 0, 3];
            let at = |section, row| Some(KitIndexPath::new(row).section(section));
            assert_eq!(navigate_index(&counts, at(0, 1), 1), at(2, 0));
            assert_eq!(navigate_index(&counts, at(2, 0), -1), at(0, 1));
            assert_eq!(navigate_index(&counts, at(0, 0), 3), at(2, 1));
            assert_eq!(navigate_index(&counts, at(2, 2), -3), at(0, 1));
            assert_eq!(navigate_index(&counts, at(2, 2), isize::MIN), at(0, 0));
            assert_eq!(navigate_index(&counts, at(0, 0), isize::MAX), at(2, 2));
            assert_eq!(navigate_index(&counts, at(99, 0), isize::MIN), at(0, 0));
            assert_eq!(navigate_index(&counts, at(99, 0), isize::MAX), at(2, 2));
            assert_eq!(navigate_index(&[0, 0], at(0, 0), 1), None);
            assert_eq!(navigate_index(&counts, at(99, 0), 0), None);
        }
    }
}

#[cfg(test)]
mod index_path_tests {
    use crate::desktop::components::IndexPath;
    // Type-check the same conversions used by the kit state and delegate.
    #[allow(dead_code)]
    fn state_and_delegate<D: gpui_kit::component::list::ListDelegate>(
        state: &mut gpui_kit::component::list::ListState<D>,
        delegate: &mut D,
        path: IndexPath,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<gpui_kit::component::list::ListState<D>>,
    ) {
        state.set_selected_index(Some(path.into()), window, cx);
        let _ = delegate.render_item(path.into(), window, cx);
        let _: Option<IndexPath> = state.selected_index().map(Into::into);
    }
    #[test]
    fn index_path_round_trips_at_kit_boundaries() {
        let path = IndexPath::new(7).section(2);
        assert_eq!((path.section, path.row), (2, 7));
        let kit: gpui_kit::component::IndexPath = path.into();
        assert_eq!((kit.section, kit.row), (2, 7));
        assert_eq!(IndexPath::from(kit), path);
        let _: gpui_kit::ElementId = path.into();
        for tokens in crate::desktop::theme::both() {
            let _row = crate::desktop::components::ListItem::new(path, "row", &tokens);
        }
    }
}
