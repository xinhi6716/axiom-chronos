use axiom::{
    engine::kinds,
    graph::{
        feedback::{collect, ingest},
        keys, GraphStore,
    },
    orch::events::{append_event, Event},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const W1: &str = "w-1700000000001-abcdef";
const W2: &str = "w-1700000000002-abcdef";

fn store(home: &Path, wish: &str) -> PathBuf {
    let path = home.join("orchestrate/demo").join(wish);
    fs::create_dir_all(&path).unwrap();
    path
}
fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn append(store: &Path, kind: &str, data: Value) -> Event {
    append_event(store, kind, None, None, data).unwrap()
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn maps_terminal_events_and_validates_fields() {
    let t = tempfile::tempdir().unwrap();
    let s = store(t.path(), W1);
    let rows = [
        append(
            &s,
            kinds::INTERJECTION_APPLIED,
            json!({"id":"a", "reason":"why", "decider":"user", "prompt":"secret"}),
        ),
        append(
            &s,
            kinds::INTERJECTION_DECLINED,
            json!({"id":"b", "reason":"decline reason", "decider":"invalid"}),
        ),
        append(
            &s,
            kinds::ADJUDICATION_OVERTURNED,
            json!({"id":"c", "reason":"overturned reason", "decider":"core"}),
        ),
        append(&s, kinds::INTERJECTION_DECLINED, json!({"id":"d"})),
        append(&s, kinds::ADJUDICATION_OVERTURNED, json!({"id":"e"})),
    ];
    append(&s, kinds::INTERJECTION_APPLIED, json!({}));
    append(&s, kinds::INTERJECTION_DECLINED, json!({"id":42}));
    let feedback = collect(t.path(), "demo").unwrap();
    assert_eq!(feedback.len(), rows.len());
    let expected = [
        ("user", "why", "interjection a applied"),
        ("core", "decline reason", "decline reason"),
        ("core", "overturned reason", "overturned reason"),
        ("core", "", "interjection d declined"),
        ("user", "", ""),
    ];
    for (row, (decider, reason, text)) in rows.iter().zip(expected) {
        let id = row.data["id"].as_str().unwrap();
        let source = format!("orchestrate/demo/{W1}/events.jsonl#{id}");
        let f = feedback.iter().find(|f| f.source == source).unwrap();
        assert_eq!((&*f.kind, &*f.decider, &*f.wish), (&*row.kind, decider, W1));
        assert_eq!((&*f.reason, &*f.text), (reason, text));
        assert_eq!(f.target, keys::wish(W1));
        assert_eq!(f.source_sha256, sha(&serde_json::to_vec(row).unwrap()));
        assert_eq!(f.supersedes, None);
    }
}

#[test]
fn pending_and_other_events_are_not_feedback() {
    let t = tempfile::tempdir().unwrap();
    let s = store(t.path(), W1);
    for kind in [
        kinds::INTERJECTION_SENT,
        kinds::INTERJECTION_RECEIVED,
        kinds::INTERJECTION_CLASSIFIED,
        kinds::ADJUDICATION,
    ] {
        append(&s, kind, json!({"id":"pending"}));
    }
    assert!(collect(t.path(), "demo").unwrap().is_empty());
}

#[test]
fn deduplicates_amendments_and_waivers_only_in_same_wish_and_project() {
    let t = tempfile::tempdir().unwrap();
    let s = store(t.path(), W1);
    let other = store(t.path(), W2);
    put(&t.path().join("goals/a.json"), &json!({"wish":"x", "wish_id":W1, "project":"demo", "status":"active", "amendments":["prompt"]}).to_string());
    put(&t.path().join("goals/b.json"), &json!({"wish":"x", "wish_id":W1, "project":"other", "status":"active", "amendments":["other project"]}).to_string());
    put(
        &s.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tbuild\tfailed\t\t\t\t\n",
    );
    put(&s.join("reports/u1.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nrun\tbuild\t1\ttree\tt\nverify\ttest\t1\ttree\tt\nreview\treviewer\t1\ttree\tt\nwaive\t1 Accepted risk\t0\ttree\tt\n");
    put(&s.join("reports/u1.verdict.json"), &json!({"unit":"u1","tree":"tree","verdict":"fail","findings":[{"check":"security","severity":"major","location":"a","evidence":"Finding text","fix":"fix"}]}).to_string());
    for (st, id, prompt, kind) in [
        (&s, "same-applied", "prompt", kinds::INTERJECTION_APPLIED),
        (&s, "same-declined", "prompt", kinds::INTERJECTION_DECLINED),
        (
            &s,
            "other-project",
            "other project",
            kinds::INTERJECTION_APPLIED,
        ),
        (&other, "other-wish", "prompt", kinds::INTERJECTION_APPLIED),
    ] {
        append(
            st,
            kinds::INTERJECTION_RECEIVED,
            json!({"id":id, "digest":sha(prompt.as_bytes())}),
        );
        append(st, kind, json!({"id":id}));
    }
    append(
        &s,
        kinds::ADJUDICATION_OVERTURNED,
        json!({"id":"waived", "reason":"Accepted risk"}),
    );
    append(
        &other,
        kinds::ADJUDICATION_OVERTURNED,
        json!({"id":"other-waiver", "reason":"Accepted risk"}),
    );
    let feedback = collect(t.path(), "demo").unwrap();
    let sources: Vec<_> = feedback.iter().map(|f| f.source.as_str()).collect();
    assert!(feedback.iter().any(|f| f.kind == "correction"));
    assert!(feedback.iter().any(|f| f.kind == "waived_finding"));
    for id in ["same-applied", "same-declined", "waived"] {
        assert!(
            !sources.iter().any(|s| s.ends_with(&format!("#{id}"))),
            "{id}"
        );
    }
    for id in ["other-project", "other-wish", "other-waiver"] {
        assert!(
            sources.iter().any(|s| s.ends_with(&format!("#{id}"))),
            "{id}"
        );
    }
}

#[test]
fn wishes_are_isolated_and_ingestion_is_revision_stable() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let project = t.path().join("project");
    fs::create_dir_all(&project).unwrap();
    for wish in [W1, W2] {
        append(
            &store(&home, wish),
            kinds::INTERJECTION_APPLIED,
            json!({"id":"same"}),
        );
    }
    let first = collect(&home, "demo").unwrap();
    assert_eq!(first, collect(&home, "demo").unwrap());
    assert_eq!(first.len(), 2);
    for wish in [W1, W2] {
        assert!(first.iter().any(|f| f.wish == wish
            && f.source == format!("orchestrate/demo/{wish}/events.jsonl#same")
            && f.target == keys::wish(wish)));
    }
    let graph = GraphStore::open(&home).unwrap();
    let initial = ingest(&graph, &home, "demo", &project).unwrap();
    let replay = ingest(&graph, &home, "demo", &project).unwrap();
    assert_eq!(initial.nodes_written, 2);
    assert_eq!(
        (replay.nodes_written, replay.edges_written, replay.unchanged),
        (0, 0, 2)
    );
    assert_eq!(initial.revision, replay.revision);
}

#[test]
fn only_last_2000_terminal_events_are_projected() {
    let t = tempfile::tempdir().unwrap();
    let s = store(t.path(), W1);
    let mut template = append(&s, kinds::INTERJECTION_APPLIED, json!({"id":"0"}));
    let mut lines = String::new();
    for i in 0..2005 {
        template.seq = i + 1;
        template.data = json!({"id":i.to_string()});
        lines.push_str(&serde_json::to_string(&template).unwrap());
        lines.push('\n');
    }
    template.kind = kinds::INTERJECTION_SENT.into();
    for i in 2005..4010 {
        template.seq = i + 1;
        template.data = json!({"id":i.to_string()});
        lines.push_str(&serde_json::to_string(&template).unwrap());
        lines.push('\n');
    }
    put(&s.join("events.jsonl"), &lines);
    let feedback = collect(t.path(), "demo").unwrap();
    assert_eq!(feedback.len(), 2000);
    assert!(!feedback.iter().any(|f| f.source.ends_with("#4")));
    assert!(feedback.iter().any(|f| f.source.ends_with("#5")));
    assert!(feedback.iter().any(|f| f.source.ends_with("#2004")));
}

#[test]
fn old_receipt_still_deduplicates_after_many_pending_events() {
    let t = tempfile::tempdir().unwrap();
    let s = store(t.path(), W1);
    put(&t.path().join("goals/a.json"), &json!({"wish":"x", "wish_id":W1, "project":"demo", "status":"active", "amendments":["prompt"]}).to_string());
    let mut event = append(
        &s,
        kinds::INTERJECTION_RECEIVED,
        json!({"id":"old", "digest":sha(b"prompt")}),
    );
    let mut lines = format!("{}\n", serde_json::to_string(&event).unwrap());
    event.kind = kinds::INTERJECTION_SENT.into();
    for i in 0..2005 {
        event.seq = i + 2;
        event.data = json!({"id":format!("pending-{i}")});
        lines.push_str(&serde_json::to_string(&event).unwrap());
        lines.push('\n');
    }
    event.seq += 1;
    event.kind = kinds::INTERJECTION_APPLIED.into();
    event.data = json!({"id":"old"});
    lines.push_str(&serde_json::to_string(&event).unwrap());
    lines.push('\n');
    put(&s.join("events.jsonl"), &lines);
    let feedback = collect(t.path(), "demo").unwrap();
    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].kind, "correction");
}
