use super::super::i18n::{tr, Table};
use super::super::model::{AppState, MenuAction, MenuTarget};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::IconName;
use gpui_kit::*;
use std::rc::Rc;

pub(super) const T: Table = &[
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

pub(in crate::desktop) fn action_label(action: MenuAction) -> &'static str {
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

fn action_icon(action: MenuAction) -> IconName {
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

pub(super) fn entries_for(state: &AppState, target: &MenuTarget) -> Vec<(MenuAction, bool)> {
    state.menu_entries(target)
}

pub(super) fn click_runs(state: &AppState, target: &MenuTarget, action: MenuAction) -> bool {
    state.menu_allowed(target, action)
}

pub(in crate::desktop) fn context_menu(
    menu: PopupMenu,
    target: MenuTarget,
    state: Entity<AppState>,
    run: Rc<dyn Fn(MenuAction, MenuTarget, &mut Window, &mut App)>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let entries = entries_for(state.read(cx), &target);
    if entries.is_empty() {
        return menu;
    }
    let mut menu = menu.min_w(px(200.));
    for (action, enabled) in entries {
        let state = state.clone();
        let target = target.clone();
        let run = run.clone();
        menu = menu.item(
            PopupMenuItem::new(tr(cx, T, action_label(action)))
                .icon(action_icon(action))
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    if click_runs(state.read(cx), &target, action) {
                        run(action, target.clone(), window, cx);
                    }
                }),
        );
    }
    menu
}
