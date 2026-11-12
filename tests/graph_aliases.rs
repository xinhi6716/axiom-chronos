use axiom::graph::aliases::{load, Resolved};
use std::fs;

fn write(store: &std::path::Path, contents: &str) {
    fs::write(store.join("aliases.csv"), contents).unwrap();
}

#[test]
fn missing_file_is_empty_and_unknown_is_unchanged() {
    let store = tempfile::tempdir().unwrap();
    let aliases = load(store.path()).unwrap();
    assert_eq!(aliases.len(), 0);
    assert_eq!(
        aliases.resolve("  Unknown  "),
        Resolved {
            canonical: "  Unknown  ".into(),
            via: None,
        }
    );
    assert!(!store.path().join("aliases.csv").exists());
}

#[test]
fn resolves_simple_chained_and_case_insensitive_names() {
    let store = tempfile::tempdir().unwrap();
    let contents =
        "\n canonical , alias \nFinal Name, Middle\n middle , First \nFINAL NAME, MIDDLE\n";
    write(store.path(), contents);
    let aliases = load(store.path()).unwrap();
    assert_eq!(aliases.len(), 2);
    assert_eq!(
        aliases.resolve("mIdDlE"),
        Resolved {
            canonical: "Final Name".into(),
            via: Some("mIdDlE".into()),
        }
    );
    assert_eq!(
        aliases.resolve("  FIRST  "),
        Resolved {
            canonical: "Final Name".into(),
            via: Some("  FIRST  ".into()),
        }
    );
    assert_eq!(
        aliases.resolve("Other"),
        Resolved {
            canonical: "Other".into(),
            via: None
        }
    );
    assert_eq!(
        fs::read_to_string(store.path().join("aliases.csv")).unwrap(),
        contents
    );
}

#[test]
fn invalid_inputs_report_kind_and_physical_line() {
    let store = tempfile::tempdir().unwrap();
    for (contents, line) in [
        ("canonical,alias\nB,A\n\nA,b\n", 4),
        ("canonical,alias\nA,A\n", 2),
        ("canonical,alias\nOne,name\nTwo, NAME\n", 3),
        ("canonical,alias\n\nonly_one\n", 3),
        ("canonical,alias\nA,b,extra\n", 2),
        ("canonical,alias\nA,  \n", 2),
        ("\nwrong,header\n", 2),
        ("\n", 1),
    ] {
        write(store.path(), contents);
        let error = load(store.path()).unwrap_err();
        assert_eq!(error.kind, "invalid_aliases", "{contents:?}");
        assert!(error.message.contains(&format!("line {line}:")), "{error}");
    }
}

#[test]
fn stores_are_isolated() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write(first.path(), "canonical,alias\nFirst,shared\n");
    write(second.path(), "canonical,alias\nSecond,shared\n");
    let a = load(first.path()).unwrap();
    let b = load(second.path()).unwrap();
    assert_eq!(a.resolve("shared").canonical, "First");
    assert_eq!(b.resolve("shared").canonical, "Second");
    assert_eq!(a.resolve("shared").canonical, "First");
}

#[test]
fn chains_stop_after_eight_steps_but_cycles_are_checked_beyond_that() {
    let store = tempfile::tempdir().unwrap();
    let mut contents = String::from("canonical,alias\n");
    for n in 0..10 {
        contents.push_str(&format!("node{},node{}\n", n + 1, n));
    }
    write(store.path(), &contents);
    let aliases = load(store.path()).unwrap();
    assert_eq!(aliases.len(), 10);
    assert_eq!(
        aliases.resolve("node0"),
        Resolved {
            canonical: "node8".into(),
            via: Some("node0".into()),
        }
    );
    contents.push_str("node0,node10\n");
    write(store.path(), &contents);
    let error = load(store.path()).unwrap_err();
    assert_eq!(error.kind, "invalid_aliases");
    assert!(error.message.contains("line 12:"), "{error}");
}
