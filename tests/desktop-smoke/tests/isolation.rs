#![cfg(windows)]
mod isolation_support;
mod release_support;

#[cfg(test)]
mod placement {
    use super::isolation_support::*;
    fn layout() -> Vec<Monitor> {
        vec![
            Monitor {
                rect: (0, 0, 2560, 1440),
                primary: true,
                hz: 170,
                dpi: 96,
                device: "primary".into(),
            },
            Monitor {
                rect: (-1920, 180, 0, 1260),
                primary: false,
                hz: 60,
                dpi: 96,
                device: "left".into(),
            },
        ]
    }
    #[test]
    fn chooses_placement() {
        let m = layout();
        assert_eq!(place(&m), Place::Secondary);
        assert_eq!(place_args(place(&m)), ["--place", "secondary"]);
        assert_eq!(place(&m[..1]), Place::Offscreen);
        assert_eq!(place_args(place(&m[..1])), ["--place", "offscreen"]);
        assert_eq!(place(&[]), Place::Offscreen);
    }
    #[test]
    fn largest_intersection() {
        let m = layout();
        assert_eq!(monitor_of(&m, (-1800, 200, -100, 1000)).unwrap().hz, 60);
        assert_eq!(monitor_of(&m, (-3000, -3000, -2900, -2900)), None);
        assert_eq!(monitor_of(&m, (-100, 200, 500, 1000)).unwrap().hz, 170);
        assert_eq!(monitor_of(&m, (-2000, 180, -1920, 300)), None);
    }
}
#[cfg(test)]
mod sweep {
    use super::isolation_support::*;
    use std::cell::RefCell;
    #[test]
    fn matches_and_drops_other_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pids.jsonl");
        record(&file, 101, 1);
        record(&file, 102, 2);
        record(&file, 103, 3);
        let stopped = RefCell::new(Vec::new());
        let result = sweep(
            &file,
            &|pid| match pid {
                101 => Some(1),
                102 => Some(99),
                103 => None,
                104 => Some(4),
                _ => panic!("unrecorded pid probed"),
            },
            &|pid| stopped.borrow_mut().push(pid),
        );
        assert_eq!(result, vec![101]);
        assert_eq!(*stopped.borrow(), vec![101]);
        assert!(std::fs::read_to_string(&file).unwrap().is_empty());
    }
    #[test]
    fn own_creation_time_is_available() {
        assert!(creation_time(std::process::id()).is_some());
    }
}

mod input {
    use std::{fs, path::Path};
    const FORBIDDEN: &[&str] = &[
        "SetCursorPos",
        "SetPhysicalCursorPos",
        "SendInput",
        "mouse_event",
        "keybd_event",
        "SetForegroundWindow",
        "SetActiveWindow",
        "SetFocus",
        "BringWindowToTop",
        "SwitchToThisWindow",
        "AttachThreadInput",
        "AllowSetForegroundWindow",
        "LockSetForegroundWindow",
        "BlockInput",
        "ClipCursor",
    ];
    fn word_hits(source: &str) -> Vec<&'static str> {
        source
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter_map(|w| FORBIDDEN.iter().copied().find(|name| *name == w))
            .collect()
    }
    fn command_hits(source: &str) -> usize {
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        compact.match_indices("Command::new(").count()
    }
    fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                files(&p, out);
            } else if p.extension().is_some_and(|ext| ext == "rs")
                && p.file_name().unwrap() != "isolation.rs"
            {
                out.push(p);
            }
        }
    }
    #[test]
    fn scanner_negative_controls() {
        for &name in FORBIDDEN {
            assert_eq!(word_hits(&format!("fn f() {{ {name}(); }}")), vec![name]);
            assert!(word_hits(&format!("prefix{name}suffix")).is_empty());
        }
        assert_eq!(command_hits("let c = Command::new(exe);"), 1);
    }
    #[test]
    fn harness_uses_only_isolated_launch_and_posted_input() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut found = Vec::new();
        files(&root, &mut found);
        let mut errors = Vec::new();
        for file in found {
            let source = fs::read_to_string(&file).unwrap();
            let hits = word_hits(&source);
            if !hits.is_empty() {
                errors.push(format!("{}: forbidden calls {hits:?}", file.display()));
            }
            let commands = command_hits(&source);
            let relative = file.strip_prefix(&root).unwrap();
            if commands > 0 && !relative.starts_with("isolation_support") {
                let git_only = relative == Path::new("release_support/mod.rs")
                    && commands == 1
                    && source.contains("Command::new(\"git\")");
                if !git_only {
                    errors.push(format!("{}: {commands} stray Command::new", file.display()));
                }
            }
        }
        for file in ["png_support/native.rs", "perf_support/scenarios.rs"] {
            let source = fs::read_to_string(root.join(file)).unwrap();
            if !source.contains("isolation::launch") {
                errors.push(format!("{file}: missing isolation::launch"));
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }
}

