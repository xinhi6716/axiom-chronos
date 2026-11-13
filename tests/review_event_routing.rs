use axiom::orch::{self, receipts::{record_routing, RoutingReceipt}, Evidence};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture { _tmp: tempfile::TempDir, store: PathBuf, repo: PathBuf, tree: String }
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git").args(["init", "-q"]).current_dir(&repo).status().unwrap().success());
        fs::write(repo.join("a.txt"), "content").unwrap();
        let brief = store.join("brief.txt");
        fs::write(&brief, "GOAL x\nSCOPE a.txt\nVERIFY echo ok\n").unwrap();
        fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n", brief.display())).unwrap();
        let tree = orch::snapshot(&repo, &["a.txt".into()]);
        orch::append_evidence(&store, "u", &Evidence { kind: "verify".into(), command: "test".into(), exit_code: 0, commit: tree.clone(), ts: "1".into() }).unwrap();
        Self { _tmp: tmp, store, repo, tree }
    }
    fn event(&self) -> Value {
        orch::record_review(&self.store, &["u".into()], "chronos:reviewer", true, &self.repo).unwrap();
        fs::read_to_string(self.store.join("events.jsonl")).unwrap().lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).find(|e| e["kind"] == "review_recorded").unwrap()["data"].clone()
    }
}
fn receipt(unit: &str, attempt: &str) -> RoutingReceipt {
    RoutingReceipt { unit: unit.into(), attempt: attempt.into(), tier: Some("sol".into()), effort: Some("high".into()), risk: Some("low".into()), model_assigned: "assigned".into(), model_used: Some("used".into()), rule: "rule-a".into(), reason: "reason".into(), settings_version: None }
}
fn assert_fields(data: &Value, expected: Value) {
    for key in ["worker_model", "worker_tier", "worker_effort", "risk", "routing_rule", "attempt"] {
        assert_eq!(data.get(key), expected.get(key), "{key}");
        assert!(data.get(key).is_some(), "missing {key}");
    }
}
#[test]
fn event_carries_routing() {
    let f = Fixture::new();
    record_routing(&f.store, &receipt("u", "1")).unwrap();
    assert_fields(&f.event(), json!({"worker_model":"used","worker_tier":"sol","worker_effort":"high","risk":"low","routing_rule":"rule-a","attempt":"1"}));
}
#[test]
fn latest_receipt_wins() {
    let f = Fixture::new();
    record_routing(&f.store, &receipt("u", "1")).unwrap();
    let mut r = receipt("u", "2");
    r.model_used = None; r.model_assigned = "new-model".into(); r.tier = Some("new-tier".into()); r.effort = Some("new-effort".into()); r.risk = Some("new-risk".into()); r.rule = "new-rule".into();
    record_routing(&f.store, &r).unwrap();
    record_routing(&f.store, &receipt("other", "3")).unwrap();
    assert_fields(&f.event(), json!({"worker_model":"new-model","worker_tier":"new-tier","worker_effort":"new-effort","risk":"new-risk","routing_rule":"new-rule","attempt":"2"}));
}
#[test]
fn no_receipt_gives_nulls() {
    let f = Fixture::new();
    assert_fields(&f.event(), json!({"worker_model":null,"worker_tier":null,"worker_effort":null,"risk":null,"routing_rule":null,"attempt":null}));
}
#[test]
fn unknown_fields_null() {
    let f = Fixture::new();
    let mut r = receipt("u", "1"); r.risk = None; r.effort = None;
    record_routing(&f.store, &r).unwrap();
    assert_fields(&f.event(), json!({"worker_model":"used","worker_tier":"sol","worker_effort":null,"risk":null,"routing_rule":"rule-a","attempt":"1"}));
}
#[test]
fn existing_keys_unchanged() {
    let f = Fixture::new();
    record_routing(&f.store, &receipt("u", "1")).unwrap();
    let mut data = f.event();
    for key in ["worker_model", "worker_tier", "worker_effort", "risk", "routing_rule", "attempt"] { assert!(data.as_object_mut().unwrap().remove(key).is_some(), "missing {key}"); }
    assert_eq!(data, json!({"review_id":"u#1","unit":"u","role":"reviewer","reviewer":"chronos:reviewer","model":"chronos:reviewer","family":"anthropic","tree":f.tree,"paths":["a.txt"],"findings":[],"pass":true}));
}

