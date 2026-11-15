use super::*;
use std::prelude::v1::test;
use crate::desktop::{palette, demo, model::{MenuAction, MenuTarget}, ux::Page};

#[test]
fn legacy_context_menu_is_palettes() {
    assert_eq!(gpui_kit::Action::name(&LegacyContextMenu), "chronos::ContextMenu");
    assert_eq!(std::any::TypeId::of::<LegacyContextMenu>(), std::any::TypeId::of::<palette::ContextMenu>());
}

#[test]
fn context_menu_action() {
    assert_eq!(gpui_kit::Action::name(&ContextMenu), "chronos_keys::ContextMenu");
    let _binding = gpui_kit::KeyBinding::new("shift-f10", ContextMenu, Some("!Input"));
}

#[test]
fn context_menu_action_is_not_palettes() {
    let facade_name = gpui_kit::Action::name(&ContextMenu);
    let palette_name = gpui_kit::Action::name(&palette::ContextMenu);
    assert_eq!(palette_name, "chronos::ContextMenu");
    // The overlays task must switch the palette to the facade action.
    assert_ne!(facade_name, palette_name);
}

#[test]
fn shortcut_table_matches_owner() {
    assert_eq!(SHORTCUTS.len(), palette::SHORTCUTS.len());
    for (a, b) in SHORTCUTS.iter().zip(palette::SHORTCUTS) {
        assert_eq!(format!("{:?}", a.area), format!("{:?}", b.area));
        assert_eq!(a.keys, b.keys); assert_eq!(a.does, b.does);
        assert_eq!(format!("{:?}", a.global), format!("{:?}", b.global));
    }
    let overlays = overlay_rows();
    let old = palette::overlay_rows();
    assert_eq!(overlays.len(), old.len());
    for (i, (a, b)) in Area::ALL.iter().zip(palette::Area::ALL).enumerate() {
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(a.title(), b.title());
        assert_eq!(in_area(*a).count(), palette::in_area(b).count());
        assert_eq!(overlays[i].1, old[i].1);
    }
    let globals = [Global::Palette, Global::NewWish, Global::Shortcuts, Global::Undo, Global::ToggleFocus, Global::ToggleMini, Global::ToggleDensity, Global::ContextMenu];
    let old_globals = [palette::Global::Palette, palette::Global::NewWish, palette::Global::Shortcuts, palette::Global::Undo, palette::Global::ToggleFocus, palette::Global::ToggleMini, palette::Global::ToggleDensity, palette::Global::ContextMenu];
    for (a,b) in globals.into_iter().zip(old_globals) { assert_eq!(keys_of(a), palette::keys_of(b)); }
    let bindings = global_bindings();
    let old_bindings = palette::global_bindings();
    assert_eq!(bindings.len(), old_bindings.len());
    use gpui_kit::AsKeystroke;
    for (binding, old) in bindings.iter().zip(&old_bindings) { assert_eq!(gpui_kit::Keystroke::parse(binding.keys).unwrap(), *old.keystrokes()[0].as_keystroke()); }
    let expected: Vec<_> = SHORTCUTS.iter().filter_map(|s| s.global.map(|g| (g,s.keys))).flat_map(|(g, keys)| keys.iter().map(move |&k| (g,k))).collect();
    for (b,(g,k)) in bindings.iter().zip(expected) {
        assert_eq!((b.global,b.keys), (g,k));
        assert_eq!(b.context, if matches!(g, Global::Shortcuts | Global::Undo | Global::ContextMenu) { Some("!Input") } else { None });
    }
}

#[test]
fn labels() {
    for (input, expected) in [("ctrl-k", vec!["Ctrl", "K"]), ("?", vec!["?"]), ("shift-f10", vec!["Shift", "F10"]), ("alt-shift-left", vec!["Alt", "Shift", "Left"]), ("ctrl-=", vec!["Ctrl", "="]), ("bad-key-extra", vec!["bad-key-extra"]), ("pageup", vec!["Page Up"]), ("escape", vec!["Esc"]), ("secondary-k", vec!["secondary-k"])] {
        assert_eq!(key_labels(input), expected, "{input}");
    }
}

#[test]
fn menus_match_model_and_labels() {
    let state = demo::state(); let session = state.runs[0].session.clone();
    let targets = [MenuTarget::Run(session.clone()), MenuTarget::Module { session: session.clone(), module: state.runs[0].modules[0].id.clone() }, MenuTarget::Unit { session: session.clone(), unit: state.runs[0].units[0].id.clone() }, MenuTarget::Memory(41), MenuTarget::Module { session, module: "missing".into() }];
    for target in targets { let expected = state.menu_entries(&target); let items = menu_items(&state, &target); assert_eq!(items.len(), expected.len()); for (item,(action,enabled)) in items.iter().zip(expected) { assert_eq!((item.action,item.enabled), (action,enabled)); assert_eq!(item.label, palette::menu::action_label(action)); } }
    use MenuAction::*;
    for action in [OpenRun,Resume,OpenFolder,CopySessionId,OpenDetail,CopyText,CopyMemory,CommentOn,CorrectMemory,DeleteMemory] { assert_eq!(action_label(action), palette::menu::action_label(action)); }
}

