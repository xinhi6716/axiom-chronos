use axiom::usage::review_stats::{collect, command, render_text, to_json};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn store(home: &Path, wish: &str) -> PathBuf {
    let path = home.join("orchestrate").join("project").join(wish);
    fs::create_dir_all(&path).unwrap();
    path
}
fn event(seq: u64, kind: &str, data: Value) -> Value {
    json!({"seq":seq,"ts":"2026-01-01T00:00:00Z","ts_ms":1767225600000i64 + seq as i64 * 1000,
        "kind":kind,"wish":"ignored","module":null,"unit":null,"data":data})
}
fn review(
    seq: u64,
    id: &str,
    role: &str,
    reviewer: &str,
    model: &str,
    unit: &str,
    paths: &[&str],
    pass: bool,
    findings: usize,
) -> Value {
    event(
        seq,
        "review_recorded",
        json!({"review_id":id,"unit":unit,"role":role,"reviewer":reviewer,"model":model,"paths":paths,"pass":pass,"findings":vec![json!({"number":1}); findings]}),
    )
}
fn verify(
    seq: u64,
    id: &str,
    finding: u64,
    path: &str,
    severity: &str,
    status: &str,
    failed: bool,
) -> Value {
    event(
        seq,
        "finding_verified",
        json!({"review_id":id,"finding":finding,"location":path,"severity":severity,"status":status,"failed":failed,"reviewer":"orphan"}),
    )
}
fn write_events(store: &Path, events: &[Value]) {
    let text = events.iter().map(|e| format!("{e}\n")).collect::<String>();
    fs::write(store.join("events.jsonl"), text).unwrap();
}
fn fix(store: &Path, scope: &str, timestamp: &str) {
    let brief = store.join("fix.md");
    fs::write(&brief, format!("GOAL fix\nSCOPE {scope}\nVERIFY test\n")).unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nfix\tcore\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(store.join("reports/fix.evidence.tsv"), format!("kind\tcommand\texit_code\tcommit\tts\nfixes\toriginal\t0\t\t{timestamp}\nfixes\toriginal\t0\t\t{timestamp}\n")).unwrap();
}

