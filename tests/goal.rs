use axiom::goal;
use std::fs;

#[test]
fn legacy_and_interjections() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    fs::create_dir(home.join("goals")).unwrap();
    for prior in ["active", "done", "blocked"] {
        for kind in ["amend", "answer", "stop"] {
            let original = serde_json::json!({"wish":"修理", "cwd":null, "status":prior, "amendments":["先前"], "note":"舊備註", "extra":42});
            fs::write(
                home.join("goals/s.json"),
                serde_json::to_vec(&original).unwrap(),
            )
            .unwrap();
            let opened = goal::interject(home, "s", "請停止？").unwrap().unwrap();
            assert_eq!(opened.status, "active");
            let result = goal::classify(home, "s", kind).unwrap();
            match kind {
                "amend" => {
                    assert_eq!(result.status, "active");
                    assert_eq!(result.amendments.len(), 2);
                }
                "answer" => {
                    assert_eq!(result.status, prior);
                    assert_eq!(result.note.as_deref(), Some("舊備註"));
                    assert_eq!(result.amendments, ["先前"]);
                }
                _ => {
                    assert_eq!(result.status, "blocked");
                    assert_eq!(result.note.as_deref(), Some("請停止？"));
                    assert_eq!(result.reason.as_deref(), Some("user_stop"));
                }
            }
            assert!(result.interjection.is_none());
            assert_eq!(result.other["extra"], 42);
            let raw = fs::read_to_string(home.join("goals/s.json")).unwrap();
            assert!(raw.contains("舊備註") || kind == "stop");
            assert!(raw.contains("\n \"wish\""));
            assert!(raw.contains("修理"));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&raw).unwrap()["extra"],
                42
            );
        }
    }
    goal::interject(home, "s", "pending").unwrap();
    assert_eq!(
        goal::classify(home, "s", "invalid").unwrap_err().kind,
        "invalid_argument"
    );
    goal::classify(home, "s", "amend").unwrap();
    let before = fs::read(home.join("goals/s.json")).unwrap();
    assert!(goal::interject(home, "s", " <task-notification>")
        .unwrap()
        .is_none());
    assert_eq!(before, fs::read(home.join("goals/s.json")).unwrap());
    assert_eq!(
        goal::classify(home, "s", "amend").unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(fs::read_dir(home.join("goals")).unwrap().count(), 1);
}

#[test]
fn modes_status_and_project_gate() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    goal::open(home, "session-1", "[astra] 願望", Some("project")).unwrap();
    assert!(goal::astra_requested(
        &goal::load(home, "session-1").unwrap().unwrap()
    ));
    goal::set_mode(home, "session-1", "change").unwrap();
    assert_eq!(
        goal::set_mode(home, "session-1", "review")
            .unwrap_err()
            .kind,
        "mode_locked"
    );
    assert_eq!(
        goal::set_status(home, "session-1", "blocked", Some(" "))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        goal::set_status(home, "session-1", "done", None)
            .unwrap_err()
            .kind,
        "not_done"
    );
    goal::interject(home, "session-1", "問題").unwrap();
    goal::set_mode(home, "session-1", "review").unwrap();
    assert_eq!(
        goal::set_status(home, "session-1", "done", None).unwrap()["status"],
        "completed"
    );
    assert!(goal::find_active(home, "project").unwrap().is_none());
    assert_eq!(
        goal::load(home, "../bad").unwrap_err().kind,
        "invalid_argument"
    );
}

#[test]
#[cfg(any(unix, windows))]
fn missing_store_gate_does_not_bypass_live_store_lock_or_corrupt_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let store = home.join("orchestrate/project");
    goal::open(home, "change", "wish", Some("project")).unwrap();
    goal::set_mode(home, "change", "change").unwrap();
    assert_eq!(
        goal::set_status(home, "change", "done", None).unwrap_err().kind,
        "not_done"
    );
    assert!(!store.exists());
    goal::open(home, "legacy", "wish", Some("project")).unwrap();
    assert_eq!(
        goal::set_status(home, "legacy", "done", None).unwrap()["status"],
        "completed"
    );
    assert!(!store.exists());

    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("core-wait.json"), "invalid json").unwrap();
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let lock = options.open(store.join(".orch.mutex")).unwrap();
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }
    goal::open(home, "locked", "wish", Some("project")).unwrap();
    assert_eq!(
        goal::set_status(home, "locked", "done", None).unwrap_err().kind,
        "busy"
    );
    assert_eq!(
        goal::load(home, "locked").unwrap().unwrap().status,
        "active"
    );
    drop(lock);
    assert_eq!(
        goal::set_status(home, "locked", "done", None).unwrap_err().kind,
        "invalid_argument"
    );
}

