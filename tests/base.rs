mod common;
use axiom::base::{self, proc, schema, Family};
use serde_json::json;
use std::{path::Path, time::Duration};
#[test]
fn primitives() {
    for s in ["ml_rram", "a"] {
        assert!(base::require_slug("name", s).is_ok());
    }
    for s in ["ML_RRAM", "_x", "../x", "a/b", ""] {
        assert!(base::require_slug("name", s).is_err());
    }
    assert_eq!(
        base::slug_for_dir(Path::new("Chronos Rust")),
        "chronos-rust"
    );
    assert_eq!(
        base::slug_for_dir(Path::new("TaiwanStockRadar")),
        "taiwanstockradar"
    );
    let d = common::temp();
    let p = d.path().join("file");
    base::atomic_write(&p, "中文".as_bytes()).unwrap();
    base::atomic_write(&p, b"second").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second");
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    assert!(
        regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$")
            .unwrap()
            .is_match(&base::now_iso())
    );
    assert_ne!(base::unique(), base::unique());
    for (m, f) in [
        ("gpt-6.1-terra", Family::OpenAi),
        ("gemini-4-argon-high", Family::Google),
        ("chronos:reviewer", Family::Anthropic),
        ("sonnet", Family::Anthropic),
    ] {
        assert_eq!(base::family_of(m), f);
    }
}
#[test]
fn resolution() {
    let key = "AXIOM_BASE_TEST_COMMAND";
    std::env::set_var(key, r#"["python","-I"]"#);
    assert_eq!(
        base::resolve_command(key, "none").unwrap(),
        vec!["python", "-I"]
    );
    std::env::set_var(key, "no");
    assert_eq!(
        base::resolve_command(key, "none").unwrap_err().kind,
        "config"
    );
    std::env::remove_var(key);
    assert_eq!(
        base::resolve_command(key, "axiom_nonexistent_test_executable")
            .unwrap_err()
            .kind,
        "not_installed"
    );
}
#[cfg(windows)]
#[test]
fn resolution_prefers_pathext() {
    const MARKER: &str = "AXIOM_BASE_RESOLUTION_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let resolved = base::resolve_command("AXIOM_BASE_UNUSED_COMMAND", "codex").unwrap();
        assert!(resolved[0].to_ascii_lowercase().ends_with("codex.cmd"));
        assert!(base::resolve_command("AXIOM_BASE_UNUSED_COMMAND", "codex.cmd").is_ok());
        return;
    }
    let dir = common::temp();
    std::fs::write(dir.path().join("codex"), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.path().join("codex.cmd"), "@exit /b 0\r\n").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resolution_prefers_pathext"])
        .env(MARKER, "1")
        .env("PATH", dir.path())
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env_remove("AXIOM_BASE_UNUSED_COMMAND")
        .status()
        .unwrap();
    assert!(status.success());
}
#[test]
fn shell_and_schema() {
    let d = common::temp();
    let run = proc::run_shell("echo hello", d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 0);
    assert!(run.output.contains("hello"));
    let cmd = format!(
        "\"{}\" -c \"import sys; sys.stdout.buffer.write(bytes([228,184,173,230,150,135]))\"",
        common::python()
    );
    let run = proc::run_shell(&cmd, d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 0, "{}", run.output);
    assert_eq!(run.output, "中文");
    let run = proc::run_shell("exit 3", d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 3);
    assert!(!run.timed_out);
    let cmd = if cfg!(windows) {
        "ping -n 5 127.0.0.1 >nul"
    } else {
        "sleep 5"
    };
    let start = std::time::Instant::now();
    let run = proc::run_shell(cmd, d.path(), Duration::from_millis(100)).unwrap();
    assert!(run.timed_out);
    assert_eq!(run.exit_code, -1);
    assert!(start.elapsed() < Duration::from_secs(5));
    let s = json!({"type":"object","required":["changes"],"additionalProperties":false,"properties":{"changes":{"type":"array","items":{"type":"object","required":["area"],"additionalProperties":false,"properties":{"area":{"enum":["code","tests"]}}}}}});
    assert!(schema::validate(&s, &json!({"changes":[{"area":"code"}]})).is_ok());
    for v in [
        json!({"changes":[{"area":"bad"}]}),
        json!({}),
        json!({"changes":[],"extra":1}),
    ] {
        assert!(schema::validate(&s, &v).is_err());
    }
}
#[test]
fn process_tree() {
    if let Ok(mode) = std::env::var("AXIOM_TEST_CANCEL_TREE") {
        if mode == "sleep" {
            std::thread::sleep(Duration::from_secs(30));
        } else {
            let (hold, path) = mode
                .strip_prefix("hold|")
                .map_or((false, mode.as_str()), |p| (true, p));
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_tree", "--nocapture"])
                .env("AXIOM_TEST_CANCEL_TREE", "sleep")
                .spawn()
                .unwrap();
            std::fs::write(
                path,
                format!("{} {} {}", std::process::id(), child.id(), parent_pid()),
            )
            .unwrap();
            if hold {
                std::thread::sleep(Duration::from_secs(30));
            }
        }
        return;
    }
    let dir = common::temp();
    let pidfile = dir.path().join("pid");
    let script="import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)";
    let args = vec![
        common::python(),
        "-I".into(),
        "-c".into(),
        script.into(),
        pidfile.to_string_lossy().into_owned(),
    ];
    let mut child = proc::command(&args).spawn().unwrap();
    let start = std::time::Instant::now();
    let grandchild: u32 = loop {
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|s| s.parse().ok())
        {
            break pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "pid file did not contain a PID"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let parent = child.id();
    assert!(
        descendant_running(grandchild),
        "grandchild must be running before cleanup"
    );
    proc::kill_tree(&mut child);
    let start = std::time::Instant::now();
    while descendant_running(grandchild) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!proc::pid_alive(parent));
    assert!(!descendant_running(grandchild));
}