fn missing_pages(entries: &[PageEntry], kinds: &[&str]) -> Vec<String> { kinds.iter().filter(|k| !entries.iter().any(|e| e.kind == **k)).map(|k| (*k).to_owned()).collect() }
#[test]
fn pages_and_negative_control() {
    let entries = registered_pages();
    assert_eq!(entries.iter().map(|e| e.kind).collect::<Vec<_>>(), Page::KINDS);
    for e in &entries { assert_eq!(e.needs_subject, matches!(e.kind, "session" | "paper")); }
    assert!(missing_pages(&entries, Page::KINDS).is_empty());
    assert_eq!(missing_pages(&entries[..entries.len()-1], Page::KINDS), vec![entries.last().unwrap().kind]);
}

#[test]
fn command_variants_match() {
    let ours = [Command::Research,Command::NewWish,Command::Open(crate::desktop::model::Screen::Wishes),Command::ToggleTheme,Command::ToggleLanguage,Command::Stop,Command::Resume,Command::Shortcuts,Command::Focus(crate::desktop::model::facade::settings::FocusPanel::Graph),Command::FullWindow,Command::Mini,Command::ToggleDensity,Command::ToggleOnTop,Command::Undo,Command::Storage,Command::Chat,Command::QuickTask,Command::Plaza,Command::UsageForWish,Command::CompareUsage,Command::MarkAllRead,Command::ExtremeOn,Command::ExtremeOff];
    let old = [palette::Command::Research,palette::Command::NewWish,palette::Command::Open(crate::desktop::model::Screen::Wishes),palette::Command::ToggleTheme,palette::Command::ToggleLanguage,palette::Command::Stop,palette::Command::Resume,palette::Command::Shortcuts,palette::Command::Focus(crate::desktop::settings::FocusPanel::Graph),palette::Command::FullWindow,palette::Command::Mini,palette::Command::ToggleDensity,palette::Command::ToggleOnTop,palette::Command::Undo,palette::Command::Storage,palette::Command::Chat,palette::Command::QuickTask,palette::Command::Plaza,palette::Command::UsageForWish,palette::Command::CompareUsage,palette::Command::MarkAllRead,palette::Command::ExtremeOn,palette::Command::ExtremeOff];
    fn count(source: &str) -> usize {
        let tree = syn::parse_file(source).unwrap();
        tree.items.iter().find_map(|item| match item { syn::Item::Enum(e) if e.ident == "Command" => Some(e.variants.len()), _ => None }).unwrap()
    }
    assert_eq!(ours.len(), count(include_str!("keys.rs")));
    assert_eq!(old.len(), count(include_str!("../../palette.rs")));
    assert_eq!(ours.len(), old.len()); for (a,b) in ours.iter().zip(old) { assert_eq!(format!("{a:?}"),format!("{b:?}")); }
}

fn no_view_imports(source: &str) -> bool {
    use syn::visit::{self, Visit};
    fn forbidden(parts: &[String]) -> bool {
        let rest = if parts.starts_with(&["crate".into(), "desktop".into()]) { &parts[2..] }
            else if parts.first().is_some_and(|p| p == "super" || p == "desktop") { &parts[1..] }
            else { return false };
        let names = ["settings", "palette", "wishes", "control", "detail", "usage", "quota", "plain_text", "evolve", "memory", "doctor", "run_view", "todo", "mesh", "shell", "composer", "mini", "superswitch", "research", "paper", "launch"];
        rest.first().is_some_and(|p| names.contains(&p.as_str())) ||
            rest.first().is_some_and(|p| p == "ux") && rest.get(1).is_some_and(|p| p == "celestial" || p == "composer")
    }
    fn uses(tree: &syn::UseTree, path: &mut Vec<String>) -> bool {
        match tree {
            syn::UseTree::Path(p) => { path.push(p.ident.to_string()); let found = uses(&p.tree, path); path.pop(); found }
            syn::UseTree::Name(n) => { path.push(n.ident.to_string()); let found = forbidden(path); path.pop(); found }
            syn::UseTree::Rename(n) => {
                path.push(n.ident.to_string());
                let allowed = path.iter().map(String::as_str).eq(["crate", "desktop", "palette", "ContextMenu"])
                    && n.rename == "LegacyContextMenu";
                let found = !allowed && forbidden(path);
                path.pop(); found
            }
            syn::UseTree::Glob(_) => forbidden(path),
            syn::UseTree::Group(g) => g.items.iter().any(|item| uses(item, path)),
        }
    }
    struct Scan(bool);
    impl<'ast> Visit<'ast> for Scan {
        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) { self.0 |= uses(&item.tree, &mut Vec::new()); }
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.0 |= forbidden(&path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>());
            visit::visit_path(self, path);
        }
    }
    let mut scan = Scan(false);
    scan.visit_file(&syn::parse_file(source).unwrap());
    !scan.0
}
#[test]
fn no_view_paths_and_negative_control() {
    assert!(no_view_imports(include_str!("keys.rs")));
    assert!(!no_view_imports("use crate::desktop::palette::Area;"));
    assert!(!no_view_imports("use crate::desktop::palette::ContextMenu;"));
    assert!(!no_view_imports("use crate::desktop::palette::Area as LegacyContextMenu;"));
    assert!(!no_view_imports("use crate::desktop::palette::ContextMenu as Other;"));
}
