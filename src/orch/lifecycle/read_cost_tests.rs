//! The scheduling read paths read each store file once per call, however many units the
//! store holds and however long their evidence is.
use crate::orch::evidence::read_count;
use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

const MODULES: usize = 20;
const PER_MODULE: usize = 8;
const UNITS: usize = MODULES * PER_MODULE;
const CYCLES: usize = 60;

fn id(module: usize, unit: usize) -> String {
    format!("m{module:02}-{unit}")
}

fn rows(text: &mut String, kind: &str, command: &str, exit_code: i32, commit: &str) {
    writeln!(
        text,
        "{kind}\t{command}\t{exit_code}\t{commit}\t2026-11-14T00:00:00Z"
    )
    .unwrap();
}

/// Rejected rounds that come before the row that decides a unit's state.
fn history(text: &mut String) {
    for n in 0..CYCLES {
        let commit = format!("old{n}");
        rows(text, "run", "gpt-6.1-terra", 0, "");
        rows(text, "verify", "cargo test", 0, &commit);
        rows(text, "accept", "cargo test", 0, &commit);
        rows(text, "review", "chronos:reviewer", 1, &commit);
        rows(text, "note", "rejected", 0, &commit);
    }
}

/// Modules m00..m09 are verified; m10 has built, pending and failed units, so m11..m19 wait.
fn generate(store: &Path) {
    std::fs::create_dir_all(store.join("reports")).unwrap();
    std::fs::create_dir_all(store.join("briefs")).unwrap();
    let filler = "the module keeps its public names and signatures; ".repeat(40);
    let mut map = String::new();
    for m in 0..MODULES {
        let depends = match m {
            0 => "none".to_string(),
            1 => "m00".to_string(),
            _ => format!("m{:02} m00", m - 1),
        };
        writeln!(
            map,
            "## m{m:02}\nPURPOSE module {m}\nPATHS src/m{m:02}/\nDEPENDS {depends}\nCONTRACT {filler}\nACCEPT test, `echo ok`\n"
        )
        .unwrap();
    }
    std::fs::write(store.join("modules.md"), map).unwrap();
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for m in 0..MODULES {
        for u in 0..PER_MODULE {
            let unit = id(m, u);
            let mut text = String::from("kind\tcommand\texit_code\tcommit\tts\n");
            let mut state = "pending".to_string();
            let mut brief = String::new();
            match u {
                0 => {
                    history(&mut text);
                    state = format!("superseded:{}", id(m, 1));
                }
                _ if m < 10 => {
                    history(&mut text);
                    rows(&mut text, "run", "gpt-6.1-terra", 0, "");
                    rows(&mut text, "verify", "cargo test", 0, "new");
                    rows(&mut text, "accept", "cargo test", 0, "new");
                    rows(&mut text, "review", "chronos:reviewer", 0, "new");
                    state = "verified".into();
                }
                1..=5 => {
                    history(&mut text);
                    rows(&mut text, "run", "gpt-6.1-terra", 0, "");
                    rows(&mut text, "verify", "cargo test", 0, "new");
                }
                6 | 7 => {
                    for _ in 0..CYCLES * 5 {
                        rows(&mut text, "note", "waiting", 0, "");
                    }
                    if u == 7 {
                        rows(&mut text, "run", "gpt-6.1-terra", 1, "");
                    }
                    // Unit 7 shares unit 6's scope, so only unit 6 can be ready.
                    let path = store.join("briefs").join(format!("{unit}.md"));
                    std::fs::write(
                        &path,
                        format!("GOAL test\nSCOPE src/m{m:02}/a.rs\nVERIFY echo ok\n"),
                    )
                    .unwrap();
                    brief = path.display().to_string();
                }
                _ => unreachable!(),
            }
            std::fs::write(
                store.join("reports").join(format!("{unit}.evidence.tsv")),
                text,
            )
            .unwrap();
            writeln!(units, "{unit}\tm{m:02}\t{state}\t\t\t\t{brief}").unwrap();
        }
    }
    std::fs::write(store.join("units.tsv"), units).unwrap();
}

fn assert_reads(call: &str, max_maps: usize) {
    let (evidence, maps) = read_count::take();
    assert!(
        evidence <= UNITS,
        "{call} read {evidence} evidence files for {UNITS} units"
    );
    assert!(
        maps <= max_maps,
        "{call} parsed modules.md {maps} times (at most {max_maps})"
    );
}

#[test]
fn read_paths_read_each_file_once_per_call() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    generate(&store);
    let started = Instant::now();
    read_count::take();

    let ready = super::ready_units(&store).unwrap();
    assert_reads("ready_units", 1);
    assert_eq!(ready, vec![id(10, 6)]);

    let waiting = super::waiting_units(&store).unwrap();
    assert_reads("waiting_units", 1);
    let next = |unit: &str| {
        waiting
            .iter()
            .find(|w| w.unit == unit)
            .map(|w| w.next.clone())
    };
    assert_eq!(next(&id(10, 3)).as_deref(), Some("accept"));
    assert_eq!(next(&id(10, 7)).as_deref(), Some("busy"));
    assert_eq!(next(&id(5, 3)), None);
    let wait_m10 = format!(
        "wait {}, m10-6 (pending; next run), m10-7 (failed; next run)",
        (1..=5)
            .map(|u| format!("{} (built; next accept)", id(10, u)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert_eq!(next(&id(11, 6)), Some(wait_m10.clone()));
    assert_eq!(next(&id(11, 7)), Some(wait_m10));
    assert!(next(&id(19, 6))
        .unwrap()
        .starts_with("wait m18-1 (built; next accept)"));
    assert_eq!(waiting.len(), 5 + 1 + 9 * 7);

    for (unit, expected) in [
        (id(10, 6), "ready"),
        (id(5, 3), "verified"),
        (id(15, 2), "accept"),
    ] {
        assert_eq!(super::next_step_for(&store, &unit).unwrap(), expected);
        assert_reads("next_step_for", 1);
    }

    let context = crate::orch::module_context(&store, "m12", &repo).unwrap();
    assert_reads("module_context", 2);
    let units: Vec<_> = context
        .units
        .iter()
        .map(|u| (u.unit.as_str(), u.state.as_str(), u.next.as_str()))
        .collect();
    assert_eq!(units.len(), PER_MODULE);
    assert_eq!(units[0], ("m12-0", "superseded:m12-1", "verified"));
    assert_eq!(units[2], ("m12-2", "built", "accept"));
    assert_eq!(units[7].1, "failed");
    assert!(units[7].2.starts_with("wait m11-1 (built; next accept)"));

    // A hang guard, far above the time these calls take once each file is read once.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "read paths took {:?}",
        started.elapsed()
    );
}