#[test]
fn aggregation_dedup_isolation_and_rendering() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    let b = store(home.path(), "w-1234567890124-abcdef");
    let events = vec![
        review(
            1,
            "r1",
            "reviewer",
            "alice",
            "m1",
            "original",
            &["src/"],
            true,
            2,
        ),
        review(
            2,
            "r2",
            "pm-qa",
            "bob",
            "",
            "other",
            &["src/lib.rs"],
            true,
            0,
        ),
        verify(3, "r3", 1, "src\\lib.rs:12-15", "MAJOR", "confirmed", true),
        review(
            4,
            "r3",
            "reviewer",
            "alice",
            "m1",
            "third",
            &["none/"],
            false,
            1,
        ),
        verify(
            5,
            "r3",
            2,
            "no/coverage.rs:1",
            "blocker",
            "confirmed",
            false,
        ),
        verify(
            6,
            "r3",
            3,
            "later/file.rs:9",
            "major",
            "inconclusive",
            false,
        ),
        review(
            7,
            "later",
            "astra-max",
            "zoe",
            "z",
            "later",
            &["later/"],
            true,
            0,
        ),
        verify(8, "r3", 4, "outside", "minor", "rejected", false),
        verify(9, "r3", 5, "outside", "minor", "rejected", true),
        verify(10, "r3", 6, "outside", "minor", "confirmed", false),
        verify(11, "r3", 6, "outside", "minor", "rejected", false),
        verify(12, "missing", 1, "outside", "minor", "confirmed", false),
    ];
    write_events(&a, &events);
    fix(&a, "src/lib.rs", "2026-01-01T00:00:20.000Z");
    write_events(
        &b,
        &[verify(
            20,
            "elsewhere",
            1,
            "src/lib.rs:1",
            "major",
            "confirmed",
            false,
        )],
    );
    let rows = collect(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (&*r.role, &*r.reviewer, &*r.model))
            .collect::<Vec<_>>(),
        vec![
            ("-", "orphan", ""),
            ("astra-max", "zoe", "z"),
            ("pm-qa", "bob", ""),
            ("reviewer", "alice", "m1")
        ]
    );
    let alice = &rows[3];
    assert_eq!(
        (
            alice.reviews,
            alice.findings,
            alice.finding_rate.num,
            alice.finding_rate.den
        ),
        (2, 3, 3, 2)
    );
    assert_eq!(
        (
            alice.rejected,
            alice.rejection_rate.num,
            alice.rejection_rate.den
        ),
        (2, 2, 4)
    );
    assert_eq!(
        (alice.escapes, alice.escape_rate.num, alice.escape_rate.den),
        (0, 0, 1)
    );
    let bob = &rows[2];
    assert_eq!(
        (bob.escapes, bob.escape_rate.num, bob.escape_rate.den),
        (2, 2, 1)
    );
    assert_eq!((rows[1].escapes, rows[0].escapes), (0, 0));
    assert_eq!(
        (
            rows[0].rejection_rate.num,
            rows[0].rejection_rate.den,
            rows[0].escape_rate.rate
        ),
        (0, 2, None)
    );
    let text = render_text(&rows);
    assert!(!text.to_lowercase().contains("unknown"));
    let json = to_json(&rows);
    for (row, line) in json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .zip(text.lines().skip(1))
    {
        for field in ["finding_rate", "rejection_rate", "escape_rate"] {
            let rate = &row[field];
            assert!(line.contains(&format!("{}/{}", rate["num"], rate["den"])));
        }
    }
    let orphan_line = text.lines().find(|l| l.contains("orphan")).unwrap();
    assert!(!orphan_line.contains("0/0 ("));
    let mut repeated = events.clone();
    repeated.extend(events);
    write_events(&a, &repeated);
    assert_eq!(rows, collect(&[a.clone(), b, a]).unwrap());
    assert_eq!(command(home.path(), &["--json".into()]), 0);
    assert_eq!(
        command(
            home.path(),
            &["--wish".into(), "w-1234567890123-abcdef".into()]
        ),
        2
    );
}

#[test]
fn negative_controls_and_empty_output() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    write_events(
        &a,
        &[
            review(
                1,
                "first",
                "reviewer",
                "a",
                "m",
                "one",
                &["other/"],
                true,
                0,
            ),
            verify(
                2,
                "source",
                1,
                "uncovered/file:1",
                "major",
                "confirmed",
                false,
            ),
            verify(
                3,
                "source",
                2,
                "later/file:1",
                "blocker",
                "inconclusive",
                false,
            ),
            review(4, "last", "reviewer", "b", "m", "two", &["later/"], true, 0),
        ],
    );
    let rows = collect(&[a]).unwrap();
    assert!(rows.iter().all(|r| r.escapes == 0));
    assert!(render_text(&[]).to_lowercase().contains("no review events"));
}

#[test]
fn fix_time_and_self_review_and_same_review_finding_do_not_count() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    write_events(
        &a,
        &[
            review(
                1,
                "self",
                "reviewer",
                "self",
                "m",
                "fix",
                &["src/"],
                true,
                0,
            ),
            verify(2, "self", 1, "src/file.rs:3", "major", "confirmed", false),
            review(
                3,
                "too-late",
                "reviewer",
                "late",
                "",
                "other",
                &["src/"],
                true,
                0,
            ),
        ],
    );
    fix(&a, "src/file.rs", "2026-01-01T00:00:02.000Z");
    let rows = collect(&[a]).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.escapes == 0));
    assert_eq!(
        command(
            home.path(),
            &[
                "--project".into(),
                "project".into(),
                "--wish".into(),
                "w-1234567890123-abcdef".into(),
                "--json".into()
            ]
        ),
        0
    );
    assert_eq!(command(home.path(), &["--project".into()]), 2);
    assert_eq!(command(home.path(), &["--bad".into()]), 2);
}
