//! Keys: shortcuts, context menu, page registry.
use crate::desktop::components::{Kbd, Tokens};
use crate::desktop::i18n::{tr, Table};
use crate::desktop::model::{AppState, MenuAction, MenuTarget, Screen};
use crate::desktop::model::facade::settings::FocusPanel;
use crate::desktop::ux::Page;
use gpui_kit::base::actions::{SelectFirst, SelectLast, SelectPageDown, SelectPageUp};
use gpui_kit::component::list::{ListDelegate, ListState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{IconName, IndexPath};
use gpui_kit::*;
use std::rc::Rc;

/// Where a shortcut works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Anywhere,
    Palette,
    NewWish,
    Lists,
    Checklist,
    Log,
    Message,
    Files,
    Graph,
    Columns,
    Menu,
    Mini,
}

impl Area {
    pub const ALL: [Area; 12] =
        [Area::Anywhere, Area::Palette, Area::NewWish, Area::Files, Area::Message, Area::Lists, Area::Checklist, Area::Graph, Area::Columns, Area::Log, Area::Menu, Area::Mini];

    pub fn title(self) -> &'static str {
        match self {
            Area::Anywhere => "Anywhere",
            Area::Palette => "Command palette",
            Area::NewWish => "New wish dialog",
            Area::Lists => "Wishes and Memory lists",
            Area::Checklist => "Checklist",
            Area::Log => "Event log",
            Area::Message => "Message to the run",
            Area::Files => "File list after @",
            Area::Graph => "Graph",
            Area::Columns => "Column splitter",
            Area::Menu => "Menu",
            Area::Mini => "Mini window shortcuts",
        }
    }
}

/// Shortcuts the shell binds itself, for the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Global {
    Palette,
    NewWish,
    Shortcuts,
    Undo,
    ToggleFocus,
    ToggleMini,
    ToggleDensity,
    ContextMenu,
}

pub struct Shortcut {
    pub area: Area,
    /// Alternatives, each in gpui's keystroke syntax.
    pub keys: &'static [&'static str],
    pub does: &'static str,
    /// Set when the shell binds these keys from this row, so the overlay and the binding agree.
    pub global: Option<Global>,
}

/// The platform's command modifier: Ctrl, or Cmd on macOS.
pub const fn primary(other: &'static str, mac: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

const fn row(area: Area, keys: &'static [&'static str], does: &'static str) -> Shortcut {
    Shortcut { area, keys, does, global: None }
}

/// Every keyboard shortcut the app has. The global ones are bound from here; the tests read the
/// sources of every screen and fail when a key handled there is missing from this table.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-k", "cmd-k")], does: "Search and run commands", global: Some(Global::Palette) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-n", "cmd-n")], does: "Make a new wish", global: Some(Global::NewWish) },
    Shortcut { area: Area::Anywhere, keys: &["?"], does: "Show keyboard shortcuts", global: Some(Global::Shortcuts) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-z", "cmd-z")], does: "Undo the last change", global: Some(Global::Undo) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-shift-f", "cmd-shift-f")], does: "Toggle focus mode", global: Some(Global::ToggleFocus) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-shift-m", "cmd-shift-m")], does: "Toggle mini window", global: Some(Global::ToggleMini) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-shift-d", "cmd-shift-d")], does: "Switch between comfortable and dense spacing", global: Some(Global::ToggleDensity) },
    Shortcut { area: Area::Anywhere, keys: &["shift-f10", "menu"], does: "Open the context menu", global: Some(Global::ContextMenu) },
    row(Area::Anywhere, &["tab", "shift-tab"], "Move between controls"),
    row(Area::Palette, &["up", "down"], "Move through the results"),
    row(Area::Palette, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Palette, &["enter"], "Go to the selected result"),
    row(Area::Palette, &["escape"], "Close"),
    row(Area::NewWish, &[primary("ctrl-enter", "cmd-enter")], "Start the wish"),
    row(Area::NewWish, &["enter"], "New line"),
    row(Area::NewWish, &["@"], "Point at a file or folder in the project"),
    row(Area::NewWish, &["escape"], "Close the dialog (asks before losing text)"),
    row(Area::Files, &["up", "down"], "Choose in the file list"),
    row(Area::Files, &["enter", "tab"], "Insert the chosen path"),
    row(Area::Files, &["escape"], "Close the file list"),
    row(Area::Message, &["enter"], "Send"),
    row(Area::Message, &["shift-enter"], "New line"),
    row(Area::Lists, &[primary("ctrl-f", "cmd-f")], "Search the list"),
    row(Area::Lists, &["down"], "From the search box, move into the list"),
    row(Area::Lists, &["home", "end"], "First or last row"),
    row(Area::Lists, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Lists, &["delete"], "Delete the selected memory (asks first)"),
    row(Area::Checklist, &["up", "down"], "Move through the checklist"),
    row(Area::Checklist, &["left", "right"], "Fold or unfold"),
    row(Area::Checklist, &["enter", "space"], "Select it everywhere"),
    row(Area::Checklist, &["home", "end"], "First or last item"),
    row(Area::Checklist, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Checklist, &["escape"], "Clear the selection"),
    row(Area::Graph, &["up", "down", "left", "right"], "Move between nodes"),
    row(Area::Graph, &["enter"], "Select the node"),
    row(Area::Graph, &["escape"], "Clear the selection"),
    row(Area::Graph, &["f"], "Fit the graph"),
    row(Area::Graph, &[primary("ctrl-=", "cmd-="), primary("ctrl-+", "cmd-+")], "Zoom in"),
    row(Area::Graph, &[primary("ctrl--", "cmd--")], "Zoom out"),
    row(Area::Graph, &[primary("ctrl-0", "cmd-0")], "Reset zoom"),
    row(Area::Columns, &["alt-left", "alt-right"], "Resize the checklist column"),
    row(Area::Columns, &["alt-shift-left", "alt-shift-right"], "Resize the details column"),
    row(Area::Columns, &["alt-0"], "Default column widths"),
    row(Area::Menu, &["up", "down"], "Move through the menu"),
    row(Area::Menu, &["enter"], "Choose the entry"),
    row(Area::Menu, &["escape"], "Close the menu"),
    row(Area::Mini, &[primary("ctrl-shift-m", "cmd-shift-m")], "Back to the full app"),
    row(Area::Log, &["up", "down"], "Scroll a line"),
    row(Area::Log, &["pageup", "pagedown"], "Scroll a page"),
    row(Area::Log, &["home"], "Go to the oldest line"),
    row(Area::Log, &["end"], "Follow new lines"),
    row(Area::Log, &["escape"], "Close the log"),
];

