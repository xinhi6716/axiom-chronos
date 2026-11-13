use axiom::{orch::{self, lifecycle::scheduler}, session::stage::{self, Stage}};
use serde_json::json;
use std::{fs, path::PathBuf, time::Duration};

struct Fixture { home: tempfile::TempDir, store: PathBuf }
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let store = orch::wish_store(home.path(), "project", Some("w-1234567890123-abcdef")).unwrap();
        let f = Self { home, store };
        f.ok(&["init"]);
        fs::create_dir_all(f.home.path().join("goals")).unwrap();
        fs::write(f.home.path().join("goals/s.json"), json!({"wish":"test", "status":"active", "project":"project", "wish_id":"w-1234567890123-abcdef"}).to_string()).unwrap();
        fs::create_dir_all(f.store.join("astra")).unwrap();
        fs::write(f.store.join("astra/plan-final.md"), "plan").unwrap();
        fs::write(f.store.join("modules.md"), "## core\nPURPOSE x\nPATHS src/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
        f
    }
    fn call(&self, args: &[&str]) -> orch::Output {
        orch::store::run(&self.store, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true, false)
    }
    fn ok(&self, args: &[&str]) {
        let out = self.call(args);
        assert_eq!(out.exit_code, 0, "{args:?}: {}", out.stderr);
    }
    fn add(&self, id: &str, old: Option<&str>) -> orch::Output {
        let brief = self.store.join(format!("{id}.md"));
        fs::write(&brief, "GOAL g\nSCOPE src/a\nVERIFY echo ok\n").unwrap();
        let mut args = vec!["unit", "add", id, "--track", "core", "--brief", brief.to_str().unwrap()];
        if let Some(old) = old { args.extend(["--fixes", old]); }
        self.call(&args)
    }
    fn chain(&self, prefix: &str) -> String {
        let mut parent = None;
        for n in 0..3 {
            let id = format!("{prefix}{n}");
            let out = self.add(&id, parent.as_deref());
            assert_eq!(out.exit_code, 0, "{}", out.stderr);
            parent = Some(id);
        }
        parent.unwrap()
    }
    fn exhaust(&self, prefix: &str) -> String {
        let unit = self.chain(prefix);
        let out = self.add(&format!("{prefix}3"), Some(&unit));
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("fix_rounds"), "{}", out.stderr);
        unit
    }
    fn pending(&self) -> String {
        fs::read_to_string(self.store.join("adjudication.pending")).unwrap_or_default()
    }
    fn current(&self) -> Stage { stage::current(self.home.path(), "s") }
    fn review(&self, unit: &str, count: usize) {
        let findings: Vec<_> = (0..count).map(|_| json!({"severity":"blocker", "check":"x", "location":"src/a:1", "evidence":"x", "fix":"x"})).collect();
        fs::write(orch::verdict_path(&self.store, unit), json!({"unit":unit,"tree":"tree:0","verdict":"fail","findings":findings}).to_string()).unwrap();
        orch::append_evidence(&self.store, unit, &orch::Evidence { kind:"review".into(), command:"test".into(), exit_code:1, commit:"tree:0".into(), ts:"2026-01-01T00:00:00Z".into() }).unwrap();
    }
    fn waive(&self, unit: &str, number: &str) { self.ok(&["waive", unit, number, "--reason", "core ruling"]); }
    fn stuck(&self) {
        scheduler::claim(&self.store, "core").unwrap();
        scheduler::release(&self.store, "core", "awaiting core ruling").unwrap();
        let reply = scheduler::next(&self.store, None, Duration::ZERO).unwrap();
        assert_eq!(reply.stuck.len(), 1);
        assert_eq!(reply.stuck[0].reason, "released");
    }
}

