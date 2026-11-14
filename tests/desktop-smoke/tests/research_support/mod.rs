#![allow(dead_code)]
use axiom::desktop::{
    demo,
    i18n::{self, Lang},
    model::AppState,
    research::ResearchSeams,
    theme,
    ux::Page,
    Shell, Store,
};
use gpui_kit::{
    component::ThemeMode, px, size, test::TestWindowExt as _, AnyWindowHandle, AppContext as _,
    Bounds, ElementId, Entity, Point, SharedString, TestAppContext, WindowBounds, WindowOptions,
};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub fn temp_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let p = std::env::temp_dir().join(format!(
            "axiom-research-smoke-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&p) {
            Ok(()) => return p,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("create temp dir: {e}"),
        }
    }
}
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to);
        } else {
            fs::copy(e.path(), to).unwrap();
        }
    }
}
pub fn fixture_copy(name: &str) -> PathBuf {
    let dst = temp_dir().join(name);
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/research")
            .join(name),
        &dst,
    );
    dst
}
pub fn tree_hash(dir: &Path) -> String {
    fn collect(root: &Path, dir: &Path, paths: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            paths.push(p.strip_prefix(root).unwrap().to_owned());
            if e.file_type().unwrap().is_dir() {
                collect(root, &p, paths);
            }
        }
    }
    let mut paths = Vec::new();
    collect(dir, dir, &mut paths);
    paths.sort();
    let mut h = DefaultHasher::new();
    for p in paths {
        p.hash(&mut h);
        let full = dir.join(p);
        full.is_dir().hash(&mut h);
        if full.is_file() {
            fs::read(full).unwrap().hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}
pub fn open_shell(
    cx: &mut TestAppContext,
    lang: Lang,
    mode: ThemeMode,
    size_: (f32, f32),
    seams: ResearchSeams,
    research: bool,
) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(mode, None, cx);
        i18n::set(lang, cx);
        cx.set_global(Store::open(true));
    });
    let mut state = demo::state();
    state.ux.research = seams;
    if research {
        state.ux.page = Some(Page::Research);
    }
    let state = cx.new(|_| state);
    let shell_state = state.clone();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(size_.0), px(size_.1)),
        })),
        ..Default::default()
    };
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Shell::new(shell_state, false, window, cx))
            })
        })
        .expect("open Shell");
    frame(cx, window);
    (window, state)
}
pub fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .expect("render frame");
    cx.run_until_parked();
}
pub fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) {
    cx.update_window(window, |_, window, cx| {
        window.click(SharedString::from(id.to_owned()), cx)
    })
    .expect("click");
    frame(cx, window);
}
pub fn type_text(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    frame(cx, window);
}
pub fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx))
        .unwrap();
    frame(cx, window);
}
pub fn start_open_unparked(cx: &mut TestAppContext, window: AnyWindowHandle, root: &Path) {
    click(cx, window, "research-open-path");
    type_text(cx, window, root.to_str().unwrap());
    let store = root.parent().unwrap().join("store");
    if store.is_dir() {
        click(cx, window, "research-open-store");
        type_text(cx, window, store.to_str().unwrap());
    }
    cx.update_window(window, |_, window, cx| {
        window.click("research-open-button", cx)
    })
    .expect("open research");
}
pub fn start_open(cx: &mut TestAppContext, window: AnyWindowHandle, root: &Path) {
    start_open_unparked(cx, window, root);
    frame(cx, window);
}
pub fn wait_load(cx: &mut TestAppContext, window: AnyWindowHandle) -> usize {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut frames = 0;
    loop {
        let lines = capture(cx, window);
        if shows(&lines, "research-load", "Ready")
            || shows(&lines, "research-load", "Failed")
            || shows(&lines, "research-load", "已就緒")
            || shows(&lines, "research-load", "失敗")
        {
            return frames;
        }
        frames += 1;
        assert!(
            Instant::now() < deadline,
            "research load timed out: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
pub fn open_research(
    cx: &mut TestAppContext,
    lang: Lang,
    mode: ThemeMode,
    size: (f32, f32),
    root: &Path,
    seams: ResearchSeams,
) -> (AnyWindowHandle, Entity<AppState>) {
    let result = open_shell(cx, lang, mode, size, seams, true);
    start_open(cx, result.0, root);
    wait_load(cx, result.0);
    result
}
pub fn capture(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<(String, String)> {
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        let mut lines: Vec<_> = gpui_kit::base::test_support::texts(window)
            .into_iter()
            .filter(|t| t.visible())
            .map(|t| {
                (
                    t.path()
                        .iter()
                        .filter_map(|id| match id {
                            ElementId::Name(n) | ElementId::NamedInteger(n, _) => {
                                Some(n.to_string())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("/"),
                    t.text().replace(['\t', '\r', '\n'], " "),
                )
            })
            .collect();
        // Button labels are exposed as native accessibility labels, not observed text.
        if let Some(button) = window.try_find("research-primary") {
            if button.visible() {
                if let Some(label) = button.label() {
                    let path = button
                        .path()
                        .iter()
                        .filter_map(|id| match id {
                            ElementId::Name(n) | ElementId::NamedInteger(n, _) => {
                                Some(n.to_string())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("/");
                    lines.push((path, label.into()));
                }
            }
        }
        lines
    })
    .unwrap()
}
pub fn shows(lines: &[(String, String)], id: &str, text: &str) -> bool {
    lines
        .iter()
        .any(|(p, t)| p.split('/').any(|part| part == id) && t.contains(text))
}
pub fn visible(lines: &[(String, String)], id: &str) -> bool {
    lines
        .iter()
        .any(|(p, _)| p.split('/').any(|part| part == id))
}

pub fn wait_retry(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.executor().advance_clock(Duration::from_millis(60));
        frame(cx, window);
        if cx
            .update_window(window, |_, w, _| w.try_find(id.to_owned()).is_some())
            .unwrap()
        {
            return;
        }
        assert!(Instant::now() < deadline, "waiting for {id}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