mod live_helpers {
    use super::isolation_support::{self as isolation, Launched, Place};
    use std::{
        fs,
        path::Path,
        process::Stdio,
        thread,
        time::{Duration, Instant, SystemTime},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
    };
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> i32 {
        let pair = &mut *(data as *mut (u32, HWND));
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == pair.0 && IsWindowVisible(hwnd) != 0 {
            pair.1 = hwnd;
            return 0;
        }
        1
    }
    fn window(pid: u32) -> Option<HWND> {
        let mut pair: (u32, HWND) = (pid, std::ptr::null_mut());
        unsafe {
            EnumWindows(Some(find), &mut pair as *mut _ as LPARAM);
        }
        (!pair.1.is_null()).then_some(pair.1)
    }
    pub fn exercise(exe: &Path, home: &Path, demo: bool) {
        let displays = isolation::monitors();
        if isolation::place(&displays) == Place::Secondary {
            assert!(displays.iter().any(|m| !m.primary));
        }
        let start = SystemTime::now();
        let log = home.join("live-perf.jsonl");
        let log_path = log.to_str().expect("UTF-8 temporary log path");
        let mut args = vec!["--target", "run", "--state", "live", "--perf-log", log_path];
        if demo {
            args.insert(0, "--demo");
        }
        let mut app = isolation::launch(exe, home, &args, Stdio::null());
        let appearance_deadline = Instant::now() + Duration::from_secs(30);
        let mut first = None;
        let mut clicked = false;
        let mut interactive = false;
        let mut last_click = Instant::now();
        loop {
            assert!(
                app.child.try_wait().unwrap().is_none(),
                "app exited before live checks finished"
            );
            if let Some(hwnd) = window(app.child.id()) {
                let appeared = *first.get_or_insert_with(Instant::now);
                isolation::shown(&app, hwnd).expect("window placement");
                isolation::not_foreground(&app).expect("foreground isolation");
                if !clicked || last_click.elapsed() >= Duration::from_millis(200) {
                    isolation::input::post_click(hwnd, (100, 100)).expect("post click");
                    clicked = true;
                    last_click = Instant::now();
                }
                interactive |= fs::read_to_string(&log)
                    .unwrap_or_default()
                    .lines()
                    .any(|s| {
                        serde_json::from_str::<serde_json::Value>(s)
                            .ok()
                            .is_some_and(|v| v["t"] == "interactive")
                    });
                if appeared.elapsed() >= Duration::from_secs(5) {
                    break;
                }
            } else {
                assert!(first.is_none(), "window disappeared during live checks");
                assert!(
                    Instant::now() < appearance_deadline,
                    "window did not appear"
                );
                isolation::not_foreground(&app).expect("foreground isolation before window");
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            interactive,
            "posted click did not produce interactive record"
        );
        if !demo {
            assert!(
                home.join("service/service.json").exists(),
                "real app did not start a service"
            );
        }
        drop(app);
        assert!(
            isolation::strays(exe, start).is_empty(),
            "chronos processes left after guard drop"
        );
    }
    pub fn seed(home: &Path) {
        let wish = "w-1234567890123-abcdef";
        fs::create_dir_all(home.join("goals")).unwrap();
        fs::create_dir_all(home.join("logs")).unwrap();
        fs::create_dir_all(home.join("orchestrate/proj").join(wish)).unwrap();
        fs::write(home.join("goals/s1.json"), serde_json::json!({"wish":"Performance run","wish_id":wish,"project":"proj","status":"active"}).to_string()).unwrap();
        fs::write(
            home.join("logs/20261109-100000-wish.jsonl"),
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}\n",
        )
        .unwrap();
    }
    pub fn guard(exe: &Path, home: &Path) -> Launched {
        isolation::launch(
            exe,
            home,
            &["--demo", "--target", "run", "--state", "live"],
            Stdio::null(),
        )
    }
}
mod cleanup {
    use super::{isolation_support as isolation, live_helpers, release_support};
    use std::{
        fs,
        panic::{catch_unwind, AssertUnwindSafe},
        thread,
        time::{Duration, Instant, SystemTime},
    };
    fn run(unwind: bool) {
        let exe = release_support::release_exe("chronos").expect("release executable required");
        let home = tempfile::tempdir().unwrap();
        let start = SystemTime::now();
        let mut service = isolation::spawn_serve(&exe, home.path());
        let pid = service.id();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !home.path().join("service/service.json").is_file() {
            assert!(
                service.try_wait().unwrap().is_none(),
                "service exited before ready"
            );
            assert!(
                Instant::now() < deadline,
                "service did not write service.json"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(home.path().join("service/service.json")).unwrap())
                .unwrap();
        assert_eq!(record["pid"].as_u64(), Some(pid as u64));
        if unwind {
            assert!(catch_unwind(AssertUnwindSafe(|| {
                let _guard = live_helpers::guard(&exe, home.path());
                panic!("exercise unwind");
            }))
            .is_err());
        } else {
            let guard = live_helpers::guard(&exe, home.path());
            drop(guard);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while service.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "service still alive after guard drop"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            isolation::strays(&exe, start).is_empty(),
            "chronos process remains after cleanup"
        );
    }
    #[test]
    #[ignore]
    fn normal_drop() {
        run(false);
    }
    #[test]
    #[ignore]
    fn unwind_drop() {
        run(true);
    }
}
mod live {
    use super::{live_helpers, release_support};
    #[test]
    #[ignore]
    fn demo_is_isolated_and_interactive() {
        let exe = release_support::release_exe("chronos").expect("release executable required");
        let home = tempfile::tempdir().unwrap();
        live_helpers::exercise(&exe, home.path(), true);
    }
    #[test]
    #[ignore]
    fn real_seams_and_service_are_isolated() {
        let exe = release_support::release_exe("chronos").expect("release executable required");
        let home = tempfile::tempdir().unwrap();
        live_helpers::seed(home.path());
        live_helpers::exercise(&exe, home.path(), false);
    }
}