#[test]
fn fix_limit_writes_unit_and_escalates_idempotently() {
    let f = Fixture::new();
    assert_eq!(f.current(), Stage::Build);
    let unit = f.exhaust("u");
    assert_eq!(f.pending(), format!("fix_rounds {unit}\n"));
    assert_eq!(f.current(), Stage::Escalation);
    let out = f.add("u3", Some(&unit));
    assert!(out.stderr.contains("fix_rounds"));
    assert_eq!(f.pending(), format!("fix_rounds {unit}\n"));
}

#[test]
fn stuck_module_writes_marker_and_claim_clears_it() {
    let f = Fixture::new();
    f.stuck();
    assert_eq!(f.pending(), "stuck core\n");
    assert_eq!(f.current(), Stage::Escalation);
    f.ok(&["schedule", "claim", "core"]);
    assert!(f.pending().is_empty());
    assert_eq!(f.current(), Stage::Build);
}

#[test]
fn waive_last_blocking_finding_clears_unit_and_stage() {
    let f = Fixture::new();
    let unit = f.exhaust("u");
    f.review(&unit, 2);
    f.waive(&unit, "1");
    assert_eq!(f.pending(), format!("fix_rounds {unit}\n"));
    f.waive(&unit, "2");
    assert!(f.pending().is_empty());
    assert_eq!(f.current(), Stage::Build);
}

#[test]
fn core_claim_clears_pending_units_in_module() {
    let f = Fixture::new();
    f.exhaust("u");
    assert!(!f.pending().is_empty());
    f.ok(&["schedule", "claim", "core"]);
    assert!(f.pending().is_empty());
    assert_eq!(f.current(), Stage::Build);
}

#[test]
fn clearing_one_of_two_units_keeps_the_other() {
    let f = Fixture::new();
    let a = f.exhaust("a");
    let b = f.exhaust("b");
    assert_eq!(f.pending().lines().count(), 2);
    f.review(&a, 1);
    f.waive(&a, "1");
    assert_eq!(f.pending(), format!("fix_rounds {b}\n"));
    assert_eq!(f.current(), Stage::Escalation);
}

#[test]
fn normal_fix_and_healthy_module_do_not_write_marker() {
    let f = Fixture::new();
    f.chain("u");
    scheduler::next(&f.store, None, Duration::ZERO).unwrap();
    assert!(!f.store.join("adjudication.pending").exists());
}

#[test]
fn unrelated_waive_keeps_marker() {
    let f = Fixture::new();
    let unit = f.exhaust("u");
    assert_eq!(f.add("other", None).exit_code, 0);
    f.review("other", 1);
    f.waive("other", "1");
    assert_eq!(f.pending(), format!("fix_rounds {unit}\n"));
}

#[test]
fn module_waive_waits_for_other_blocking_units() {
    let f = Fixture::new();
    let a = f.exhaust("a");
    let b = f.exhaust("b");
    f.stuck();
    f.review(&a, 1);
    f.review(&b, 1);
    f.waive(&a, "1");
    assert_eq!(f.pending(), format!("fix_rounds {b}\nstuck core\n"));
    f.waive(&b, "1");
    assert!(f.pending().is_empty());
    assert_eq!(f.current(), Stage::Build);
}

#[test]
fn module_claim_keeps_other_module_and_stuck_is_idempotent() {
    let f = Fixture::new();
    f.exhaust("u");
    f.stuck();
    let map = fs::read_to_string(f.store.join("modules.md")).unwrap();
    fs::write(f.store.join("modules.md"), map + "## other\nPURPOSE x\nPATHS other/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    scheduler::next(&f.store, None, Duration::ZERO).unwrap();
    scheduler::release(&f.store, "other", "awaiting ruling").unwrap();
    scheduler::next(&f.store, None, Duration::ZERO).unwrap();
    scheduler::next(&f.store, None, Duration::ZERO).unwrap();
    assert_eq!(f.pending().lines().count(), 3);
    f.ok(&["schedule", "claim", "core"]);
    assert_eq!(f.pending(), "stuck other\n");
    assert_eq!(f.current(), Stage::Escalation);
}
