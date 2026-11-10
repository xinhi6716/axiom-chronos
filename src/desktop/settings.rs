//! Preferences that survive a restart, kept in one JSON file in the data folder. A missing or
//! damaged file means defaults; demo and off-screen runs neither read nor write it.
use super::i18n::{self, Lang};
use super::model::{AppState, Screen};
use super::theme;
use crate::base;
use gpui_kit::component::ThemeMode;
use gpui_kit::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FILE_NAME: &str = "app-settings.json";
const MAX_RECENT_PROJECTS: usize = 12;
/// Long enough that dragging a window does not write on every step.
const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LanguageChoice {
    #[default]
    System,
    ZhTw,
    English,
}

impl LanguageChoice {
    pub fn resolve(self) -> Lang {
        match self {
            Self::System => i18n::system_lang(),
            Self::ZhTw => Lang::ZhTw,
            Self::English => Lang::En,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn resolve(self, system: WindowAppearance) -> ThemeMode {
        match self {
            Self::System => system.into(),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        }
    }
}

const LANGUAGE_KEYS: [(LanguageChoice, &str); 3] = [
    (LanguageChoice::System, "system"),
    (LanguageChoice::ZhTw, "zh-TW"),
    (LanguageChoice::English, "en"),
];
const THEME_KEYS: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "system"),
    (ThemeChoice::Light, "light"),
    (ThemeChoice::Dark, "dark"),
];
const SCREEN_KEYS: [(Screen, &str); 5] = [
    (Screen::Run, "run"),
    (Screen::Wishes, "wishes"),
    (Screen::Memory, "memory"),
    (Screen::Usage, "usage"),
    (Screen::Doctor, "settings"),
];

fn key_of<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(v, _)| *v == value).map_or("", |(_, key)| key)
}

fn value_of<T: Copy>(table: &[(T, &'static str)], key: Option<&str>) -> Option<T> {
    table.iter().find(|(_, k)| Some(*k) == key).map(|(v, _)| *v)
}

/// The window's restore rectangle, and whether it opens maximized over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl WindowPlacement {
    pub fn of(bounds: WindowBounds) -> Self {
        let rect = bounds.get_bounds();
        Self {
            x: rect.origin.x.as_f32(),
            y: rect.origin.y.as_f32(),
            width: rect.size.width.as_f32(),
            height: rect.size.height.as_f32(),
            maximized: matches!(bounds, WindowBounds::Maximized(_)),
        }
    }

    fn parse(value: &Value) -> Option<Self> {
        let number = |key: &str| value.get(key)?.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
        let (width, height) = (number("width")?, number("height")?);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(Self {
            x: number("x")?,
            y: number("y")?,
            width,
            height,
            maximized: value.get("maximized").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// Where to reopen, moved fully onto the display it overlaps most. `None` when the title
    /// strip would not be reachable on any connected display.
    pub fn restored(&self, displays: &[Bounds<Pixels>], min: Size<Pixels>) -> Option<WindowBounds> {
        let saved = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width).max(min.width), px(self.height).max(min.height)),
        };
        let strip = Bounds {
            origin: saved.origin,
            size: size(saved.size.width, px(TITLE_STRIP_HEIGHT)),
        };
        let overlap = |display: &Bounds<Pixels>| {
            let shared = display.intersect(&strip);
            shared.size.width.max(px(0.)).as_f32() * shared.size.height.max(px(0.)).as_f32()
        };
        let display = displays
            .iter()
            .filter(|d| overlap(d) >= MIN_REACHABLE_STRIP)
            .max_by(|a, b| overlap(a).total_cmp(&overlap(b)))?;
        let width = saved.size.width.min(display.size.width).max(min.width);
        let height = saved.size.height.min(display.size.height).max(min.height);
        let origin = point(
            saved.origin.x.min(display.right() - width).max(display.left()),
            saved.origin.y.min(display.bottom() - height).max(display.top()),
        );
        let rect = Bounds { origin, size: size(width, height) };
        Some(if self.maximized { WindowBounds::Maximized(rect) } else { WindowBounds::Windowed(rect) })
    }
}

const TITLE_STRIP_HEIGHT: f32 = 34.0;
/// A strip about 120 by 24 pixels: enough to grab and drag.
const MIN_REACHABLE_STRIP: f32 = 2880.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub language: LanguageChoice,
    pub theme: ThemeChoice,
    pub plain_words: bool,
    /// `None` follows the system's own setting.
    pub reduce_motion: Option<bool>,
    pub window: Option<WindowPlacement>,
    pub last_screen: Screen,
    /// Most recent first.
    pub recent_projects: Vec<PathBuf>,
    /// The first-day checklist was completed once; it never comes back.
    pub first_day_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LanguageChoice::default(),
            theme: ThemeChoice::default(),
            plain_words: true,
            reduce_motion: None,
            window: None,
            last_screen: Screen::Run,
            recent_projects: vec![],
            first_day_done: false,
        }
    }
}

