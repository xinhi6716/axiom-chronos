use axiom::orch::{run, with_lock, Output};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "orch-store-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn call(p: &Path, args: &[&str], json: bool, force: bool) -> Output {
    run(
        p,
        &args.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
        json,
        force,
    )
}
fn ok(p: &Path, args: &[&str], json: bool) -> Output {
    let o = call(p, args, json, false);
    assert_eq!(o.exit_code, 0, "{:?}: {}", args, o.stderr);
    o
}
#[test]
fn commands_round_trip() {
    let t = Temp::new();
    let p = &t.0;
    for json in [false, true] {
        let o = ok(p, &["init"], json);
        if json {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&o.stdout).unwrap()["store"],
                p.to_string_lossy().as_ref()
            )
        } else {
            assert!(o.stdout.starts_with("initialized "))
        }
    }
    for json in [false, true] {
        let id = if json { "u2" } else { "u1" };
        let add = ok(p, &["unit", "add", id, "--track", "core"], json);
        if json {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&add.stdout).unwrap()["state"],
                "pending"
            )
        } else {
            assert_eq!(add.stdout, "u1\tcore\tpending\t\t\t\t\n")
        };
        ok(
            p,
            &[
                "unit",
                "set",
                id,
                "--state",
                "review-failed",
                "--branch",
                "b",
                "--pr",
                "3",
                "--sha",
                "abc",
            ],
            json,
        );
        ok(p, &["unit", "get", id], json);
        ok(
            p,
            &[
                "unit",
                "list",
                "--state",
                "review-failed",
                "--track",
                "core",
            ],
            json,
        );
        ok(p, &["unit", "counts"], json);
        let row = ok(
            p,
            &[
                "ledger",
                "record",
                "3",
                id,
                "unit-test-verified",
                "--evidence",
                "proof",
                "--verifier",
                "v",
            ],
            json,
        );
        if !json {
            assert_eq!(row.stdout, format!("3\t{id}\tunit-test-verified\n"))
        }
        let check = ok(p, &["ledger", "check", "3", id], json);
        if !json {
            assert_eq!(check.stdout, "unit-test-verified\n")
        }
        ok(p, &["ledger", "summary"], json);
        ok(
            p,
            &["inbox", "push", "a", id, "done", "--report", "r"],
            json,
        );
        let peek = ok(p, &["inbox", "drain", "--peek"], json);
        assert!(peek.stdout.contains(id));
        ok(p, &["inbox", "count"], json);
        let drain = ok(p, &["inbox", "drain"], json);
        assert!(drain.stdout.contains(id));
        ok(
            p,
            &[
                "gate",
                "park",
                id,
                "--question",
                "Q?",
                "--options",
                "a,b",
                "--default",
                "a",
            ],
            json,
        );
        ok(p, &["gate", "list"], json);
        ok(p, &["gate", "resolve", id, "--answer", "b"], json);
        ok(p, &["standing", "add", id], json);
        ok(p, &["standing", "show"], json);
        ok(p, &["frontier", "show"], json);
        ok(p, &["status"], json);
    }
    let missing = call(p, &["ledger", "check", "4", "none"], false, false);
    assert_eq!(missing.exit_code, 2);
    assert_eq!(missing.stdout, "NOT-VERIFIED\n");
    let missing = call(p, &["ledger", "check", "4", "none"], true, false);
    assert_eq!(missing.exit_code, 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&missing.stdout).unwrap()["verdict"],
        "NOT-VERIFIED"
    );
    assert!(std::fs::read_to_string(p.join("status.md"))
        .unwrap()
        .contains("<!-- orch-summary "));
}
#[test]
fn errors_and_locks() {
    let t = Temp::new();
    let p = &t.0;
    ok(p, &["init"], false);
    ok(p, &["unit", "add", "u", "--track", "core"], false);
    for state in ["built", "verified"] {
        assert_eq!(
            call(p, &["unit", "set", "u", "--state", state], false, false).exit_code,
            1
        )
    }
    assert_eq!(
        call(p, &["frontier", "set"], false, false).stderr,
        "error: frontier set is not supported\n"
    );
    let brief = p.join("brief.md");
    std::fs::write(&brief, "GOAL g\nSCOPE src/a\n").unwrap();
    assert!(call(
        p,
        &[
            "unit",
            "add",
            "x",
            "--track",
            "core",
            "--brief",
            brief.to_str().unwrap()
        ],
        false,
        false
    )
    .stderr
    .contains("VERIFY"));
    std::fs::write(p.join(".orch.lock"), format!("{}\n", std::process::id())).unwrap();
    assert!(call(p, &["standing", "add", "x"], false, false)
        .stderr
        .contains("store lock held"));
    assert!(call(p, &["standing", "add", "x"], false, true)
        .stderr
        .contains("stealing store lock"));
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    std::fs::write(p.join(".orch.lock"), format!("{pid}\n")).unwrap();
    assert!(call(p, &["standing", "add", "dead"], false, false)
        .stderr
        .contains("replacing stale store lock"));
    std::fs::write(p.join(".orch.lock"), "4294967295\n").unwrap();
    assert!(call(p, &["standing", "add", "y"], false, false)
        .stderr
        .contains("replacing stale store lock"));
}

