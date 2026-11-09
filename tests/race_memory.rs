#[path = "common/race.rs"]
mod race;

use axiom::memory::{Memory, Record};
use race::{child, children};
use std::fs;

#[test]
fn digest_lines_from_many_processes_stay_whole() {
    const WRITERS: usize = 8;
    const LINES: usize = 40;
    if let Some((dir, i)) = child() {
        let memory = Memory::open(&dir).unwrap();
        for n in 0..LINES {
            memory
                .digest("demo", Some(&format!("writer-{i}-{n}")))
                .unwrap();
        }
        return;
    }
    let t = tempfile::tempdir().unwrap();
    Memory::open(t.path()).unwrap();
    children(
        "digest_lines_from_many_processes_stay_whole",
        t.path(),
        WRITERS,
    );
    let text = fs::read_to_string(t.path().join("memory/demo.md")).unwrap();
    let whole = regex::Regex::new(r"^- \d{4}-\d{2}-\d{2} writer-\d+-\d+$").unwrap();
    for line in text.lines() {
        assert!(whole.is_match(line), "torn line: {line:?}");
    }
    assert_eq!(text.lines().count(), WRITERS * LINES);
}

#[test]
fn memory_created_and_recorded_by_many_processes_keeps_every_entry() {
    const WRITERS: usize = 8;
    if let Some((dir, i)) = child() {
        let memory = Memory::open(&dir).unwrap();
        let recorded = memory
            .record(&Record {
                project: "demo".into(),
                project_dir: dir.join("repo").to_string_lossy().into_owned(),
                unit: format!("u{i}"),
                title: format!("title {i}"),
                summary: "summary".into(),
                body: "body".into(),
                symbols: vec![],
                files: vec![format!("f{i}.txt")],
                wish: "wish".into(),
            })
            .unwrap();
        assert_eq!(recorded["duplicate"], false);
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join("orchestrate/demo");
    let repo = t.path().join("repo");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&repo).unwrap();
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for i in 0..WRITERS {
        fs::write(repo.join(format!("f{i}.txt")), "content").unwrap();
        let brief = store.join(format!("u{i}.brief"));
        fs::write(&brief, format!("GOAL g\nSCOPE f{i}.txt\nVERIFY echo ok\n")).unwrap();
        units.push_str(&format!("u{i}\tm\tverified\t\t\t\t{}\n", brief.display()));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    children(
        "memory_created_and_recorded_by_many_processes_keeps_every_entry",
        t.path(),
        WRITERS,
    );
    let found = Memory::open(t.path())
        .unwrap()
        .search("title", Some("demo"), 100)
        .unwrap();
    assert_eq!(found["entries"].as_array().unwrap().len(), WRITERS);
}
