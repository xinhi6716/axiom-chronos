use axiom::orch::{self, Evidence, Output};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn call(store: &Path, args: &[&str]) -> Output {
    orch::run(
        store,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        false,
        false,
    )
}

fn ok(store: &Path, args: &[&str]) {
    let out = call(store, args);
    assert_eq!(out.exit_code, 0, "{args:?}: {}", out.stderr);
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    ok(p, &["init"]);
    for (id, scope) in [("old", "src/a"), ("new", "src/a"), ("dependent", "src/b")] {
        let brief = p.join(format!("{id}.md"));
        fs::write(&brief, format!("GOAL g\nSCOPE {scope}\nVERIFY echo ok\n")).unwrap();
        let (unit, track) = match id {
            "old" => ("hello-api-fix", "build"),
            "new" => ("hello-api-fix2", "workshop-host"),
            _ => ("dependent", "client"),
        };
        ok(
            p,
            &[
                "unit",
                "add",
                unit,
                "--track",
                track,
                "--brief",
                brief.to_str().unwrap(),
            ],
        );
    }
    dir
}

fn evidence(store: &Path, unit: &str, kind: &str, command: &str) {
    orch::append_evidence(
        store,
        unit,
        &Evidence {
            kind: kind.into(),
            command: command.into(),
            exit_code: 0,
            commit: "tree:abc".into(),
            ts: "2026-01-01T00:00:00Z".into(),
        },
    )
    .unwrap();
}

fn verify(store: &Path, unit: &str, accept: bool) {
    evidence(store, unit, "run", "worker");
    evidence(store, unit, "verify", "echo ok");
    if accept {
        evidence(store, unit, "accept", "echo ok");
    }
    evidence(store, unit, "review", "engine-astra:review");
    assert_eq!(
        orch::derive_state(&orch::evidence(store, unit).unwrap(), accept),
        "verified"
    );
}

fn snapshot(store: &Path) -> Vec<Vec<u8>> {
    ["units.tsv", "claims.tsv", "events.jsonl"]
        .iter()
        .map(|file| match fs::read(store.join(file)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => panic!("{file}: {e}"),
        })
        .collect()
}

fn supersede(store: &Path) -> Output {
    call(
        store,
        &[
            "unit",
            "supersede",
            "hello-api-fix",
            "--by",
            "hello-api-fix2",
        ],
    )
}

#[test]
fn orphan_cross_track_success_records_basis_and_preserves_evidence() {
    for (basis, has_evidence) in [("state", false), ("state", true), ("evidence", true)] {
        let dir = fixture();
        let p = dir.path();
        // build is deliberately absent from this module map.
        fs::write(p.join("modules.md"), "## workshop-host\nPURPOSE x\nPATHS src/a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
        evidence(p, "hello-api-fix", "run", "worker");
        let before = fs::read(p.join("reports/hello-api-fix.evidence.tsv")).unwrap();
        if has_evidence {
            verify(p, "hello-api-fix2", true);
        }
        if basis == "state" {
            orch::set_unit_state(p, "hello-api-fix2", "verified").unwrap();
        }
        let out = supersede(p);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        assert_eq!(
            orch::read_units(p)
                .unwrap()
                .iter()
                .find(|u| u.id == "hello-api-fix")
                .unwrap()
                .state,
            "superseded:hello-api-fix2"
        );
        assert_eq!(
            fs::read(p.join("reports/hello-api-fix.evidence.tsv")).unwrap(),
            before
        );
        let lines = fs::read_to_string(p.join("events.jsonl")).unwrap();
        let events: Vec<Value> = lines
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let events: Vec<_> = events
            .iter()
            .filter(|e| e["kind"] == "unit_superseded_cross_track")
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["data"],
            json!({"old":"hello-api-fix", "new":"hello-api-fix2", "old_track":"build", "new_track":"workshop-host", "basis":basis})
        );
        assert!(events[0]["module"].is_null() && events[0]["unit"].is_null());
    }
}

#[test]
fn cross_track_replacement_unblocks_dependent() {
    let dir = fixture();
    let p = dir.path();
    fs::write(p.join("modules.md"), "## build\nPURPOSE x\nPATHS src/a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## workshop-host\nPURPOSE x\nPATHS src/c\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## client\nPURPOSE x\nPATHS src/b\nDEPENDS build\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    verify(p, "hello-api-fix2", true);
    assert!(!orch::ready_units(p).unwrap().contains(&"dependent".into()));
    let out = supersede(p);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(orch::ready_units(p).unwrap().contains(&"dependent".into()));
}

#[test]
fn refused_cross_track_supersedes_do_not_write() {
    for reason in ["unverified", "busy", "scope", "cycle"] {
        let dir = fixture();
        let p = dir.path();
        if reason != "unverified" {
            orch::set_unit_state(p, "hello-api-fix2", "verified").unwrap();
        }
        match reason {
            "busy" => fs::write(p.join("claims.tsv"), "unit\tattempt\tscope\tstarted\nhello-api-fix\tattempt\tsrc/a\t2026-01-01T00:00:00Z\n").unwrap(),
            "scope" => fs::write(p.join("new.md"), "GOAL g\nSCOPE src/other\nVERIFY echo ok\n").unwrap(),
            "cycle" => orch::set_unit_state(p, "hello-api-fix2", "superseded:hello-api-fix").unwrap(),
            _ => (),
        }
        let before = snapshot(p);
        let out = supersede(p);
        assert_ne!(out.exit_code, 0, "{reason}");
        let expected = match reason {
            "unverified" => "belongs to workshop-host, not build",
            "busy" => "running claim",
            "scope" => "does not cover src/a",
            _ => "cycle",
        };
        assert!(out.stderr.contains(expected), "{reason}: {}", out.stderr);
        assert_eq!(snapshot(p), before, "{reason}");
    }
}