impl Settings {
    /// Reads what it can: a damaged value costs only that value, a damaged file only the defaults.
    pub fn parse(text: &str) -> Self {
        let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or(Value::Null);
        let word = |key: &str| root.get(key).and_then(Value::as_str);
        Self {
            language: value_of(&LANGUAGE_KEYS, word("language")).unwrap_or_default(),
            theme: value_of(&THEME_KEYS, word("theme")).unwrap_or_default(),
            plain_words: root.get("plain_words").and_then(Value::as_bool).unwrap_or(true),
            reduce_motion: root.get("reduce_motion").and_then(Value::as_bool),
            window: root.get("window").and_then(WindowPlacement::parse),
            last_screen: value_of(&SCREEN_KEYS, word("last_screen")).unwrap_or(Screen::Run),
            recent_projects: root
                .get("recent_projects")
                .and_then(Value::as_array)
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|path| !path.is_empty())
                        .take(MAX_RECENT_PROJECTS)
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            first_day_done: root.get("first_day_done").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    pub fn to_json(&self) -> String {
        let window = self.window.map(|w| {
            json!({ "x": w.x, "y": w.y, "width": w.width, "height": w.height, "maximized": w.maximized })
        });
        let recent: Vec<_> = self.recent_projects.iter().map(|p| p.to_string_lossy()).collect();
        let body = json!({
            "language": key_of(&LANGUAGE_KEYS, self.language),
            "theme": key_of(&THEME_KEYS, self.theme),
            "plain_words": self.plain_words,
            "reduce_motion": self.reduce_motion,
            "window": window,
            "last_screen": key_of(&SCREEN_KEYS, self.last_screen),
            "recent_projects": recent,
            "first_day_done": self.first_day_done,
        });
        serde_json::to_string_pretty(&body).unwrap_or_default()
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let folded = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        folded(a) == folded(b)
    } else {
        a == b
    }
}

fn push_recent(list: &mut Vec<PathBuf>, path: PathBuf) {
    list.retain(|known| !same_path(known, &path));
    list.insert(0, path);
    list.truncate(MAX_RECENT_PROJECTS);
}

fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).map(|text| Settings::parse(&text)).unwrap_or_default()
}

fn save(path: &Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    base::atomic_write(path, json.as_bytes())
}

/// The loaded settings and where they go. With no path nothing is ever written.
pub struct Store {
    pub settings: Settings,
    path: Option<PathBuf>,
    version: u64,
    /// The newest version on disk. Holding the lock is what keeps writes one at a time.
    written: Arc<Mutex<u64>>,
    pending: Option<Task<()>>,
    error: Option<SharedString>,
}

impl Global for Store {}

impl Store {
    pub fn open(ephemeral: bool) -> Self {
        let path = (!ephemeral).then(|| base::axiom_home().join(FILE_NAME));
        Self::at(path)
    }

    fn at(path: Option<PathBuf>) -> Self {
        Self {
            settings: path.as_deref().map(load).unwrap_or_default(),
            path,
            version: 0,
            written: Arc::new(Mutex::new(0)),
            pending: None,
            error: None,
        }
    }

