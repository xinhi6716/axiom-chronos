use axiom::graph::constraints::{load, prepend, DEFAULT_BUDGET};
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn absent_and_empty_sources() {
    let project = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let empty = load(project.path(), Some(store.path()), DEFAULT_BUDGET).unwrap();
    assert_eq!(empty.text, "");
    assert_eq!(
        (empty.bytes, empty.included, empty.omitted, empty.overflow),
        (0, 0, 0, false)
    );
    assert!(empty.sources.is_empty());
    assert_eq!(prepend("brief\nunchanged", &empty), "brief\nunchanged");
    fs::write(project.path().join("CONSTRAINTS.md"), "").unwrap();
    assert_eq!(
        load(project.path(), Some(store.path()), DEFAULT_BUDGET).unwrap(),
        empty
    );
}

#[test]
fn ordering_dedupe_sources_and_prepend_are_deterministic() {
    let project = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let p = project.path().join("CONSTRAINTS.md");
    let s = store.path().join("constraints.md");
    let project_data = "- first\n  continuation\n- duplicate\n\nParagraph line one\nline two\n";
    let store_data = "- duplicate\n- last\n";
    fs::write(&p, project_data).unwrap();
    fs::write(&s, store_data).unwrap();
    let block = load(project.path(), Some(store.path()), DEFAULT_BUDGET).unwrap();
    assert_eq!(
        block,
        load(project.path(), Some(store.path()), DEFAULT_BUDGET).unwrap()
    );
    assert_eq!(
        block.text,
        "CONSTRAINTS\n- first\n  continuation\n- duplicate\nParagraph line one\nline two\n- last\n"
    );
    assert_eq!(
        (block.included, block.omitted, block.overflow),
        (4, 0, false)
    );
    assert_eq!(block.bytes, block.text.len());
    assert_eq!(
        block.sources,
        vec![
            (
                p.to_string_lossy().into_owned(),
                format!("{:x}", Sha256::digest(project_data.as_bytes()))
            ),
            (
                s.to_string_lossy().into_owned(),
                format!("{:x}", Sha256::digest(store_data.as_bytes()))
            ),
        ]
    );
    assert_eq!(
        prepend("brief\n", &block),
        format!("{}\nbrief\n", block.text)
    );
    assert_eq!(fs::read_to_string(p).unwrap(), project_data);
    assert_eq!(fs::read_to_string(s).unwrap(), store_data);
}

#[test]
fn overflow_names_omitted_files_without_splitting_entries_or_utf8() {
    let project = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("CONSTRAINTS.md"),
        "- 短\n- 長い制約を繰り返す長い制約を繰り返す\n",
    )
    .unwrap();
    fs::write(
        store.path().join("constraints.md"),
        "- another long constraint that will not fit within this budget\n",
    )
    .unwrap();
    let expected = "CONSTRAINTS\n- 短\nOMITTED 2 constraints; see CONSTRAINTS.md, constraints.md\n";
    let budget = expected.len();
    let block = load(project.path(), Some(store.path()), budget).unwrap();
    assert_eq!(block.text, expected);
    assert_eq!(block.bytes, budget);
    assert_eq!(
        (block.included, block.omitted, block.overflow),
        (1, 2, true)
    );
    assert!(block.text.len() <= budget);
    assert_eq!(
        load(project.path(), Some(store.path()), budget).unwrap(),
        block
    );
    for limit in 0..budget {
        let constrained = load(project.path(), Some(store.path()), limit).unwrap();
        assert!(constrained.text.len() <= limit);
        assert!(constrained.overflow);
        assert_eq!(constrained.included + constrained.omitted, 3);
        assert!(!constrained.text.contains("長い制約"));
    }
}

#[test]
fn projects_are_isolated_and_invalid_utf8_is_an_error() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("CONSTRAINTS.md"), "- first project\n").unwrap();
    fs::write(second.path().join("CONSTRAINTS.md"), "- second project\n").unwrap();
    assert!(!load(first.path(), None, DEFAULT_BUDGET)
        .unwrap()
        .text
        .contains("second"));
    assert!(!load(second.path(), None, DEFAULT_BUDGET)
        .unwrap()
        .text
        .contains("first"));
    fs::write(second.path().join("CONSTRAINTS.md"), [0xff]).unwrap();
    assert!(load(second.path(), None, DEFAULT_BUDGET).is_err());
}