/// The key context in which `?` opens the overlay: anywhere but a text box, where it is typed.
pub const OUTSIDE_TEXT: &str = "!Input";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalBinding { pub global: Global, pub keys: &'static str, pub context: Option<&'static str> }
pub fn global_bindings() -> Vec<GlobalBinding> {
    SHORTCUTS.iter().filter_map(|s| s.global.map(|global| (global, s.keys)))
        .flat_map(|(global, keys)| keys.iter().map(move |&keys| GlobalBinding {
            global, keys, context: if matches!(global, Global::Shortcuts | Global::Undo | Global::ContextMenu) { Some(OUTSIDE_TEXT) } else { None }
        })).collect()
}

pub fn key_labels(keys: &str) -> Vec<String> {
    // `parse` accepts only static strings; the dynamic parser differs only in
    // accepting `secondary`, which the palette parser rejects.
    if keys.split('-').any(|part| part.eq_ignore_ascii_case("secondary")) { return vec![keys.to_owned()]; }
    let Ok(stroke) = Keystroke::parse_with_os(keys, OperatingSystem::Windows) else { return vec![keys.to_owned()] };
    let mut labels = Vec::new();
    for (on, name) in [(stroke.modifiers.control, "Ctrl"), (stroke.modifiers.alt, "Alt"), (stroke.modifiers.shift, "Shift"), (stroke.modifiers.platform, "Cmd")] {
        if on { labels.push(name.to_owned()); }
    }
    if stroke.modifiers.function { labels.push("Fn".to_owned()); }
    let label = match stroke.key.as_str() {
        "escape" => "Esc".to_owned(), "pageup" => "Page Up".to_owned(), "pagedown" => "Page Down".to_owned(),
        key => { let mut chars = key.chars(); chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default() }
    };
    labels.push(label);
    labels
}
pub fn key_cap(keys: &'static str, tokens: &Tokens) -> Kbd {
    key_labels(keys).into_iter().fold(Kbd::new(keys, tokens), |kbd, label| kbd.key(label))
}

/// The first keys of a global shortcut, to show beside the command it runs.
pub fn keys_of(global: Global) -> &'static str {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.global == Some(global))
        .and_then(|shortcut| shortcut.keys.first().copied())
        .unwrap_or_default()
}

pub fn in_area(area: Area) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS.iter().filter(move |shortcut| shortcut.area == area)
}

/// The sections and rows actually drawn by the shortcuts overlay.
pub fn overlay_rows() -> Vec<(Area, Vec<(&'static [&'static str], &'static str)>)> {
    Area::ALL.iter().map(|&area| (area, in_area(area).map(|s| (s.keys, s.does)).collect())).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    // --- research workspace (phase 9) ---
    Research,
    // --- end research workspace (phase 9) ---
    NewWish,
    Open(Screen),
    ToggleTheme,
    ToggleLanguage,
    Stop,
    Resume,
    Shortcuts,
    Focus(FocusPanel),
    FullWindow,
    Mini,
    ToggleDensity,
    ToggleOnTop,
    Undo,
    // ux: begin commands
    Storage, Chat, QuickTask, Plaza, UsageForWish, CompareUsage, MarkAllRead, ExtremeOn, ExtremeOff,
    // ux: end
}