    pub fn current(cx: &App) -> &Settings {
        &cx.global::<Self>().settings
    }

    pub fn last_error(cx: &App) -> Option<SharedString> {
        cx.global::<Self>().error.clone()
    }

    /// Applies a change and, when it changed anything, saves shortly after on a background thread.
    pub fn edit(cx: &mut App, change: impl FnOnce(&mut Settings)) {
        let store = cx.global_mut::<Self>();
        let before = store.settings.clone();
        change(&mut store.settings);
        if store.settings == before || store.path.is_none() {
            return;
        }
        store.version += 1;
        let pending = cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Some(write) = cx.update(Self::write) else { return };
            let outcome = write.await.err().map(SharedString::from);
            cx.update(|cx| {
                let store = cx.global_mut::<Self>();
                if store.error != outcome {
                    store.error = outcome;
                    cx.refresh_windows();
                }
            });
        });
        cx.global_mut::<Self>().pending = Some(pending);
    }

    fn write(cx: &mut App) -> Option<Task<Result<(), String>>> {
        let store = cx.global::<Self>();
        let path = store.path.clone()?;
        let (json, version, written) = (store.settings.to_json(), store.version, store.written.clone());
        Some(cx.background_spawn(async move {
            let mut newest = written.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if version <= *newest {
                return Ok(());
            }
            save(&path, &json).map_err(|error| error.to_string())?;
            *newest = version;
            Ok(())
        }))
    }

    /// Flushes a change that is still waiting for its delay when the app exits.
    pub fn save_on_quit(cx: &mut App) {
        cx.on_app_quit(|cx| {
            let write = Self::write(cx);
            async move {
                if let Some(write) = write {
                    let _ = write.await;
                }
            }
        })
        .detach();
    }
}

pub fn set_language(choice: LanguageChoice, window: &mut Window, cx: &mut App) {
    Store::edit(cx, |s| s.language = choice);
    i18n::set(choice.resolve(), cx);
    window.refresh();
}

pub fn set_theme(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    Store::edit(cx, |s| s.theme = choice);
    apply_theme(window, cx);
}

fn apply_theme(window: &mut Window, cx: &mut App) {
    let mode = Store::current(cx).theme.resolve(window.appearance());
    theme::apply(mode, Some(window), cx);
    window.refresh();
}

pub fn system_appearance_changed(window: &mut Window, cx: &mut App) {
    if Store::current(cx).theme == ThemeChoice::System {
        apply_theme(window, cx);
    }
}

pub fn set_reduce_motion(on: bool, state: &Entity<AppState>, cx: &mut App) {
    Store::edit(cx, |s| s.reduce_motion = Some(on));
    cx.set_reduce_motion(on);
    state.update(cx, |state, cx| {
        state.reduce_motion = on;
        cx.notify();
    });
}

pub fn remember_window(bounds: WindowBounds, cx: &mut App) {
    Store::edit(cx, |s| s.window = Some(WindowPlacement::of(bounds)));
}

pub fn remember_screen(screen: Screen, cx: &mut App) {
    Store::edit(cx, |s| s.last_screen = screen);
}

pub fn remember_project(path: PathBuf, cx: &mut App) {
    Store::edit(cx, |s| push_recent(&mut s.recent_projects, path));
}

/// Records that the first-day checklist is complete, for good.
pub fn finish_first_day(cx: &mut App) {
    Store::edit(cx, |s| s.first_day_done = true);
}

pub fn first_day_done(cx: &App) -> bool {
    Store::current(cx).first_day_done
}

pub fn has_recent_project(cx: &App) -> bool {
    !Store::current(cx).recent_projects.is_empty()
}

// called by the screens
#[allow(dead_code)]
pub fn recent_projects(cx: &App) -> &[PathBuf] {
    &Store::current(cx).recent_projects
}