#[test]
fn done_requires_verified_units_and_current_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let store = home.join("orchestrate/project");
    fs::create_dir_all(store.join("reports")).unwrap();
    let header = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
    fs::write(
        store.join("units.tsv"),
        format!("{header}unit\tt\tpending\t\t\t\t\n"),
    )
    .unwrap();
    goal::open(home, "s", "wish", Some("project")).unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap_err().kind,
        "not_done"
    );
    fs::write(
        store.join("units.tsv"),
        format!("{header}unit\tt\tverified\t\t\t\t\n"),
    )
    .unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap_err().kind,
        "not_done"
    );
    let evidence_header = "kind\tcommand\texit_code\tcommit\tts\n";
    fs::write(
        store.join("reports/_full.evidence.tsv"),
        format!("{evidence_header}full\ttest\t0\t\t9999\n"),
    )
    .unwrap();
    fs::write(
        store.join("reports/_stages.evidence.tsv"),
        format!("{evidence_header}rewrite\tmodel\t0\t\t9999\n"),
    )
    .unwrap();
    fs::write(
        store.join("reports/unit.evidence.tsv"),
        format!("{evidence_header}unit\ttest\t0\t\t9999z\n"),
    )
    .unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap_err().kind,
        "not_done"
    );
    fs::remove_file(store.join("reports/unit.evidence.tsv")).unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap_err().kind,
        "not_done"
    );
}

#[test]
fn done_uses_goal_cwd_for_current_sources() {
    use axiom::orch::*;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let repo = home.join("project");
    fs::create_dir(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("file.txt"), "verified").unwrap();
    let goal = goal::open(home, "s", "wish", Some(repo.to_str().unwrap())).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
    let store = store_for(home, goal.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::create_dir(store.join("inbox")).unwrap();
    let brief = store.join("unit.brief");
    fs::write(&brief, "GOAL test\nSCOPE file.txt\nVERIFY echo ok\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nunit\tt\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    begin_unit(&store, "unit", "worker", &repo, false).unwrap();
    finish_unit(
        &store,
        "unit",
        &repo,
        "worker",
        "gpt-worker",
        &serde_json::json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    record_review(&store, &["unit".into()], "claude-review", true, &repo).unwrap();
    for kind in ["full", "rewrite"] {
        append_evidence(
            &store,
            if kind == "full" { "_full" } else { "_stages" },
            &Evidence {
                kind: kind.into(),
                command: "test".into(),
                exit_code: 0,
                commit: snapshot(&repo, &[".".into()]),
                ts: "9999".into(),
            },
        )
        .unwrap();
    }
    fs::write(repo.join("file.txt"), "unverified").unwrap();
    assert!(goal::set_status(home, "s", "done", None)
        .unwrap_err()
        .message
        .contains("stale verification"));
    assert_eq!(goal::load(home, "s").unwrap().unwrap().status, "active");
    fs::write(repo.join("file.txt"), "verified").unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap()["status"],
        "completed"
    );
}

#[test]
fn readonly_goals_ignore_old_project_engineering_units() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let store = home.join("orchestrate/project");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nold\tt\tfailed\t\t\t\t\n",
    )
    .unwrap();
    for mode in ["answer", "review"] {
        goal::open(home, mode, "readonly wish", Some("project")).unwrap();
        goal::set_mode(home, mode, mode).unwrap();
        assert_eq!(
            goal::set_status(home, mode, "done", None).unwrap()["status"],
            "completed"
        );
    }
}

#[test]
fn interjection_answer_restores_original_mode_and_lock() {
    let temp = tempfile::tempdir().unwrap();
    for mode in [None, Some("change"), Some("review"), Some("answer")] {
        goal::open(temp.path(), "s", "wish", None).unwrap();
        if let Some(mode) = mode {
            goal::set_mode(temp.path(), "s", mode).unwrap();
        }
        goal::interject(temp.path(), "s", "first question").unwrap();
        goal::set_mode(temp.path(), "s", "answer").unwrap();
        goal::interject(temp.path(), "s", "second question").unwrap();
        let restored = goal::classify(temp.path(), "s", "answer").unwrap();
        assert_eq!(restored.mode.as_deref(), mode);
        assert_eq!(restored.mode_locked, mode.is_some());
        if mode.is_some() {
            assert_eq!(
                goal::set_mode(temp.path(), "s", "answer").unwrap_err().kind,
                "mode_locked"
            );
        } else {
            goal::set_mode(temp.path(), "s", "change").unwrap();
        }
    }
}

#[test]
fn legacy_without_note_round_trips_raw_format() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    fs::create_dir(home.join("goals")).unwrap();
    fs::write(home.join("goals/s.json"), "{\n \"wish\": \"修理願望\",\n \"cwd\": \"project\",\n \"status\": \"active\",\n \"amendments\": [],\n \"extra\": 42\n}").unwrap();
    goal::interject(home, "s", "追加").unwrap();
    goal::classify(home, "s", "amend").unwrap();
    let raw = fs::read_to_string(home.join("goals/s.json")).unwrap();
    assert!(raw.contains("修理願望"));
    assert!(raw.contains("\n \"wish\""));
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["cwd"], "project");
    assert_eq!(value["extra"], 42);
    for key in [
        "note",
        "mode",
        "mode_locked",
        "asked",
        "dispatched",
        "reason",
        "interjection",
    ] {
        assert!(value.get(key).is_none(), "unexpected {key}");
    }
}