struct LockChild(Child);
impl Drop for LockChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn wait_for_file(path: &Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn lock_child(store: &Path, tag: &str) -> LockChild {
    LockChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "store_cases::lock_child_process", "--nocapture"])
            .env("AXIOM_TEST_LOCK_STORE", store)
            .env("AXIOM_TEST_LOCK_TAG", tag)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}

// Executed by the test harness in a separate OS process. File handshakes make
// entry and exit from the critical section deterministic.
#[test]
fn lock_child_process() {
    let Some(store) = std::env::var_os("AXIOM_TEST_LOCK_STORE") else {
        return;
    };
    let store = PathBuf::from(store);
    let tag = std::env::var("AXIOM_TEST_LOCK_TAG").unwrap();
    std::fs::write(store.join(format!("{tag}-ready")), "ready").unwrap();
    wait_for_file(&store.join("start"));
    let result = with_lock(&store, false, || {
        std::fs::write(
            store.join(format!("{tag}-entered")),
            std::process::id().to_string(),
        )
        .unwrap();
        wait_for_file(&store.join("release"));
        Ok(())
    });
    if let Err(error) = result {
        std::fs::write(store.join(format!("{tag}-error")), error.message).unwrap();
    }
}

#[test]
fn stale_lock_recovery_serializes_independent_processes() {
    let t = Temp::new();
    let p = &t.0;
    std::fs::write(p.join(".orch.lock"), "4294967295\n").unwrap();
    let mut a = lock_child(p, "a");
    let mut b = lock_child(p, "b");
    wait_for_file(&p.join("a-ready"));
    wait_for_file(&p.join("b-ready"));
    std::fs::write(p.join("start"), "go").unwrap();
    let started = Instant::now();
    loop {
        let entered = ["a", "b"]
            .into_iter()
            .filter(|tag| p.join(format!("{tag}-entered")).exists())
            .count();
        let rejected = ["a", "b"]
            .into_iter()
            .filter(|tag| p.join(format!("{tag}-error")).exists())
            .count();
        assert!(entered <= 1, "two processes entered the same store");
        if entered == 1 && rejected == 1 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "no lock result"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let marker = std::fs::read_to_string(p.join(".orch.lock")).unwrap();
    let entered_pid = ["a", "b"]
        .into_iter()
        .find_map(|tag| std::fs::read_to_string(p.join(format!("{tag}-entered"))).ok())
        .unwrap();
    assert_eq!(marker.trim(), entered_pid.trim());
    std::fs::write(p.join("release"), "go").unwrap();
    assert!(a.0.wait().unwrap().success());
    assert!(b.0.wait().unwrap().success());
    assert!(!p.join(".orch.lock").exists());
}

#[test]
fn killed_holder_releases_kernel_lock_and_is_recoverable() {
    let t = Temp::new();
    let p = &t.0;
    let mut holder = lock_child(p, "holder");
    wait_for_file(&p.join("holder-ready"));
    std::fs::write(p.join("start"), "go").unwrap();
    wait_for_file(&p.join("holder-entered"));
    let error = with_lock(p, false, || Ok(())).unwrap_err();
    assert!(error.message.contains("store lock held"));
    holder.0.kill().unwrap();
    holder.0.wait().unwrap();
    with_lock(p, false, || Ok(())).unwrap();
    assert!(!p.join(".orch.lock").exists());
}

#[cfg(unix)]
#[test]
fn fork_inherited_guard_does_not_block_next_lock() {
    struct Forked {
        pid: libc::pid_t,
        release: i32,
    }
    impl Forked {
        fn release(&mut self) {
            let byte = [1u8];
            unsafe {
                assert_eq!(libc::write(self.release, byte.as_ptr().cast(), 1), 1);
                libc::close(self.release);
                self.release = -1;
                let mut status = 0;
                assert_eq!(libc::waitpid(self.pid, &mut status, 0), self.pid);
                assert!(libc::WIFEXITED(status));
                assert_eq!(libc::WEXITSTATUS(status), 0);
            }
            self.pid = 0;
        }
    }
    impl Drop for Forked {
        fn drop(&mut self) {
            unsafe {
                if self.release >= 0 {
                    libc::close(self.release);
                }
                if self.pid > 0 {
                    libc::kill(self.pid, libc::SIGKILL);
                    libc::waitpid(self.pid, std::ptr::null_mut(), 0);
                }
            }
        }
    }

    let t = Temp::new();
    let p = &t.0;
    let mut ready = [0; 2];
    let mut release = [0; 2];
    unsafe {
        assert_eq!(libc::pipe(ready.as_mut_ptr()), 0);
        assert_eq!(libc::pipe(release.as_mut_ptr()), 0);
    }
    let mut child = None;
    with_lock(p, false, || {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // After fork in the multithreaded test harness, use only
            // async-signal-safe libc calls until _exit.
            unsafe {
                libc::close(ready[0]);
                libc::close(release[1]);
                let byte = [1u8];
                let sent = libc::write(ready[1], byte.as_ptr().cast(), 1);
                let mut done = [0u8];
                let received = libc::read(release[0], done.as_mut_ptr().cast(), 1);
                libc::_exit(if sent == 1 && received == 1 { 0 } else { 1 });
            }
        }
        unsafe {
            libc::close(ready[1]);
            libc::close(release[0]);
        }
        child = Some(Forked {
            pid,
            release: release[1],
        });
        let mut poll = libc::pollfd {
            fd: ready[0],
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut poll, 1, 10_000) }, 1);
        let mut byte = [0u8];
        assert_eq!(
            unsafe { libc::read(ready[0], byte.as_mut_ptr().cast(), 1) },
            1
        );
        unsafe { libc::close(ready[0]) };
        Ok(())
    })
    .unwrap();
    assert!(!p.join(".orch.lock").exists());
    // The child remains alive holding the inherited descriptor. Explicit
    // unlock must let a fresh descriptor enter before that child exits.
    let next = with_lock(p, false, || Ok(()));
    child.as_mut().unwrap().release();
    next.unwrap();
}

