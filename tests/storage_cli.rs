mod common;

use axiom::storage::{Class, Ledger, Owner};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

fn command(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom"))
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env_remove("AXIOM_FAKE_DISK")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn registered(home: &Path) -> std::path::PathBuf {
    let path = home.join("scratch-test");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("data"), b"hello").unwrap();
    let l = Ledger::open(home).unwrap();
    l.register(
        &path,
        Class::Scratch,
        &Owner {
            project: "test".into(),
            wish: None,
        },
        json!({}),
    )
    .unwrap();
    l.touch(&path, 0).unwrap();
    path
}
#[test]
fn storage_clean_requires_yes() {
    let t = common::temp();
    let p = registered(t.path());
    let out = command(t.path(), &["storage", "clean"]);
    assert!(out.status.success(), "{:?}", out);
    let s = text(&out);
    assert!(
        s.contains("delete")
            && s.contains("scratch")
            && s.contains("scratch-test")
            && s.contains("nothing changed; run again with --yes"),
        "{s}"
    );
    assert!(p.exists());
    let out = command(t.path(), &["storage", "clean", "--pressure", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["plan"]["items"][0]["action"], "delete");
    assert!(p.exists());
    let out = command(t.path(), &["storage", "clean", "--yes"]);
    assert!(out.status.success(), "{:?}", out);
    assert!(!p.exists());
    assert!(text(&out).contains("delete"));
}
#[test]
fn storage_lists_usage_json() {
    let t = common::temp();
    registered(t.path());
    let out = command(t.path(), &["storage", "--json", "--measure"]);
    assert!(out.status.success(), "{:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["by_class"]["scratch"]["bytes"], 5);
    assert_eq!(v["reclaimable"]["entries"], 1);
}
#[test]
fn storage_worktrees_remove_requires_yes() {
    let t = common::temp();
    let home = t.path().join("home");
    let repo = t.path().join("repo");
    let wt = t.path().join("wt");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    fs::write(repo.join(".gitignore"), "/target/\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "initial"]);
    git(
        &repo,
        &["worktree", "add", "-b", "feature", wt.to_str().unwrap()],
    );
    fs::create_dir(wt.join("target")).unwrap();
    fs::write(wt.join("change"), "x").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "change"]);
    git(&repo, &["merge", "--no-ff", "feature", "-m", "merge"]);
    let out = command(
        &home,
        &[
            "storage",
            "worktrees",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(out.status.success(), "{:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["state"], "merged");
    assert_eq!(v[0]["cleanup_offer"], true);
    assert_eq!(v[0]["build_caches"].as_array().unwrap().len(), 1);
    let out = command(
        &home,
        &["storage", "worktrees", "remove", wt.to_str().unwrap()],
    );
    assert!(out.status.success(), "{:?}", out);
    assert!(text(&out).contains("target"));
    assert!(wt.exists());
    let out = command(
        &home,
        &[
            "storage",
            "worktrees",
            "remove",
            wt.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(out.status.success(), "{:?}", out);
    assert!(!wt.exists());
}
fn disk_command(home: &Path, free: u64, args: &[&str]) -> Output {
    let disk = home.join("disk.json");
    fs::write(&disk, json!({"free_bytes": free}).to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_axiom"))
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env("AXIOM_FAKE_DISK", disk)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
#[test]
fn launch_paused_exits_75() {
    let t = common::temp();
    let out = disk_command(
        t.path(),
        1,
        &[
            "launch",
            "--kind",
            "batch",
            "--need",
            "1G",
            "--",
            &common::python(),
            "-c",
            "pass",
        ],
    );
    assert_eq!(out.status.code(), Some(75), "{:?}", out);
    assert!(text(&out).contains("paused"));
}
#[test]
fn launch_started_prints_pid_json() {
    let t = common::temp();
    let dir = t.path().join("registered");
    let register = format!("{}=scratch", dir.display());
    let out = disk_command(
        t.path(),
        u64::MAX,
        &[
            "launch",
            "--kind",
            "cache",
            "--need",
            "1K",
            "--progress",
            "progress",
            "--marker",
            "marker",
            "--register",
            &register,
            "--",
            &common::python(),
            "-c",
            "pass",
        ],
    );
    assert!(out.status.success(), "{:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["pid"].as_u64().unwrap() > 0);
    assert_eq!(v["progress_file"], "progress");
    assert_eq!(v["marker_file"], "marker");
    assert_eq!(
        Ledger::open(t.path())
            .unwrap()
            .get(&dir)
            .unwrap()
            .unwrap()
            .class,
        Class::Scratch
    );
}
#[test]
fn wish_need_space_exits_75() {
    let t = common::temp();
    let disk = t.path().join("disk.json");
    let record = t.path().join("record");
    fs::create_dir_all(t.path().join("storage")).unwrap();
    fs::write(
        t.path().join("storage/policy.json"),
        json!({"reserve_bytes": 10, "estimates": {"wish": 20}}).to_string(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    for skill in ["wish", "research"] {
        fs::write(&disk, "{\"free_bytes\":100}").unwrap();
        // Always reap the child on failure or timeout; never touch other processes.
        struct Guard(Child);
        impl Drop for Guard {
            fn drop(&mut self) {
                if self.0.try_wait().ok().flatten().is_none() {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
        }
        let mut child = Guard(Command::new(env!("CARGO_BIN_EXE_axiom"))
            .env("AXIOM_HOME", t.path())
            .env("AXIOM_DATA_HOME", t.path())
            .env("AXIOM_FAKE_DISK", &disk)
            .env("AXIOM_DISK_POLL_MS", "50")
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .env("FAKE_CLAUDE_RECORD", &record)
            .args([skill, "--need-space", "1G", "test"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap());
        let stdout = child.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        loop {
            assert!(Instant::now() < deadline, "{skill} did not pause before the deadline");
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = rx.recv_timeout(remaining).expect("timed out or exited before pause");
            if line.contains("paused") {
                break;
            }
        }
        assert!(!record.exists());
        if skill == "wish" {
            axiom::base::atomic_write(&disk, b"{\"free_bytes\":4294967296}").unwrap();
        }
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "{skill} did not resume before the deadline");
            std::thread::sleep(Duration::from_millis(10));
        };
        if skill == "wish" {
            assert!(status.success(), "{skill} exited {status}");
            assert!(record.exists(), "{skill} did not start after space returned");
            fs::remove_file(&record).unwrap();
        } else {
            assert_eq!(status.code(), Some(75), "{skill} exited {status}");
            assert!(!record.exists());
        }
    }
}
#[test]
fn usage_errors_exit_2() {
    let t = common::temp();
    assert!(command(t.path(), &["storage", "--json"]).status.success());
    for args in [
        &["storage", "--bad"][..],
        &["storage", "clean", "--measure"],
        &["storage", "worktrees", "remove"],
        &["launch", "--kind", "bad", "--", "x"],
        &["launch", "--kind", "batch"],
        &["launch", "--kind", "batch", "--need", "bad", "--", "x"],
        &["wish", "--need-space", "bad", "test"],
    ] {
        let out = command(t.path(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }
}