#[test]
fn pending_interjections_preserve_origin_and_clear_reason() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    goal::open(home, "s", "wish", None).unwrap();
    goal::interject(home, "s", "stop").unwrap();
    goal::classify(home, "s", "stop").unwrap();
    let first = goal::interject(home, "s", "first").unwrap().unwrap();
    assert_eq!(first.status, "active");
    goal::interject(home, "s", "second").unwrap();
    let result = goal::classify(home, "s", "answer").unwrap();
    assert_eq!(result.status, "blocked");
    assert_eq!(result.note.as_deref(), Some("stop"));
    assert_eq!(result.amendments, ["stop", "first"]);
}

#[test]
fn interject_clears_reason() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    goal::open(home, "s", "wish", None).unwrap();
    goal::interject(home, "s", "stop").unwrap();
    goal::classify(home, "s", "stop").unwrap();
    assert!(goal::interject(home, "s", "again")
        .unwrap()
        .unwrap()
        .reason
        .is_none());
}

#[test]
fn set_status_clears_pending_interjection() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    goal::open(home, "s", "wish", None).unwrap();
    goal::interject(home, "s", "pending").unwrap();
    goal::set_status(home, "s", "blocked", Some("blocked")).unwrap();
    assert!(goal::load(home, "s")
        .unwrap()
        .unwrap()
        .interjection
        .is_none());
}

#[test]
fn find_active_selects_newest_matching_project() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    goal::open(home, "first", "one", Some("project")).unwrap();
    goal::open(home, "second", "two", Some("project")).unwrap();
    goal::open(home, "other", "other", Some("other")).unwrap();
    for session in ["first", "second", "other"] {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        goal::mark_asked(home, session).unwrap();
    }
    assert_eq!(
        goal::find_active(home, "project").unwrap().unwrap().0,
        "second"
    );
}

#[test]
fn interrupt_reopen_and_markers() {
    let t = tempfile::tempdir().unwrap();
    for (wish, creative, astra) in [
        ("[creative] x", true, true),
        ("[no-architect] [creative] x", true, true),
        ("[astra] x", false, true),
        ("x", false, false),
    ] {
        let g = axiom::goal::open(t.path(), "markers", wish, None).unwrap();
        assert_eq!(axiom::goal::creative_requested(&g), creative);
        assert_eq!(axiom::goal::astra_requested(&g), astra);
    }
    axiom::goal::interject(t.path(), "markers", "amend").unwrap();
    let file = t.path().join("goals/markers.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    data["project"] = "project".into();
    data["mode"] = "creative".into();
    data["mode_locked"] = true.into();
    data["asked"] = true.into();
    data["dispatched"] = true.into();
    data["unknown_key"] = serde_json::json!({"retained": true});
    std::fs::write(&file, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(axiom::goal::interrupt(t.path(), "missing", "crash")
        .unwrap()
        .is_none());
    let g = axiom::goal::interrupt(t.path(), "markers", "crash")
        .unwrap()
        .unwrap();
    assert_eq!(g.reason.as_deref(), Some("interrupted"));
    assert_eq!(
        g.note.as_deref(),
        Some("crash. Resume with: axiom wish --resume markers")
    );
    assert!(g.interjection.is_none());
    let before = std::fs::read(t.path().join("goals/markers.json")).unwrap();
    assert!(axiom::goal::interrupt(t.path(), "markers", "again")
        .unwrap()
        .is_none());
    assert_eq!(
        before,
        std::fs::read(t.path().join("goals/markers.json")).unwrap()
    );
    let g = axiom::goal::reopen(t.path(), "markers").unwrap();
    assert_eq!(g.status, "active");
    assert_eq!(g.amendments, ["amend"]);
    assert_eq!(g.project.as_deref(), Some("project"));
    assert_eq!(g.mode.as_deref(), Some("creative"));
    assert!(g.mode_locked && g.asked && g.dispatched);
    assert_eq!(
        g.other["unknown_key"],
        serde_json::json!({"retained": true})
    );
    assert!(g.note.is_none() && g.reason.is_none() && g.interjection.is_none());
    axiom::goal::set_status(t.path(), "markers", "done", None).unwrap();
    let done_before = std::fs::read(&file).unwrap();
    assert!(axiom::goal::interrupt(t.path(), "markers", "crash")
        .unwrap()
        .is_none());
    assert_eq!(std::fs::read(&file).unwrap(), done_before);
    assert_eq!(
        axiom::goal::reopen(t.path(), "markers").unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        axiom::goal::reopen(t.path(), "missing").unwrap_err().kind,
        "not_found"
    );
}
