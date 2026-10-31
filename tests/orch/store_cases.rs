use axiom::orch::{run, Output};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
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

#[test]
fn unusable_os_mutex_is_io_not_transient_busy() {
    let t = Temp::new();
    std::fs::create_dir(t.0.join(".orch.mutex")).unwrap();
    let error = axiom::orch::with_lock(&t.0, false, || Ok(())).unwrap_err();
    assert_eq!(error.kind, "io");
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
