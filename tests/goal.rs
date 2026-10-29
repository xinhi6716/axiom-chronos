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
        goal::set_status(home, "s", "done", None).unwrap()["status"],
        "completed"
    );
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