#[test]
fn cancelled_scope_blocks_late_spawn_without_affecting_standalone_or_nested_scope() {
    let dir = common::temp();
    let cancelled = proc::Cancellation::default();
    let independent = proc::Cancellation::default();
    cancelled.cancel();
    cancelled.scope(|| {
        let r = proc::run_shell(
            "echo late > must-not-exist",
            dir.path(),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_ne!(r.exit_code, 0);
        assert!(!dir.path().join("must-not-exist").exists());
        independent.scope(|| {
            assert_eq!(
                proc::run_shell("echo independent", dir.path(), Duration::from_secs(2))
                    .unwrap()
                    .exit_code,
                0
            )
        });
        assert_ne!(
            proc::run_shell("echo late", dir.path(), Duration::from_secs(2))
                .unwrap()
                .exit_code,
            0
        );
    });
    assert_eq!(
        proc::run_shell("echo standalone", dir.path(), Duration::from_secs(2))
            .unwrap()
            .exit_code,
        0
    );
}

#[test]
fn cancellation_kills_descendants_even_after_verifier_shell_exits() {
    let dir = common::temp();
    let pids = dir.path().join("pids");
    let executable = std::env::current_exe().unwrap();
    #[cfg(windows)]
    let command = format!(
        "set \"AXIOM_TEST_CANCEL_TREE={}\" && \"{}\" --exact process_tree --nocapture & rem keep shell",
        pids.display(),
        executable.display()
    );
    #[cfg(unix)]
    let command = format!(
        "AXIOM_TEST_CANCEL_TREE={} {} --exact process_tree --nocapture; :",
        shell_quote(&pids),
        shell_quote(&executable)
    );
    let cancellation = proc::Cancellation::default();
    let worker = cancellation.clone();
    let cwd = dir.path().to_owned();
    let job = std::thread::spawn(move || {
        worker.scope(|| proc::run_shell(&command, &cwd, Duration::from_secs(60)).unwrap())
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let ids = loop {
        if let Some(ids) = std::fs::read_to_string(&pids)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids| ids.len() == 3)
        {
            break ids;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    // The trailing shell command prevents exec replacement, so the verifier's
    // recorded parent (ids[2]) is the managed root shell on both platforms.
    while (descendant_running(ids[0]) || descendant_running(ids[2]))
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !descendant_running(ids[0]),
        "intermediate verifier has not exited"
    );
    assert!(
        root_exited_with_live_descendant(ids[2], ids[1]),
        "managed root must have exited while descendant remains running"
    );
    let cancelled_at = std::time::Instant::now();
    cancellation.cancel();
    let result = job.join().unwrap();
    assert!(result.evidence.launched && result.evidence.cancelled);
    assert!(result.evidence.root_reaped, "managed root must be observed reaped");
    assert_eq!(result.evidence.root_exit_code, Some(0), "late cancellation must retain the root's successful exit");
    #[cfg(windows)]
    assert!(result.evidence.group_empty, "job emptiness must be observed after cancellation");
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(5),
        "cancellation waited for natural descendant exit"
    );
    assert_ne!(
        result.exit_code, 0,
        "cancelled pipe draining must not report success"
    );
    assert!(!result.timed_out, "cancellation was reported as timeout");
    assert!(
        result.output.contains("verification cancelled"),
        "missing cancellation marker: {}",
        result.output
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while descendant_running(ids[1]) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !descendant_running(ids[1]),
        "descendant survived cancellation after parent exit"
    );
}

#[test]
fn research_schemas() {
    let audit: serde_json::Value = serde_json::from_str(r#"{"type": "object", "additionalProperties": false, "required": ["summary", "coverage", "changes", "out_of_scope"], "properties": {"summary": {"type": "string"}, "coverage": {"type": "object", "additionalProperties": false, "required": ["read", "not_read"], "properties": {"read": {"type": "array", "items": {"type": "string"}}, "not_read": {"type": "array", "items": {"type": "string"}}}}, "changes": {"type": "array", "items": {"type": "object", "additionalProperties": false, "required": ["id", "area", "location", "evidence", "problem", "change", "verify"], "properties": {"id": {"type": "string"}, "area": {"type": "string", "enum": ["design", "feature", "algorithm", "correctness", "performance", "tests"]}, "location": {"type": "string"}, "evidence": {"type": "string"}, "problem": {"type": "string"}, "change": {"type": "string"}, "verify": {"type": "string"}}}}, "out_of_scope": {"type": "array", "items": {"type": "string"}}}}"#).unwrap();
    let review: serde_json::Value = serde_json::from_str(r#"{"type": "object", "required": ["verdict", "findings"], "properties": {"verdict": {"type": "string", "enum": ["pass", "revise", "reject"]}, "findings": {"type": "array", "items": {"type": "object", "required": ["check", "item", "reason"], "properties": {"check": {"type": "string", "enum": ["out_of_scope", "overclaim", "unfalsifiable", "complexity"]}, "item": {"type": "string"}, "reason": {"type": "string"}}}}}}"#).unwrap();
    let valid = json!({"summary":"ok","coverage":{"read":[],"not_read":[]},"changes":[{"id":"1","area":"tests","location":"x","evidence":"x","problem":"x","change":"x","verify":"x"}],"out_of_scope":[]});
    assert!(schema::validate(&audit, &valid).is_ok());
    let mut wrong = valid.clone();
    wrong["changes"][0]["area"] = json!("wrong");
    assert!(schema::validate(&audit, &wrong)
        .unwrap_err()
        .contains("$.changes[0].area"));
    assert!(schema::validate(&audit, &json!({})).is_err());
    let mut extra = valid;
    extra["extra"] = json!(1);
    assert!(schema::validate(&audit, &extra).is_err());
    assert!(schema::validate(&review, &json!({"verdict":"pass","findings":[]})).is_ok());
    assert!(schema::validate(&review, &json!({"verdict":"bad","findings":[]})).is_err());
}

#[cfg(unix)]
#[test]
fn shell_deadline_includes_inherited_pipes() {
    let started = std::time::Instant::now();
    let result = axiom::base::proc::run_shell(
        "sleep 20 &",
        std::path::Path::new("."),
        std::time::Duration::from_millis(200),
    )
    .unwrap();
    assert!(result.timed_out);
    assert_ne!(result.exit_code, 0);
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

// kill(pid, 0) counts zombies as live; Linux procfs exposes their actual state.
fn descendant_running(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(e) => panic!("cannot inspect process {pid}: {e}"),
        };
        let state = stat
            .rsplit_once(')')
            .expect("malformed proc stat")
            .1
            .trim_start()
            .chars()
            .next()
            .expect("missing process state");
        state != 'Z' && state != 'X'
    }
    #[cfg(windows)]
    {
        proc::pid_alive(pid)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        panic!("process state inspection unsupported on this platform: {pid}")
    }
}

fn root_exited_with_live_descendant(root: u32, descendant: u32) -> bool {
    !descendant_running(root) && descendant_running(descendant)
}

#[cfg(unix)]
fn parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

#[cfg(windows)]
fn parent_pid() -> u32 {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        assert_ne!(snapshot, INVALID_HANDLE_VALUE);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry);
        let mut parent = None;
        while more != 0 {
            if entry.th32ProcessID == std::process::id() {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        parent.expect("current process missing from snapshot")
    }
}

#[cfg(target_os = "linux")]
#[test]
fn process_state_distinguishes_zombie_from_live_process() {
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0);
        if pid == 0 {
            libc::_exit(0);
        }
        let mut info: libc::siginfo_t = std::mem::zeroed();
        assert_eq!(
            libc::waitid(
                libc::P_PID,
                pid as _,
                &mut info,
                libc::WEXITED | libc::WNOWAIT
            ),
            0
        );
        assert!(
            proc::pid_alive(pid as u32),
            "negative control must be an unreaped zombie"
        );
        let running = descendant_running(pid as u32);
        assert_eq!(libc::waitpid(pid, std::ptr::null_mut(), 0), pid);
        assert!(!running, "zombie must not be reported running");
    }
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(unix)]
#[test]
fn shell_quote_handles_single_quotes() {
    let dir = common::temp();
    let path = dir.path().join("file'with quote");
    std::fs::write(&path, "").unwrap();
    for (candidate, expected) in [(&path, 0), (&dir.path().join("missing'file"), 1)] {
        let command = format!("test -f {}", shell_quote(candidate));
        let result = proc::run_shell(&command, dir.path(), Duration::from_secs(3)).unwrap();
        assert_eq!(result.exit_code, expected, "{}", result.output);
        assert!(!result.timed_out);
    }
}

fn assert_pipe_holder_timeout(direct: bool) {
    let dir = common::temp();
    let pids = dir.path().join("pids");
    let executable = std::env::current_exe().unwrap();
    let timeout = Duration::from_secs(2);
    let start = std::time::Instant::now();
    let result = if direct {
        let mut command = std::process::Command::new(&executable);
        command
            .args(["--exact", "process_tree", "--nocapture"])
            .env("AXIOM_TEST_CANCEL_TREE", &pids);
        proc::run_command(command, dir.path(), timeout).unwrap()
    } else {
        #[cfg(windows)]
        let command = format!(
            "set \"AXIOM_TEST_CANCEL_TREE={}\" && \"{}\" --exact process_tree --nocapture",
            pids.display(),
            executable.display()
        );
        #[cfg(unix)]
        let command = format!(
            "AXIOM_TEST_CANCEL_TREE={} {} --exact process_tree --nocapture",
            shell_quote(&pids),
            shell_quote(&executable)
        );
        proc::run_shell(&command, dir.path(), timeout).unwrap()
    };
    assert!(
        result.timed_out,
        "expected pipe-draining timeout: {}",
        result.output
    );
    assert!(result.evidence.launched && result.evidence.root_reaped);
    assert!(!result.evidence.cancelled);
    #[cfg(windows)]
    assert!(result.evidence.group_empty, "job emptiness must be observed after timeout");
    assert_ne!(result.exit_code, 0, "timeout reported success");
    assert!(
        result.output.contains("test result: ok."),
        "parent did not complete successfully before timeout: {}",
        result.output
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    let ids: Vec<u32> = std::fs::read_to_string(pids)
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while descendant_running(ids[1]) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !descendant_running(ids[1]),
        "pipe-holding descendant survived timeout"
    );
}

#[test]
fn timeout_after_successful_shell_parent_kills_pipe_holder() {
    assert_pipe_holder_timeout(false);
}

#[test]
fn timeout_isolates_direct_command_and_kills_pipe_holder() {
    assert_pipe_holder_timeout(true);
}

#[test]
fn cancellation_terminates_live_managed_shell_and_descendant() {
    let dir = common::temp();
    let pids = dir.path().join("pids");
    let executable = std::env::current_exe().unwrap();
    #[cfg(windows)]
    let command = format!("set \"AXIOM_TEST_CANCEL_TREE=hold|{}\" && \"{}\" --exact process_tree --nocapture & rem keep shell", pids.display(), executable.display());
    #[cfg(unix)]
    let command = format!(
        "AXIOM_TEST_CANCEL_TREE={} {} --exact process_tree --nocapture; :",
        shell_quote(Path::new(&format!("hold|{}", pids.display()))),
        shell_quote(&executable)
    );
    let cancellation = proc::Cancellation::default();
    let worker = cancellation.clone();
    let cwd = dir.path().to_owned();
    let job = std::thread::spawn(move || {
        worker.scope(|| proc::run_shell(&command, &cwd, Duration::from_secs(60)).unwrap())
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let ids: Vec<u32> = loop {
        if let Some(ids) = std::fs::read_to_string(&pids)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids: &Vec<u32>| ids.len() == 3)
        {
            break ids;
        }
        assert!(std::time::Instant::now() < deadline, "pid file not ready");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        descendant_running(ids[2]),
        "managed root shell must be live before cancellation"
    );
    assert!(
        descendant_running(ids[1]),
        "descendant must be live before cancellation"
    );
    assert!(
        !root_exited_with_live_descendant(ids[2], ids[1]),
        "live root must not satisfy the exited-root precondition"
    );
    let started = std::time::Instant::now();
    cancellation.cancel();
    let result = job.join().unwrap();
    assert!(result.evidence.launched && result.evidence.root_reaped && result.evidence.cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_ne!(result.exit_code, 0);
    assert!(!result.timed_out);
    assert!(result.output.contains("verification cancelled"));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while (descendant_running(ids[2]) || descendant_running(ids[1]))
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !descendant_running(ids[2]),
        "managed root shell survived cancellation"
    );
    assert!(
        !descendant_running(ids[1]),
        "descendant survived cancellation"
    );
}


#[test]
fn cargo_targets_are_short_writable_bound_and_isolated() {
    use std::{collections::HashSet, fs, process::Command};
    let temp = common::temp();
    let a = temp.path().join("a".repeat(120));
    let b = temp.path().join("b".repeat(120));
    for (dir, output) in [(&a, "source-a"), (&b, "source-b")] {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname='short-target-fixture'\nversion='0.1.0'\nedition='2021'\n").unwrap();
        fs::write(dir.join("src/main.rs"), format!("fn main() {{ println!({output:?}); }}\n")).unwrap();
        // Exercise build_script_build.exe linking as well as the final executable.
        fs::write(dir.join("build.rs"), "fn main() {}\n").unwrap();
        assert!(Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(dir).status().unwrap().success());
    }
    let identity = "execution".repeat(100);
    let paths: Vec<_> = std::thread::scope(|s| {
        (0..8).map(|i| {
            let source = if i % 2 == 0 { &a } else { &b };
            let identity = &identity;
            s.spawn(move || base::cargo_target(source, identity).unwrap())
        }).collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(paths.iter().collect::<HashSet<_>>().len(), paths.len());
    for (i, path) in paths.iter().enumerate() {
        assert!(path.to_string_lossy().encode_utf16().count() <= 78);
        let binding: serde_json::Value = serde_json::from_slice(&fs::read(path.join(".chronos-binding.json")).unwrap()).unwrap();
        assert_eq!(binding["execution"], identity);
        assert_eq!(binding["source"], fs::canonicalize(if i % 2 == 0 { &a } else { &b }).unwrap().to_string_lossy().as_ref());
    }
    std::thread::scope(|s| {
        let builds: Vec<_> = [(&a, &paths[0], "source-a"), (&b, &paths[1], "source-b")]
            .into_iter().map(|(source, target, expected)| s.spawn(move || {
                let result = Command::new("cargo").args(["build", "--locked", "--offline"]).current_dir(source).env("CARGO_TARGET_DIR", target).output().unwrap();
                assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
                let binary = target.join("debug").join(format!("short-target-fixture{}", std::env::consts::EXE_SUFFIX));
                let run = Command::new(binary).output().unwrap();
                assert!(run.status.success());
                assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), expected);
            })).collect();
        for build in builds { build.join().unwrap(); }
    });
    // Unused concurrent invocations must not inherit either source's artifacts.
    assert!(!paths[2].join("debug").exists());
    assert!(!paths[3].join("debug").exists());
    let invalid = paths[0].parent().unwrap().join(format!("cf-{}", &base::unique()[..8]));
    fs::write(&invalid, "x").unwrap();
    assert!(invalid.to_string_lossy().encode_utf16().count() <= 50);
    let failure = base::cargo_target_in(&a, "x", &[invalid.clone()]).unwrap_err();
    assert_eq!(failure.kind, "cargo_target");
    assert!(failure.message.contains("no writable short Cargo target root"));
    let fallback = base::cargo_target_in(&a, "fallback", &[invalid.clone(), paths[0].parent().unwrap().to_path_buf()]).unwrap();
    fs::remove_dir_all(fallback).unwrap();
    fs::remove_file(invalid).unwrap();
    for path in paths { fs::remove_dir_all(path).unwrap(); }
}
#[test]
fn typed_domain_result_never_promotes_transport_completion() {
    use axiom::base::tool_succeeded;
    use serde_json::json;
    for outcome in ["blocked", "failed", "timeout", "cancelled", "unconfirmed", "protocol_incomplete", "invented"] {
        assert!(!tool_succeeded(&json!({"status":"completed","outcome":outcome})), "{outcome}");
    }
    for outcome in ["awaiting_core", "verified"] {
        assert!(tool_succeeded(&json!({"status":"completed","outcome":outcome})));
        assert!(!tool_succeeded(&json!({"status":"timeout","outcome":outcome})));
    }
}

#[test]
fn runner_terminal_evidence_is_independent_of_result() {
    let dir = common::temp();
    for (command, code) in [("exit 0", 0), ("exit 3", 3)] {
        let run = proc::run_shell(command, dir.path(), Duration::from_secs(3)).unwrap();
        assert_eq!(run.exit_code, code);
        assert!(run.evidence.launched && run.evidence.root_reaped);
        assert_eq!(run.evidence.root_exit_code, Some(code));
        assert!(run.evidence.stdout_eof && run.evidence.stderr_eof && run.evidence.group_empty);
        assert!(!run.evidence.cancelled && !run.timed_out);
    }
    let cancelled = proc::Cancellation::default();
    cancelled.cancel();
    let run = cancelled.scope(|| proc::run_shell("exit 0", dir.path(), Duration::from_secs(3)).unwrap());
    assert!(run.evidence.cancelled);
    assert!(!run.evidence.launched && !run.evidence.root_reaped && !run.evidence.group_empty);

    // A runner error is not a successful ShellRun or positive stopping evidence.
    let error = proc::run_command(
        std::process::Command::new(dir.path().join("missing-executable")),
        dir.path(),
        Duration::from_secs(3),
    ).err().expect("missing executable must fail to launch");
    assert_eq!(error.kind, "process");
}

#[cfg(windows)]
#[test]
fn closed_pipes_do_not_prove_windows_job_empty() {
    let dir = common::temp();
    let pids = dir.path().join("pids");
    // close_fds=True matters on Windows: redirecting standard streams alone
    // can leave unrelated inherited pipe handles open in the descendant.
    let mut command = std::process::Command::new(common::python());
    command
        .args(["-I", "-c", "import pathlib, subprocess, sys; child = subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(30)'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True); pathlib.Path(sys.argv[1]).write_text(str(child.pid))"])
        .arg(&pids);
    let run = proc::run_command(command, dir.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 0, "{:?}: {}", run.evidence, run.output);
    assert!(run.evidence.root_reaped && run.evidence.stdout_eof && run.evidence.stderr_eof);
    assert!(!run.evidence.group_empty, "a live descendant with closed pipes must not certify the job empty");
    // Dropping the job requests cleanup only after the observation. Do not
    // promote that unobserved request into evidence in the returned result.
    let pid: u32 = std::fs::read_to_string(pids).unwrap().trim().parse().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while descendant_running(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!descendant_running(pid), "test descendant survived job cleanup");
    assert!(!run.evidence.group_empty, "later cleanup must not rewrite recorded evidence");
}

#[test]
fn runner_retains_root_exit_when_pipe_holder_exceeds_budget() {
    let dir = common::temp();
    // A pipe-holding descendant makes timeout deterministic after the root's
    // successful exit, without relying on hitting a scheduler's 10 ms window.
    let mut command=std::process::Command::new(common::python());
    command.args(["-I","-c","import subprocess,sys; subprocess.Popen([sys.executable,'-I','-c','import time; time.sleep(30)'])"]);
    let run=proc::run_command(command,dir.path(),Duration::from_secs(2)).unwrap();
    assert!(run.timed_out);
    assert_eq!(run.evidence.root_exit_code,Some(0));
    assert!(run.evidence.root_reaped && run.evidence.stdout_eof && run.evidence.stderr_eof);
    assert_eq!(run.exit_code, -1);
    assert!(run.evidence.group_empty);
}

#[cfg(unix)]
#[test]
fn closed_pipes_do_not_prove_process_group_empty() {
    let dir = common::temp();
    let pidfile = dir.path().join("background-pid");
    let command = format!("sleep 20 </dev/null >/dev/null 2>&1 & echo $! > {}", shell_quote(&pidfile));
    let run = proc::run_shell(&command, dir.path(), Duration::from_secs(3)).unwrap();
    let pid: i32 = std::fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
    unsafe { libc::kill(pid, libc::SIGKILL); }
    assert_eq!(run.exit_code, 0);
    assert!(run.evidence.root_reaped && run.evidence.stdout_eof && run.evidence.stderr_eof);
    assert!(!run.evidence.group_empty, "live group must not be certified empty");
}

#[test]
fn absolute_deadline_scope_cannot_be_renewed_and_prevents_command_launch() {
    use axiom::base::proc::{deadline_scope, remaining, unix_ms};
    use std::time::Duration;
    let end = unix_ms().saturating_sub(1);
    deadline_scope(end, || {
        assert!(remaining(Duration::from_secs(99)).is_zero());
        deadline_scope(unix_ms() + 99999, || {
            assert!(remaining(Duration::from_secs(99)).is_zero());
            let error = axiom::base::proc::run_command(std::process::Command::new("must-not-launch"), std::path::Path::new("."), Duration::from_secs(99)).err().expect("expired operation cannot launch");
            assert_eq!(error.kind, "timeout");
        });
    });
    assert!(!remaining(Duration::from_secs(99)).is_zero());
}