/// Home, End, Page Up and Page Down for a list inside `context`.
pub fn bind_list_keys(context: &'static str, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("home", SelectFirst, Some(context)),
        KeyBinding::new("end", SelectLast, Some(context)),
        KeyBinding::new("pageup", SelectPageUp, Some(context)),
        KeyBinding::new("pagedown", SelectPageDown, Some(context)),
    ]);
}

pub fn select_row<D: ListDelegate>(
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

pub const MENU_LABELS: Table = &[
    ("Open", "開啟"),
    ("Resume", "接續"),
    ("Open folder", "開啟資料夾"),
    ("Copy session id", "複製工作階段 ID"),
    ("Open details", "開啟詳細資料"),
    ("Copy", "複製"),
    ("Comment on this item", "評論此項目"),
    ("Correct…", "修正…"),
    ("Delete…", "刪除…"),
];

pub fn action_label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::OpenRun => "Open",
        MenuAction::Resume => "Resume",
        MenuAction::OpenFolder => "Open folder",
        MenuAction::CopySessionId => "Copy session id",
        MenuAction::OpenDetail => "Open details",
        MenuAction::CopyText | MenuAction::CopyMemory => "Copy",
        MenuAction::CommentOn => "Comment on this item",
        MenuAction::CorrectMemory => "Correct…",
        MenuAction::DeleteMemory => "Delete…",
    }
}

pub fn action_icon(action: MenuAction) -> IconName {
    match action {
        MenuAction::OpenRun => IconName::ExternalLink,
        MenuAction::Resume => IconName::Play,
        MenuAction::OpenFolder => IconName::FolderOpen,
        MenuAction::CopySessionId | MenuAction::CopyText | MenuAction::CopyMemory => IconName::Copy,
        MenuAction::OpenDetail => IconName::Eye,
        MenuAction::CommentOn => IconName::FileText,
        MenuAction::CorrectMemory => IconName::Replace,
        MenuAction::DeleteMemory => IconName::CircleX,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem { pub action: MenuAction, pub label: &'static str, pub enabled: bool }
pub fn menu_items(state: &AppState, target: &MenuTarget) -> Vec<MenuItem> {
    state.menu_entries(target).into_iter().map(|(action, enabled)| MenuItem { action, label: action_label(action), enabled }).collect()
}

pub fn context_menu(
    menu: PopupMenu,
    target: MenuTarget,
    state: Entity<AppState>,
    run: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let entries = menu_items(state.read(cx), &target);
    if entries.is_empty() {
        return menu;
    }
    const CONTEXT_MENU_MIN_WIDTH: f32 = 200.;
    let mut menu = menu.min_w(px(CONTEXT_MENU_MIN_WIDTH));
    for MenuItem { action, enabled, .. } in entries {
        let state = state.clone();
        let target = target.clone();
        let run = run.clone();
        menu = menu.item(
            PopupMenuItem::new(tr(cx, MENU_LABELS, action_label(action)))
                .icon(action_icon(action))
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    if state.read(cx).menu_allowed(&target, action) {
                        run(action, target.clone(), window, cx);
                    }
                }),
        );
    }
    menu
}

/// Append run attention actions without extending the shared MenuAction contract.
pub fn append_read_actions(mut menu: PopupMenu, session: gpui_kit::SharedString, state: Entity<AppState>, cx: &mut Context<PopupMenu>) -> PopupMenu {
    use crate::desktop::model::facade::{settings::Store, work::{action_label, read_actions}};
    let actions = state.read(cx).runs.iter().find(|r| r.session == session)
        .map(|run| read_actions(run, &Store::current(cx).ux)).unwrap_or_default();
    let lang = *cx.global::<crate::desktop::i18n::Lang>();
    for action in actions {
        let state = state.clone(); let session = session.clone();
        menu = menu.item(PopupMenuItem::new(action_label(action, lang)).on_click(move |_, _, cx| {
            crate::desktop::model::facade::work::apply_read_action(&state, &session, action, cx);
        }));
    }
    menu
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEntry { pub kind: &'static str, pub needs_subject: bool }
pub fn registered_pages() -> Vec<PageEntry> {
    Page::KINDS.iter().map(|&kind| PageEntry { kind, needs_subject: Page::needs_subject(kind) }).collect()
}

#[cfg(test)]
#[path = "keys_accept.rs"]
mod accept;
