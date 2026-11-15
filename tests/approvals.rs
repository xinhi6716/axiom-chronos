mod common;

use axiom::engine::approvals::{self, Action, Origin};
use axiom::engine::kinds::*;
use axiom::memory::Memory;
use axiom::orch::events::{append_event, read_events};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

struct Fixture {
    temp: tempfile::TempDir,
    store: PathBuf,
    marker: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let store = temp.path().join("orchestrate/project/wish");
        fs::create_dir_all(&store).unwrap();
        let marker = temp.path().join("marker");
        Self {
            temp,
            store,
            marker,
        }
    }
    fn action(&self) -> Action {
        Action {
            command: common::python(),
            args: vec![
                "-c".into(),
                "import sys; open(sys.argv[1], 'a', encoding='utf-8').write('line\\n')".into(),
                self.marker.to_string_lossy().into_owned(),
            ],
            cwd: self.temp.path().to_string_lossy().into_owned(),
        }
    }
    fn verify_unit(&self, id: &str) {
        let project_store = self.temp.path().join("orchestrate/project");
        let brief = project_store.join("approval-brief.md");
        fs::write(self.temp.path().join("scope.txt"), "evidence").unwrap();
        fs::write(&brief, "GOAL test\nSCOPE scope.txt\nVERIFY test\n").unwrap();
        fs::write(
            project_store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n{id}\tx\tverified\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
    }
    fn request(&self) -> approvals::Requested {
        approvals::request(&self.store, self.action(), "run", Origin::Model).unwrap()
    }
    fn reply(
        &self,
        id: &str,
        approve: bool,
        hash: &str,
        wish: &str,
        reason: Option<&str>,
        origin: Origin,
    ) -> axiom::base::Res<axiom::orch::events::Event> {
        approvals::reply(
            self.temp.path(),
            &self.store,
            id,
            approve,
            hash,
            wish,
            reason,
            origin,
        )
    }
    fn lines(&self) -> usize {
        fs::read_to_string(&self.marker)
            .unwrap_or_default()
            .lines()
            .count()
    }
}
fn state(store: &Path, id: &str, expected: &str) {
    assert_eq!(approvals::status(store, id).unwrap()["status"], expected);
}

#[test]
fn approved_action_is_claimed_once_with_concurrent_executions() {
    let f = Fixture::new();
    let req = f.request();
    state(&f.store, &req.id, "requested");
    f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
        .unwrap();
    state(&f.store, &req.id, "approved");
    assert_eq!(
        f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
            .unwrap_err()
            .kind,
        "already_replied"
    );
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = f.store.clone();
            let id = req.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                approvals::execute(&store, &id)
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| e.kind == "already_executed"));
    assert_eq!(
        results
            .iter()
            .find_map(|r| r.as_ref().ok())
            .unwrap()
            .exit_code,
        0
    );
    assert_eq!(
        approvals::execute(&f.store, &req.id).unwrap_err().kind,
        "already_executed"
    );
    assert_eq!(f.lines(), 1);
    state(&f.store, &req.id, "executed");
    let events = read_events(&f.store, 0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == APPROVAL_EXECUTING)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == APPROVAL_EXECUTED)
            .count(),
        1
    );
}

#[test]
fn refuses_wrong_wish_model_and_tampering() {
    let f = Fixture::new();
    let req = f.request();
    assert_eq!(
        f.reply(&req.id, true, &req.digest, "other", None, Origin::Person)
            .unwrap_err()
            .kind,
        "wrong_wish"
    );
    assert_eq!(
        f.reply(&req.id, true, &req.digest, "wish", None, Origin::Model)
            .unwrap_err()
            .kind,
        "person_only"
    );
    assert_eq!(
        f.reply(&req.id, true, "bad", "wish", None, Origin::Person)
            .unwrap_err()
            .kind,
        "tampered"
    );
    // Replace the stored action while retaining its original digest.
    let file = f.store.join("events.jsonl");
    let mut event: serde_json::Value =
        serde_json::from_str(fs::read_to_string(&file).unwrap().trim()).unwrap();
    event["data"]["action"]["command"] = json!("altered");
    fs::write(&file, format!("{}\n", event)).unwrap();
    assert_eq!(
        f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
            .unwrap_err()
            .kind,
        "tampered"
    );
    assert_eq!(f.lines(), 0);
}

