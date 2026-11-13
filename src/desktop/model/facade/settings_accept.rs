#![cfg(test)]
use super::*;
use std::any::TypeId;
use std::prelude::v1::test;

#[test]
fn store_preserves_unknown_and_repairs_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    std::fs::write(&path, r#"{"future":{"x":1}}"#).unwrap();
    let mut store = Store::at(Some(path.clone()));
    let pending = store.edit_pending(|s| s.plain_words = false).unwrap();
    pending.flush().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap()["future"]["x"], 1);
    assert!(!Store::at(Some(path.clone())).settings.plain_words);
    std::fs::write(&path, "{broken").unwrap();
    let mut store = Store::at(Some(path.clone()));
    assert_eq!(store.settings, Settings::default());
    store.edit_pending(|s| s.plain_words = false).unwrap().flush().unwrap();
    assert!(!Store::at(Some(path.clone())).settings.plain_words);
    serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap()).unwrap();
}

#[test]
fn view_memory_and_column_migration() {
    let mut s = Settings::parse(r#"{"columns":{"checklist":123,"details":456}}"#);
    assert_eq!(s.columns, Some(ColumnWidths { checklist: 123., details: 456. }));
    let m = s.view_memory("run.columns");
    assert_eq!((m.filter.as_deref(), m.sort.as_deref()), (Some("123"), Some("456")));
    s.with_view("run.columns", |m| m.scroll = 12.);
    s.with_view("bad/key", |m| m.scroll = 99.);
    assert_eq!(s.view_memory("run.columns").scroll, 12.);
    assert_eq!(s.view_memory("bad/key"), ViewMemory::default());
}

#[test]
fn all_prior_variants_restore() {
    let mut s = Settings::default();
    let original = s.clone();
    let priors = [s.set_theme(ThemeChoice::Dark), s.set_language(LanguageChoice::English), s.set_reduce_motion(Some(true)), s.set_plain_words(false), s.set_density(Density::Dense), s.set_mode(WindowMode::Mini), s.set_mini_on_top(true), s.set_board(BoardChoice::Off)];
    assert_eq!(priors[5], SettingPrior::Mode(WindowMode::Full));
    for p in priors { s.restore(p); }
    assert_eq!(s, original);
}

#[test]
fn prefs_are_the_same_types() {
    let prefs = UxPrefs::default();
    fn id<T: 'static>(_: &T) -> TypeId { TypeId::of::<T>() }
    assert_eq!(TypeId::of::<UxPrefs>(), TypeId::of::<crate::desktop::ux::prefs::UxPrefs>());
    assert_eq!(TypeId::of::<ReadMark>(), TypeId::of::<crate::desktop::ux::prefs::ReadMark>());
    assert_eq!(id(&prefs.read_filter), TypeId::of::<crate::desktop::ux::ReadFilter>());
    assert_eq!(TypeId::of::<crate::desktop::ux::prefs::ReadFilter>(), TypeId::of::<crate::desktop::ux::ReadFilter>());
    assert_eq!(id(&prefs.read), TypeId::of::<std::collections::BTreeMap<String, crate::desktop::ux::prefs::ReadMark>>());
}

#[test]
fn command_on_temp_path() {
    let dir = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) { "axiom-settings-fake.exe" } else { "axiom-settings-fake" };
    let path = dir.path().join(name);
    std::fs::write(&path, "fake").unwrap();
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap(); }
    let old = std::env::var_os("PATH");
    std::env::set_var("PATH", dir.path());
    let yes = command_exists("AXIOM_FACADE_TEST_COMMAND_UNSET", name);
    let no = command_exists("AXIOM_FACADE_TEST_COMMAND_UNSET", "axiom-no-such-command");
    match old { Some(p) => std::env::set_var("PATH", p), None => std::env::remove_var("PATH") }
    assert!(yes); assert!(!no);
}

#[test]
fn pure_behavior_matches_original() {
    use crate::desktop::settings as old;
    for input in ["{}", "invalid", r#"{"theme":"dark","future":7,"recent_projects":["a","b"],"ux":{"extreme":true}}"#] {
        assert_eq!(serde_json::from_str::<Value>(&Settings::parse(input).to_json()).unwrap(), serde_json::from_str::<Value>(&old::Settings::parse(input).to_json()).unwrap());
    }
    let a = ColumnWidths { checklist: 400., details: 900. }.clamped(1000., 100. .. 500., 100. .. 500., 300.);
    let b = old::ColumnWidths { checklist: 400., details: 900. }.clamped(1000., 100. .. 500., 100. .. 500., 300.);
    assert_eq!((a.checklist, a.details), (b.checklist, b.details));
    let rect = Bounds { origin: point(px(0.),px(0.)), size: size(px(1920.),px(1080.)) };
    let p = WindowPlacement { x:100., y:20., width:1200., height:800., maximized:true };
    let q = old::WindowPlacement { x:p.x, y:p.y, width:p.width, height:p.height, maximized:p.maximized };
    assert_eq!(p.restored(&[rect], size(px(800.),px(600.))), q.restored(&[rect], size(px(800.),px(600.))));
    for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
        assert_eq!(ThemeChoice::System.resolve(appearance), old::ThemeChoice::System.resolve(appearance).into());
        assert_eq!(ThemeChoice::Light.resolve(appearance), old::ThemeChoice::Light.resolve(appearance).into());
        assert_eq!(ThemeChoice::Dark.resolve(appearance), old::ThemeChoice::Dark.resolve(appearance).into());
    }
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
            syn::UseTree::Rename(n) => { path.push(n.ident.to_string()); let found = forbidden(path); path.pop(); found }
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
    let source = include_str!("settings.rs").split("#[cfg(test)]").next().unwrap();
    assert!(no_view_imports(source));
    assert!(!no_view_imports("use crate::desktop::settings::Store;"));
    assert!(!no_view_imports("use crate::desktop::{settings::Store};"));
}


#[test]
fn private_original_helpers_remain_identical() {
    let old = include_str!("../../settings.rs");
    let new = include_str!("settings.rs");
    for name in ["same_path", "push_recent", "recovered_bounds"] {
        fn helper<'a>(source: &'a str, name: &str) -> &'a str {
            let start = source.find(&format!("fn {name}(")).unwrap();
            let end = source[start..].find("\n}").unwrap() + start + 2;
            &source[start..end]
        }
        assert_eq!(helper(old, name), helper(new, name), "{name}");
    }
    let mut recent = Vec::new();
    for path in ["a", "b", "a"] { push_recent(&mut recent, path.into()); }
    assert_eq!(recent, vec![PathBuf::from("a"), PathBuf::from("b")]);
    assert!(same_path(Path::new("a"), Path::new("a")));
    assert!(!same_path(Path::new("a"), Path::new("b")));
}
