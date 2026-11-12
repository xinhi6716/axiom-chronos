#![allow(dead_code)]
use axiom::desktop::model::{
    AppState, LogLevel, LogLine, Phase, RunStatus, Screen, Selection, Stage, UnitState,
};
use axiom::desktop::{
    demo,
    i18n::{self, Lang},
    theme, Shell, Store,
};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext,
    WindowBounds, WindowOptions,
};
use std::path::Path;

pub struct Scenario {
    pub id: &'static str,
    pub build: fn() -> AppState,
    pub screen: Screen,
    pub action: Option<fn(&mut TestAppContext, AnyWindowHandle, &Entity<AppState>)>,
}

fn run_with_log(text: &'static str) -> AppState {
    let mut state = demo::state();
    let run = &mut state.runs[state.current.unwrap()];
    run.log = vec![LogLine {
        time: "09:52:00".into(),
        source: "chronos".into(),
        text: text.into(),
        level: LogLevel::Warn,
    }];
    run.warnings.clear();
    run.notes.clear();
    run.decisions.clear();
    state
}
fn planning() -> AppState {
    let mut s = demo::state();
    let r = &mut s.runs[s.current.unwrap()];
    r.stages = vec![Stage {
        name: "Astra".into(),
        phase: Phase::Running,
        note: None,
    }];
    r.units.clear();
    r.modules.clear();
    r.goals.clear();
    s
}
fn between_modules() -> AppState {
    let mut s = run_with_log(r#"{"type":"chronos","subtype":"reviewing","module":"Core"}"#);
    for unit in &mut s.runs[s.current.unwrap()].units {
        if unit.state == UnitState::Running {
            unit.state = UnitState::Built;
        }
    }
    s
}
fn council() -> AppState {
    let mut s = planning();
    s.runs[s.current.unwrap()].log = vec![LogLine {
        time: "09:52:00".into(),
        source: "council".into(),
        text: r#"{"type":"council","subtype":"round","round":1}"#.into(),
        level: LogLevel::Info,
    }];
    s
}
fn max_turns() -> AppState {
    let mut s = run_with_log(r#"{"type":"chronos","subtype":"max_turns","turn_limit":40}"#);
    s.runs[s.current.unwrap()].status = RunStatus::Interrupted;
    s
}
fn degraded_council() -> AppState {
    run_with_log(r#"{"type":"council","subtype":"member_degraded","member":"Fable"}"#)
}
fn scope_failure() -> AppState {
    run_with_log(
        r#"{"type":"codex","subtype":"tool_result","outside_scope":["src/outside.rs"],"scope_ignored":true}"#,
    )
}
fn raw_events() -> AppState {
    run_with_log(r#"{"type":"chronos","subtype":"max_turns","turn_limit":40}"#)
}
fn stale_runs() -> AppState {
    let mut s = demo::state();
    s.runs[s.current.unwrap()].status = RunStatus::Interrupted;
    s
}
fn usage_per_wish() -> AppState {
    demo::state()
}
fn who_running() -> AppState {
    demo::state()
}
fn closing() -> AppState {
    demo::state()
}
fn request_close(cx: &mut TestAppContext, window: AnyWindowHandle, _: &Entity<AppState>) {
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(
        !visual.simulate_close(),
        "the running wish opens the close dialog"
    );
    cx.run_until_parked();
}
fn unit_actions() -> AppState {
    let mut s = demo::state();
    let run = &s.runs[s.current.unwrap()];
    s.selection = Selection::Unit(
        run.units
            .iter()
            .position(|u| u.state == UnitState::Failed)
            .unwrap(),
    );
    s
}

pub fn scenarios() -> Vec<Scenario> {
    [
        ("UX01", planning as fn() -> AppState, Screen::Run),
        ("UX02", between_modules, Screen::Run),
        ("UX03", council, Screen::Run),
        ("UX04", max_turns, Screen::Run),
        ("UX05", degraded_council, Screen::Run),
        ("UX06", scope_failure, Screen::Run),
        ("UX07", raw_events, Screen::Run),
        ("UX08", stale_runs, Screen::Wishes),
        ("UX09", usage_per_wish, Screen::Usage),
        ("UX10", who_running, Screen::Run),
        ("UX11", closing, Screen::Run),
        ("UX12", unit_actions, Screen::Run),
    ]
    .into_iter()
    .map(|(id, build, screen)| Scenario {
        id,
        build,
        screen,
        action: if id == "UX11" {
            Some(request_close)
        } else {
            None
        },
    })
    .collect()
}

pub fn open(
    cx: &mut TestAppContext,
    lang: Lang,
    mut state: AppState,
    screen: Screen,
) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(lang, cx);
        cx.set_global(Store::open(true));
    });
    state.view = screen;
    let state = cx.new(|_| state);
    let bounds = Bounds {
        origin: Point::default(),
        size: size(px(1440.), px(900.)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let shell_state = state.clone();
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Shell::new(shell_state, false, window, cx))
            })
        })
        .expect("open Shell");
    for _ in 0..4 {
        frame(cx, window);
    }
    (window, state)
}
fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .expect("render frame");
    cx.run_until_parked();
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureLine {
    pub path: Vec<String>,
    pub text: String,
}
pub fn capture(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<CaptureLine> {
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        gpui_kit::base::test_support::texts(window)
            .into_iter()
            .filter(|t| t.visible())
            .map(|t| CaptureLine {
                path: t
                    .path()
                    .iter()
                    .filter_map(|id| match id {
                        ElementId::Name(n) | ElementId::NamedInteger(n, _) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect(),
                text: t.text().replace(['\t', '\r', '\n'], " "),
            })
            .collect()
    })
    .expect("capture visible text")
}
fn filename(dir: &Path, id: &str, lang: Lang) -> std::path::PathBuf {
    dir.join(format!(
        "{id}.{}.txt",
        match lang {
            Lang::En => "en",
            Lang::ZhTw => "zh-TW",
        }
    ))
}
pub fn write_capture(dir: &Path, id: &str, lang: Lang, lines: &[CaptureLine]) {
    std::fs::create_dir_all(dir).expect("create capture directory");
    let data: String = lines
        .iter()
        .map(|line| format!("{}\t{}\n", line.path.join("/"), line.text))
        .collect();
    std::fs::write(filename(dir, id, lang), data).expect("write capture");
}
pub fn read_capture(dir: &Path, id: &str, lang: Lang) -> Option<Vec<CaptureLine>> {
    let data = std::fs::read_to_string(filename(dir, id, lang)).ok()?;
    Some(
        data.lines()
            .map(|line| {
                let (path, text) = line.split_once('\t').expect("malformed capture line");
                CaptureLine {
                    path: if path.is_empty() {
                        vec![]
                    } else {
                        path.split('/').map(str::to_owned).collect()
                    },
                    text: text.to_owned(),
                }
            })
            .collect(),
    )
}
pub fn shows(lines: &[CaptureLine], element: &str, marker: &str) -> bool {
    lines
        .iter()
        .any(|line| line.path.iter().any(|id| id == element) && line.text.contains(marker))
}
pub fn newest_log_is_raw_json(state: &AppState) -> bool {
    state
        .current
        .and_then(|current| state.runs.get(current))
        .and_then(|run| run.log.last())
        .is_some_and(|line| line.text.starts_with("{\""))
}
#[derive(Debug)]
pub struct ChecklistRow {
    pub id: String,
    pub title: String,
    pub scenario: String,
    pub element: String,
    pub after_en: String,
    pub after_zh_tw: String,
    pub before_lacks: String,
}
pub fn checklist() -> Vec<ChecklistRow> {
    let text = include_str!("../../ux-checklist.md");
    let mut parts = text.split("\n## ");
    assert_eq!(
        parts.next().unwrap().trim(),
        "# UX checklist",
        "malformed checklist header"
    );
    parts
        .map(|section| {
            let mut lines = section.lines();
            let (id, title) = lines
                .next()
                .expect("section heading")
                .split_once(' ')
                .expect("section title");
            assert!(
                id.len() == 4
                    && id.starts_with("UX")
                    && id[2..].bytes().all(|b| b.is_ascii_digit())
                    && !title.trim().is_empty(),
                "malformed heading"
            );
            let mut field = |prefix: &str| {
                let value = lines
                    .next()
                    .unwrap_or_else(|| panic!("missing {prefix} in {id}"))
                    .strip_prefix(prefix)
                    .unwrap_or_else(|| panic!("malformed {prefix} in {id}"))
                    .trim();
                assert!(!value.is_empty(), "empty {prefix} in {id}");
                value.to_owned()
            };
            let row = ChecklistRow {
                id: id.to_owned(),
                title: title.to_owned(),
                scenario: field("Scenario:"),
                element: field("Element:"),
                after_en: field("After en:"),
                after_zh_tw: field("After zh-TW:"),
                before_lacks: field("Before lacks:"),
            };
            assert!(
                lines.all(|line| line.trim().is_empty()),
                "extra lines in {id}"
            );
            row
        })
        .collect()
}