#[test]
fn denial_is_a_correction_and_never_executes() {
    let f = Fixture::new();
    let req = f.request();
    f.verify_unit(&req.id);
    assert_eq!(
        approvals::execute(&f.store, &req.id).unwrap_err().kind,
        "not_approved"
    );
    assert_eq!(
        f.reply(
            &req.id,
            false,
            &req.digest,
            "wish",
            Some("  "),
            Origin::Person
        )
        .unwrap_err()
        .kind,
        "invalid_argument"
    );
    let denied = f
        .reply(
            &req.id,
            false,
            &req.digest,
            "wish",
            Some("unsafe"),
            Origin::Person,
        )
        .unwrap();
    assert_eq!(denied.kind, APPROVAL_DENIED);
    assert_eq!(denied.data["reason"], "unsafe");
    assert_eq!(
        denied.data["action"],
        serde_json::to_value(f.action()).unwrap()
    );
    let memory = Memory::open(f.temp.path()).unwrap();
    let entries = memory.list(Some("project"), 10).unwrap();
    let entries = entries["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = memory.get(entries[0]["id"].as_i64().unwrap()).unwrap();
    let entry = &entry["entry"];
    assert_eq!(
        entry["title"],
        format!("correction: approval {} denied", req.id)
    );
    assert_eq!(entry["summary"], "unsafe");
    assert_eq!(entry["body"], serde_json::to_string(&f.action()).unwrap());
    assert!(!entry["project_dir"].as_str().unwrap().starts_with(r"\\?\"));
    state(&f.store, &req.id, "denied");
    assert_eq!(
        approvals::execute(&f.store, &req.id).unwrap_err().kind,
        "not_approved"
    );
    assert_eq!(
        f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
            .unwrap_err()
            .kind,
        "already_replied"
    );
    assert_eq!(f.lines(), 0);
}

#[test]
fn execution_checks_action_after_approval() {
    let f = Fixture::new();
    let req = f.request();
    f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
        .unwrap();
    let file = f.store.join("events.jsonl");
    let content = fs::read_to_string(&file).unwrap();
    let mut rows: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    rows[0]["data"]["action"]["command"] = json!("altered");
    fs::write(
        &file,
        rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
    )
    .unwrap();
    assert_eq!(
        approvals::execute(&f.store, &req.id).unwrap_err().kind,
        "tampered"
    );
    assert_eq!(f.lines(), 0);
    assert!(!read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == APPROVAL_EXECUTING));
}

#[test]
fn digest_is_canonical_and_output_tail_is_limited() {
    let f = Fixture::new();
    let action = Action {
        command: common::python(),
        args: vec!["-c".into(), "print('x' * 2100)".into()],
        cwd: f.temp.path().to_string_lossy().into_owned(),
    };
    let canonical =
        json!({"wish":"wish", "command":action.command, "args":action.args, "cwd":action.cwd});
    use sha2::{Digest, Sha256};
    assert_eq!(
        approvals::digest("wish", &action),
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&canonical).unwrap())
        )
    );
    let req = approvals::request(&f.store, action, "output", Origin::Person).unwrap();
    f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
        .unwrap();
    let result = approvals::execute(&f.store, &req.id).unwrap();
    assert_eq!(result.output_tail.chars().count(), 2000);
    assert!(result.output_tail.ends_with('\n'));
    // Other events do not change status.
    append_event(&f.store, "unrelated", None, None, json!({"id":req.id})).unwrap();
    state(&f.store, &req.id, "executed");
}

#[test]
fn denial_propagates_memory_record_errors() {
    let f = Fixture::new();
    let req = f.request();
    f.verify_unit(&req.id);
    fs::remove_file(f.temp.path().join("scope.txt")).unwrap();
    let error = f
        .reply(
            &req.id,
            false,
            &req.digest,
            "wish",
            Some("unsafe"),
            Origin::Person,
        )
        .unwrap_err();
    assert_eq!(error.kind, "invalid_argument");
    assert!(error.message.contains("files are required"), "{error}");
    state(&f.store, &req.id, "requested");
    assert!(!read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == APPROVAL_DENIED));
}

#[test]
fn missing_cwd_reaches_memory_validation_with_raw_path() {
    let f = Fixture::new();
    let mut action = f.action();
    action.cwd = f.temp.path().join("gone").to_string_lossy().into_owned();
    let req = approvals::request(&f.store, action.clone(), "run", Origin::Model).unwrap();
    let error = f
        .reply(
            &req.id,
            false,
            &req.digest,
            "wish",
            Some("unsafe"),
            Origin::Person,
        )
        .unwrap_err();
    assert_eq!(error.kind, "invalid_argument");
    assert!(
        error
            .message
            .contains("project_dir must be an existing absolute directory"),
        "{error}"
    );
    assert!(error.message.contains("gone"), "{error}");
    state(&f.store, &req.id, "requested");
}

#[test]
fn spawn_failure_is_recorded_and_cannot_be_retried() {
    let f = Fixture::new();
    let mut action = f.action();
    action.command = f
        .temp
        .path()
        .join("nonexistent-command")
        .to_string_lossy()
        .into_owned();
    let req = approvals::request(&f.store, action, "run", Origin::Person).unwrap();
    f.reply(&req.id, true, &req.digest, "wish", None, Origin::Person)
        .unwrap();
    let error = approvals::execute(&f.store, &req.id).unwrap_err();
    assert_eq!(error.kind, "io");
    let events = read_events(&f.store, 0).unwrap();
    let executed: Vec<_> = events
        .iter()
        .filter(|e| e.kind == APPROVAL_EXECUTED)
        .collect();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].data["id"], req.id);
    assert_eq!(executed[0].data["exit_code"], -1);
    assert_eq!(executed[0].data["output_tail"], error.message);
    state(&f.store, &req.id, "executed");
    assert_eq!(
        approvals::execute(&f.store, &req.id).unwrap_err().kind,
        "already_executed"
    );
}