/// Where the window opens: its saved spot if a connected display can still show it, otherwise centered.
pub fn opening_bounds(settings: &Settings, default: Size<Pixels>, min: Size<Pixels>, cx: &App) -> WindowBounds {
    let displays: Vec<_> = cx.displays().iter().map(|d| d.visible_bounds()).collect();
    settings
        .window
        .and_then(|saved| saved.restored(&displays, min))
        .unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(None, default, cx)))
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    fn sample() -> Settings {
        Settings {
            language: LanguageChoice::English,
            theme: ThemeChoice::Light,
            plain_words: false,
            reduce_motion: Some(true),
            window: Some(WindowPlacement { x: -40.0, y: 12.5, width: 1280.0, height: 800.0, maximized: true }),
            last_screen: Screen::Doctor,
            recent_projects: vec![PathBuf::from("E:\\codex\\one"), PathBuf::from("/home/me/two")],
            first_day_done: true,
        }
    }

    fn display(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(x), px(y)), size: size(px(width), px(height)) }
    }

    fn minimum() -> Size<Pixels> {
        size(px(1100.), px(700.))
    }

    fn placement(x: f32, y: f32, width: f32, height: f32) -> WindowPlacement {
        WindowPlacement { x, y, width, height, maximized: false }
    }

    #[test]
    fn every_setting_survives_a_round_trip() {
        assert_eq!(Settings::parse(&sample().to_json()), sample());
    }

    #[test]
    fn plain_words_defaults_on_and_round_trips() {
        assert!(Settings::parse("{}").plain_words);
        assert!(Settings::parse(r#"{"plain_words":"no"}"#).plain_words);
        assert!(!Settings::parse(&sample().to_json()).plain_words);
        assert!(Settings::parse(&Settings::default().to_json()).plain_words);
    }

    #[test]
    fn missing_and_corrupt_text_give_defaults() {
        for text in ["", "not json", "[1, 2]", "{\"language\":", "null"] {
            assert_eq!(Settings::parse(text), Settings::default(), "{text}");
        }
    }

    #[test]
    fn the_first_day_list_stays_gone_once_completed_and_a_bad_value_does_not_retire_it() {
        assert!(!Settings::default().first_day_done);
        assert!(Settings::parse(r#"{"first_day_done": true}"#).first_day_done);
        assert!(!Settings::parse(r#"{"first_day_done": "yes"}"#).first_day_done);
        let mut settings = Settings::default();
        settings.first_day_done = true;
        assert!(Settings::parse(&settings.to_json()).first_day_done);
    }

    #[test]
    fn a_damaged_value_costs_only_that_value() {
        let text = r#"{"language":"klingon","theme":"dark","reduce_motion":"yes",
            "window":{"x":"left","y":0,"width":1200,"height":800},
            "last_screen":12,"recent_projects":["E:\\a","",7]}"#;
        let settings = Settings::parse(text);
        assert_eq!(settings.language, LanguageChoice::System);
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.reduce_motion, None);
        assert_eq!(settings.window, None);
        assert_eq!(settings.last_screen, Screen::Run);
        assert_eq!(settings.recent_projects, vec![PathBuf::from("E:\\a")]);
    }

    #[test]
    fn a_byte_order_mark_is_not_damage() {
        assert_eq!(Settings::parse("\u{feff}{\"theme\":\"light\"}").theme, ThemeChoice::Light);
    }

    #[test]
    fn load_tolerates_a_missing_and_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(load(&path), Settings::default());
        std::fs::write(&path, [0xff, 0xfe, 0x00, 0x7b]).unwrap();
        assert_eq!(load(&path), Settings::default());
    }

    #[test]
    fn save_creates_the_folder_and_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);
        save(&path, &sample().to_json()).unwrap();
        save(&path, &sample().to_json()).unwrap();
        assert_eq!(load(&path), sample());
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)]);
    }

    #[test]
    fn a_store_without_a_path_reads_nothing_and_has_nowhere_to_write() {
        let store = Store::at(None);
        assert_eq!(store.settings, Settings::default());
        assert!(store.path.is_none());
        assert!(Store::open(true).path.is_none());
    }

    #[test]
    fn a_store_with_a_path_loads_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        save(&path, &sample().to_json()).unwrap();
        assert_eq!(Store::at(Some(path)).settings, sample());
    }

    #[test]
    fn recent_projects_are_newest_first_without_repeats_and_bounded() {
        let mut list = vec![];
        for n in 0..MAX_RECENT_PROJECTS + 3 {
            push_recent(&mut list, PathBuf::from(format!("/p/{n}")));
        }
        assert_eq!(list.len(), MAX_RECENT_PROJECTS);
        assert_eq!(list[0], PathBuf::from(format!("/p/{}", MAX_RECENT_PROJECTS + 2)));
        push_recent(&mut list, PathBuf::from("/p/8"));
        assert_eq!(list[0], PathBuf::from("/p/8"));
        assert_eq!(list.iter().filter(|p| **p == PathBuf::from("/p/8")).count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_without_case_or_slash_differences() {
        let mut list = vec![PathBuf::from("E:\\Codex\\Proj")];
        push_recent(&mut list, PathBuf::from("e:/codex/proj"));
        assert_eq!(list, vec![PathBuf::from("e:/codex/proj")]);
    }

    #[test]
    fn theme_choice_follows_the_system_only_when_asked() {
        assert_eq!(ThemeChoice::System.resolve(WindowAppearance::Dark), ThemeMode::Dark);
        assert_eq!(ThemeChoice::System.resolve(WindowAppearance::Light), ThemeMode::Light);
        assert_eq!(ThemeChoice::Light.resolve(WindowAppearance::Dark), ThemeMode::Light);
        assert_eq!(ThemeChoice::Dark.resolve(WindowAppearance::Light), ThemeMode::Dark);
    }

    #[test]
    fn a_visible_window_reopens_where_it_was() {
        let screens = [display(0., 0., 2560., 1400.)];
        let restored = placement(100., 80., 1300., 900.).restored(&screens, minimum());
        assert_eq!(
            restored,
            Some(WindowBounds::Windowed(Bounds { origin: point(px(100.), px(80.)), size: size(px(1300.), px(900.)) }))
        );
    }

    #[test]
    fn a_maximized_window_reopens_maximized_over_its_restore_rectangle() {
        let screens = [display(0., 0., 2560., 1400.)];
        let saved = WindowPlacement { maximized: true, ..placement(100., 80., 1300., 900.) };
        assert!(matches!(saved.restored(&screens, minimum()), Some(WindowBounds::Maximized(_))));
    }

    #[test]
    fn a_window_on_a_disconnected_display_is_not_restored() {
        let screens = [display(0., 0., 1920., 1040.)];
        assert_eq!(placement(3000., 200., 1300., 900.).restored(&screens, minimum()), None);
        assert_eq!(placement(-32000., -32000., 1300., 900.).restored(&screens, minimum()), None);
        assert_eq!(placement(100., 100., 1300., 900.).restored(&[], minimum()), None);
    }

    #[test]
    fn a_window_hanging_off_a_display_is_brought_fully_onto_it() {
        let screens = [display(0., 0., 1920., 1040.)];
        let restored = placement(1400., 500., 1300., 900.).restored(&screens, minimum()).unwrap().get_bounds();
        assert_eq!(restored.origin, point(px(620.), px(140.)));
        assert_eq!(restored.size, size(px(1300.), px(900.)));
    }

    #[test]
    fn a_saved_size_is_held_to_the_minimum_and_to_the_display() {
        let screens = [display(0., 0., 1920., 1040.)];
        let small = placement(10., 10., 400., 300.).restored(&screens, minimum()).unwrap().get_bounds();
        assert_eq!(small.size, minimum());
        let large = placement(0., 0., 3000., 2000.).restored(&screens, minimum()).unwrap().get_bounds();
        assert_eq!(large.size, size(px(1920.), px(1040.)));
    }
}