#[test]
fn panic_clears_pid_marker_and_releases_gate() {
    let t = Temp::new();
    let p = &t.0;
    let result = std::panic::catch_unwind(|| {
        let _: axiom::base::Res<()> = with_lock(p, false, || panic!("injected panic"));
    });
    assert!(result.is_err());
    assert!(!p.join(".orch.lock").exists());
    with_lock(p, false, || Ok(())).unwrap();
}

#[test]
fn legacy_unknown_and_explicit_force_preserve_existing_contract() {
    let t = Temp::new();
    let p = &t.0;
    std::fs::write(p.join(".orch.lock"), b"").unwrap();
    let error = with_lock(p, false, || Ok(())).unwrap_err();
    assert!(error.message.contains("pid unknown"));
    assert_eq!(std::fs::read(p.join(".orch.lock")).unwrap(), b"");
    with_lock(p, true, || Ok(())).unwrap();

    // Preserve the explicit old force override. It can overlap another writer;
    // ordinary calls are guarded and must never steal this active lock.
    let mut holder = lock_child(p, "holder");
    wait_for_file(&p.join("holder-ready"));
    std::fs::write(p.join("start"), "go").unwrap();
    wait_for_file(&p.join("holder-entered"));
    assert!(with_lock(p, false, || Ok(())).is_err());
    with_lock(p, true, || Ok(())).unwrap();
    std::fs::write(p.join("release"), "go").unwrap();
    assert!(holder.0.wait().unwrap().success());
}
#[test]
fn old_stores_read() {
    for fixture in ["old", "old-crlf"] {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/orch/stores")
            .join(fixture);
        for args in [
            &["unit", "list"][..],
            &["ledger", "summary"],
            &["gate", "list"],
            &["standing", "show"],
            &["inbox", "drain", "--peek"],
            &["frontier", "show"],
        ] {
            let value: serde_json::Value =
                serde_json::from_str(&ok(&p, args, true).stdout).unwrap();
            match args[0] {
                "unit" => {
                    assert_eq!(value.as_array().unwrap().len(), 5);
                    assert_eq!(value[4]["state"], "review-failed");
                }
                "ledger" => assert_eq!(value, serde_json::json!({"unit-test-verified":1})),
                "gate" | "standing" => assert_eq!(value, serde_json::json!([])),
                "inbox" => {
                    assert_eq!(value[0]["unit"], "legacy");
                    assert_eq!(value[0]["status"], "done");
                }
                "frontier" => assert_eq!(
                    value,
                    serde_json::json!({"generation":0,"prs":[],"lowestUnmerged":null})
                ),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn old_records_rewrite() {
    for fixture in ["old", "old-crlf"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/orch/stores")
            .join(fixture);
        let temp = Temp::new();
        for name in [
            "units.tsv",
            "ledger.tsv",
            "gates.md",
            "preferences.md",
            "frontier.json",
        ] {
            std::fs::copy(source.join(name), temp.0.join(name)).unwrap();
        }
        std::fs::create_dir(temp.0.join("inbox")).unwrap();
        for entry in std::fs::read_dir(source.join("inbox")).unwrap() {
            let e = entry.unwrap();
            std::fs::copy(e.path(), temp.0.join("inbox").join(e.file_name())).unwrap();
        }
        ok(
            &temp.0,
            &["unit", "set", "u5", "--state", "review-failed"],
            false,
        );
        let unit_bytes = std::fs::read(source.join("units.tsv")).unwrap();
        assert_eq!(
            std::fs::read(temp.0.join("units.tsv")).unwrap(),
            String::from_utf8(unit_bytes)
                .unwrap()
                .replace('\r', "")
                .as_bytes()
        );
        let record = ok(
            &temp.0,
            &[
                "ledger",
                "record",
                "3",
                "abc",
                "unit-test-verified",
                "--evidence",
                "proof",
                "--verifier",
                "verifier",
            ],
            true,
        );
        let row: serde_json::Value = serde_json::from_str(&record.stdout).unwrap();
        assert_eq!(std::fs::read_to_string(temp.0.join("ledger.tsv")).unwrap(), format!("pr\tsha\tverdict\tevidence\tverifier\tts\n3\tabc\tunit-test-verified\tproof\tverifier\t{}\n", row["ts"].as_str().unwrap()));
        ok(&temp.0, &["standing", "add", "legacy order"], false);
        assert_eq!(
            std::fs::read(temp.0.join("preferences.md")).unwrap(),
            b"1. legacy order\n"
        );
        assert_eq!(
            std::fs::read(temp.0.join("frontier.json")).unwrap(),
            std::fs::read(source.join("frontier.json")).unwrap()
        );
        let before = std::fs::read_to_string(temp.0.join("gates.md"))
            .unwrap()
            .replace('\r', "");
        ok(
            &temp.0,
            &["gate", "resolve", "decision", "--answer", "yes"],
            false,
        );
        assert_eq!(
            std::fs::read_to_string(temp.0.join("gates.md")).unwrap(),
            before
        );
        let peek = ok(&temp.0, &["inbox", "drain", "--peek"], false);
        assert!(peek.stdout.contains("worker\tlegacy\tdone"));
    }
}
#[test]
fn scope_outside_module_rejected() {
    let t = Temp::new();
    ok(&t.0, &["init"], false);
    std::fs::write(t.0.join("modules.md"),"## core\nPURPOSE x\nPATHS src/core/\nDEPENDS none\nCONTRACT\n  x\nACCEPT tests/core.rs, `cargo test`\n\nFULL TEST cargo test\n").unwrap();
    let brief = t.0.join("brief");
    std::fs::write(
        &brief,
        "GOAL g\nSCOPE src/other/file.rs\nVERIFY cargo test\n",
    )
    .unwrap();
    let out = call(
        &t.0,
        &[
            "unit",
            "add",
            "x",
            "--track",
            "core",
            "--brief",
            brief.to_str().unwrap(),
        ],
        false,
        false,
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("outside module"), "{}", out.stderr);
}

#[test]
fn invalid_unit_ids_rejected() {
    let t = Temp::new();
    ok(&t.0, &["init"], false);
    for id in ["U1", "a.b"] {
        let out = call(&t.0, &["unit", "add", id, "--track", "core"], false, false);
        assert_eq!(out.exit_code, 1, "{id}");
        assert!(out.stderr.contains("unit"));
    }
}

#[test]
fn compact_and_json_contracts() {
    let t = Temp::new();
    let p = &t.0;
    ok(p, &["init"], false);
    for id in ["u1", "u2", "u3", "u4", "u5"] {
        ok(p, &["unit", "add", id, "--track", "core"], false);
    }
    let value = |args: &[&str]| {
        serde_json::from_str::<serde_json::Value>(&ok(p, args, true).stdout).unwrap()
    };
    assert_eq!(
        ok(p, &["unit", "get", "u1"], false).stdout,
        "u1\tcore\tpending\t\t\t\t\n"
    );
    assert_eq!(
        value(&["unit", "get", "u1"]),
        serde_json::json!({"id":"u1","track":"core","state":"pending","branch":"","pr":"","sha":"","brief":""})
    );
    assert_eq!(ok(p, &["unit", "list"], false).stdout, "u1\tcore\tpending\t\t\t\t\nu2\tcore\tpending\t\t\t\t\nu3\tcore\tpending\t\t\t\t\nu4\tcore\tpending\t\t\t\t\n... 1 more; use --json\n");
    assert_eq!(value(&["unit", "list"]).as_array().unwrap().len(), 5);
    assert_eq!(ok(p, &["unit", "counts"], false).stdout, "pending=5\n");
    assert_eq!(value(&["unit", "counts"]), serde_json::json!({"pending":5}));
    ok(
        p,
        &[
            "ledger",
            "record",
            "3",
            "abc",
            "unit-test-verified",
            "--evidence",
            "proof",
        ],
        false,
    );
    assert_eq!(
        ok(p, &["ledger", "summary"], false).stdout,
        "unit-test-verified=1\n"
    );
    assert_eq!(
        value(&["ledger", "summary"]),
        serde_json::json!({"unit-test-verified":1})
    );
    ok(
        p,
        &[
            "gate",
            "park",
            "g",
            "--question",
            "Q?",
            "--options",
            "a,b",
            "--default",
            "a",
        ],
        false,
    );
    assert_eq!(ok(p, &["gate", "list"], false).stdout, "g\tQ?\ta,b\ta\n");
    assert_eq!(
        value(&["gate", "list"]),
        serde_json::json!([{"id":"g","kind":"open","question":"Q?","options":"a,b","defaultAnswer":"a"}])
    );
    ok(p, &["standing", "add", "order"], false);
    assert_eq!(ok(p, &["standing", "show"], false).stdout, "1. order\n");
    assert_eq!(
        value(&["standing", "show"]),
        serde_json::json!([{"number":1,"line":"order"}])
    );
    assert_eq!(
        ok(p, &["frontier", "show"], false).stdout,
        "generation=0 prs=none lowest-unmerged=none\n"
    );
    assert_eq!(
        value(&["frontier", "show"]),
        serde_json::json!({"generation":0,"prs":[],"lowestUnmerged":null})
    );
    assert_eq!(ok(p, &["status"], false).stdout, "counts: units=5; states=pending=5; ledger=unit-test-verified=1\nchanged: first render\ngates open: 1; ids=g\n");
    assert_eq!(ok(p, &["status"], false).stdout, "counts: units=5; states=pending=5; ledger=unit-test-verified=1\nchanged: no derived changes\ngates open: 1; ids=g\n");
    let status = value(&["status"]);
    assert_eq!(status["changed"], "no derived changes");
    assert_eq!(
        status["summary"],
        serde_json::json!({"unitStates":{"pending":5},"ledgerVerdicts":{"unit-test-verified":1},"frontierGeneration":0,"openGateIds":["g"]})
    );
    assert_eq!(status["units"].as_array().unwrap().len(), 5);
    assert_eq!(status["ledger"].as_array().unwrap().len(), 1);
    assert_eq!(status["gates"].as_array().unwrap().len(), 1);
}
