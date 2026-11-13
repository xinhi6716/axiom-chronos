use axiom::{
    paper::{
        csv,
        evidence::{self, BundleInputs},
        state, *,
    },
    research_program::{self, dossier, Stage},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use tempfile::{tempdir, TempDir};

const DOSSIER: &str = include_str!("fixtures/paper/evidence/dossier.json");
const RESULTS: &str = include_str!("fixtures/paper/evidence/results.json");
const DATA: &[u8] = include_bytes!("fixtures/paper/evidence/data.csv");
const MODEL: &[u8] = include_bytes!("fixtures/paper/evidence/model.txt");

struct Case {
    dir: TempDir,
    inputs: BundleInputs,
}
impl Case {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let store = root.join("store");
        fs::create_dir_all(research_program::program_dir(&store)).unwrap();
        let dossier: dossier::Dossier = serde_json::from_str(DOSSIER).unwrap();
        dossier::write(
            &research_program::program_dir(&store).join("02-dossier.json"),
            &dossier,
        )
        .unwrap();
        fs::write(root.join("results.json"), RESULTS).unwrap();
        fs::write(root.join("data.csv"), DATA).unwrap();
        fs::write(root.join("model.txt"), MODEL).unwrap();
        let inputs = BundleInputs {
            store,
            project_root: root.clone(),
            results: vec![root.join("results.json")],
            datasets: vec![root.join("data.csv")],
            models: vec![root.join("model.txt")],
        };
        Self { dir, inputs }
    }
    fn build(&self) -> EvidenceBundle {
        evidence::build(&self.inputs).unwrap()
    }
}
fn kind<T: std::fmt::Debug>(result: axiom::base::Res<T>, expected: &str) {
    assert_eq!(result.unwrap_err().kind, expected);
}
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn csv_rfc4180_and_units() {
    let table = csv::parse("\u{feff}name;note;time [s]\r\n\"A\";\"x;y\";1\r\nB;\"two\r\nlines with \"\"quotes\"\"\";2\r\n", b';').unwrap();
    assert_eq!(table.headers, ["name", "note", "time [s]"]);
    assert_eq!(
        table.rows,
        vec![
            vec!["A", "x;y", "1"],
            vec!["B", "two\r\nlines with \"quotes\"", "2"]
        ]
    );
    let ragged = csv::parse("a,b\n1,2\n3\n", b',').unwrap_err();
    assert_eq!(ragged.kind, "csv");
    assert!(ragged.message.contains('3'), "{}", ragged.message);
    assert_eq!(
        csv::split_unit("time [s]"),
        ("time".into(), Some("s".into()))
    );
    assert_eq!(csv::split_unit("n"), ("n".into(), None));
}

#[test]
fn builds_claims_evidence_references_quantities_and_hashes() {
    let c = Case::new();
    let b = c.build();
    let dossier: dossier::Dossier = serde_json::from_str(DOSSIER).unwrap();
    assert_eq!(b.question, dossier.question);
    assert_eq!(b.claims.len(), 2);
    for item in &dossier.items {
        let claim = b.claims.iter().find(|x| x.id == item.id).unwrap();
        assert_eq!(claim.text, item.claim);
        assert_eq!(claim.confidence, item.confidence);
        assert_eq!(claim.evidence.len(), item.sources.len());
        for id in &claim.evidence {
            assert!(b.evidence.iter().any(|e| &e.id == id));
        }
    }
    assert_eq!(b.evidence.len(), 5);
    let sources: Vec<_> = b.evidence.iter().map(|e| e.source.as_str()).collect();
    for item in &dossier.items {
        for source in &item.sources {
            assert!(sources.contains(&source.source.as_str()));
        }
    }
    assert_eq!(sources.iter().filter(|s| **s == "10.1234/example").count(), 2);
    assert_eq!(
        b.evidence
            .iter()
            .map(|e| &e.id)
            .collect::<BTreeSet<_>>()
            .len(),
        5
    );
    assert_eq!(b.references.len(), 3);
    assert_eq!(
        b.references
            .iter()
            .map(|r| &r.key)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let doi = b
        .references
        .iter()
        .find(|r| r.doi.as_deref() == Some("10.1234/example"))
        .unwrap();
    assert_eq!(doi.evidence.len(), 2);
    assert!(doi.evidence.iter().all(|id| b
        .evidence
        .iter()
        .any(|e| &e.id == id && e.source == "10.1234/example")));
    assert!(b
        .references
        .iter()
        .any(|r| r.openalex.as_deref() == Some("W123456789")));
    assert!(b
        .references
        .iter()
        .any(|r| r.url.as_deref() == Some("https://example.org/study")));
    let results: ResultsFile = serde_json::from_str(RESULTS).unwrap();
    assert_eq!(b.quantities, results.quantities);
    assert_eq!(b.artifacts.len(), 4);
    for artifact in &b.artifacts {
        let bytes = fs::read(c.inputs.project_root.join(&artifact.path)).unwrap();
        assert_eq!(artifact.sha256, axiom::seams::sha256_hex(&bytes));
    }
    for path in [
        &c.inputs.results[0],
        &c.inputs.datasets[0],
        &c.inputs.models[0],
        &research_program::program_dir(&c.inputs.store).join("02-dossier.json"),
    ] {
        assert!(b
            .artifacts
            .iter()
            .any(|a| c.inputs.project_root.join(&a.path) == *path));
    }
    assert_eq!(b.hash, b.compute_hash());
}

#[test]
fn build_refuses_missing_dangling_and_escaping_inputs() {
    let c = Case::new();
    fs::remove_file(research_program::program_dir(&c.inputs.store).join("02-dossier.json"))
        .unwrap();
    kind(evidence::build(&c.inputs), "missing_input");
    let c = Case::new();
    fs::remove_file(&c.inputs.results[0]).unwrap();
    kind(evidence::build(&c.inputs), "missing_input");
    let mut c = Case::new();
    let mut result: serde_json::Value = serde_json::from_str(RESULTS).unwrap();
    result["quantities"][0]["claim_ids"][0] = "unknown".into();
    fs::write(&c.inputs.results[0], serde_json::to_vec(&result).unwrap()).unwrap();
    kind(evidence::build(&c.inputs), "dangling");
    result["quantities"][0]["claim_ids"][0] = "C1".into();
    result["quantities"][0]["locator"]["artifact"] = "other.csv".into();
    fs::write(&c.inputs.results[0], serde_json::to_vec(&result).unwrap()).unwrap();
    kind(evidence::build(&c.inputs), "dangling");
    fs::write(&c.inputs.results[0], RESULTS).unwrap();
    let outside = c.dir.path().join("outside.csv");
    fs::write(&outside, DATA).unwrap();
    c.inputs.datasets.push(outside);
    kind(evidence::build(&c.inputs), "path_escape");
}

#[test]
fn bundle_roundtrip_verification_and_table() {
    let c = Case::new();
    assert!(evidence::load(&c.inputs.store).unwrap().is_none());
    let b = c.build();
    evidence::save(&c.inputs.store, &b).unwrap();
    assert!(paper_dir(&c.inputs.store).join("evidence.json").is_file());
    assert_eq!(evidence::load(&c.inputs.store).unwrap(), Some(b.clone()));
    evidence::verify(&b, &c.inputs.project_root).unwrap();
    let t = evidence::load_table(&b, &c.inputs.project_root, "data.csv").unwrap();
    assert_eq!(t.columns, ["time", "n", "note"]);
    assert_eq!(t.units, [Some("s".into()), None, None]);
    assert_eq!(
        t.rows,
        vec![vec!["1", "2", "alpha,beta"], vec!["4", "2", "two\nlines"]]
    );
    let mut tampered = b.clone();
    tampered.question.push('!');
    kind(
        evidence::verify(&tampered, &c.inputs.project_root),
        "bundle_mismatch",
    );
    fs::write(&c.inputs.datasets[0], b"modified").unwrap();
    kind(
        evidence::verify(&b, &c.inputs.project_root),
        "artifact_modified",
    );
    kind(
        evidence::load_table(&b, &c.inputs.project_root, "data.csv"),
        "artifact_modified",
    );
}

#[test]
fn negative_control_one_dataset_byte_changes_verification_and_rebuild_hash() {
    let c = Case::new();
    let b = c.build();
    evidence::save(&c.inputs.store, &b).unwrap();
    record(&c.inputs.store, Step::Evidence, &[], &b.hash, true);
    record(&c.inputs.store, Step::Shortlist, &[], "s", true);
    record(
        &c.inputs.store,
        Step::Journal,
        &[("shortlist", "s")],
        "j",
        true,
    );
    record(
        &c.inputs.store,
        Step::Pass1,
        &[("evidence", &b.hash), ("journal", "j")],
        "p1",
        true,
    );
    record(&c.inputs.store, Step::Pass2, &[("pass1", "p1")], "p2", true);
    let mut bytes = DATA.to_vec();
    let i = bytes.iter().position(|x| *x == b'4').unwrap();
    bytes[i] = b'5';
    fs::write(&c.inputs.datasets[0], bytes).unwrap();
    kind(
        evidence::verify(&b, &c.inputs.project_root),
        "artifact_modified",
    );
    let rebuilt = c.build();
    assert_ne!(b.hash, rebuilt.hash);
    let s = record(&c.inputs.store, Step::Evidence, &[], &rebuilt.hash, true);
    assert!(!s.steps.contains_key(&Step::Pass1) && !s.steps.contains_key(&Step::Pass2));
    assert_eq!(evidence::load(&c.inputs.store).unwrap(), Some(b));
}

const ALL: [Step; 15] = [
    Step::Evidence,
    Step::Shortlist,
    Step::Journal,
    Step::Pass1,
    Step::Pass2,
    Step::Pass3,
    Step::Pass4,
    Step::Figures,
    Step::Citations,
    Step::Numbers,
    Step::Compile,
    Step::Render,
    Step::Disclosure,
    Step::Review,
    Step::Ready,
];
fn set(steps: &[Step]) -> BTreeSet<Step> {
    steps.iter().copied().collect()
}
#[test]
fn upstream_dag_is_exact() {
    use Step::*;
    for (step, expected) in [
        (Evidence, vec![]),
        (Shortlist, vec![]),
        (Journal, vec![Shortlist]),
        (Pass1, vec![Evidence, Journal]),
        (Pass2, vec![Pass1]),
        (Pass3, vec![Pass2]),
        (Pass4, vec![Pass3]),
        (Figures, vec![Pass4, Evidence]),
        (Citations, vec![Pass4]),
        (Numbers, vec![Pass4, Evidence, Figures]),
        (Compile, vec![Pass4, Journal, Figures, Citations]),
        (Render, vec![Compile]),
        (Disclosure, vec![Journal, Pass4]),
        (
            Review,
            vec![Compile, Render, Citations, Numbers, Disclosure],
        ),
        (Ready, ALL.iter().copied().filter(|s| *s != Ready).collect()),
    ] {
        assert_eq!(state::upstream(step).len(), expected.len(), "{step:?}");
        assert_eq!(set(state::upstream(step)), set(&expected), "{step:?}");
    }
}
fn inputs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
fn record(store: &Path, step: Step, pairs: &[(&str, &str)], output: &str, ok: bool) -> PaperState {
    state::record(store, step, inputs(pairs), output, ok, None).unwrap()
}
#[test]
fn records_current_invalidation_and_roundtrip() {
    use Step::*;
    let c = Case::new();
    let store = &c.inputs.store;
    assert!(state::load(store).unwrap().is_none());
    record(store, Evidence, &[], "e1", true);
    record(store, Shortlist, &[], "s1", true);
    record(store, Journal, &[("shortlist", "s1")], "j1", true);
    record(
        store,
        Pass1,
        &[("evidence", "e1"), ("journal", "j1")],
        "p1",
        true,
    );
    let s = record(store, Pass2, &[("pass1", "p1")], "p2", true);
    assert!(state::current(&s, Pass2));
    assert!(!state::current(&s, Pass3));
    let s = record(store, Evidence, &[], "e1", true);
    assert!(s.steps.contains_key(&Pass1) && s.steps.contains_key(&Pass2));
    let s = record(store, Evidence, &[], "e2", true);
    assert!(!s.steps.contains_key(&Pass1) && !s.steps.contains_key(&Pass2));
    assert!(s.steps.contains_key(&Journal));
    assert!(!state::current(&s, Pass2));
    let s = record(
        store,
        Pass1,
        &[("evidence", "old"), ("journal", "j1")],
        "p1b",
        true,
    );
    assert!(!state::current(&s, Pass1));
    let s = record(
        store,
        Pass1,
        &[("evidence", "e2"), ("journal", "j1")],
        "p1b",
        false,
    );
    assert!(!state::current(&s, Pass1));
    let s = state::invalidate(store, Journal).unwrap();
    assert!(!s.steps.contains_key(&Journal));
    assert!(!s.steps.contains_key(&Pass1));
    assert!(s.steps.contains_key(&Evidence));
    state::save(store, &s).unwrap();
    assert!(paper_dir(store).join("state.json").is_file());
    assert_eq!(state::load(store).unwrap(), Some(s));
}

#[test]
fn paper_saves_preserve_program_tree_and_gate() {
    use research_program::state::{ProgramState, StageEntry, StageStatus};
    let c = Case::new();
    let store = &c.inputs.store;
    let dir = research_program::program_dir(store);
    let stages = Stage::ALL
        .into_iter()
        .map(|stage| {
            let path = dir.join(stage.artifact());
            if stage.artifact().ends_with('/') {
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("nested.txt"), b"nested").unwrap();
            } else if !path.exists() {
                fs::write(path, b"{}\n").unwrap();
            }
            StageEntry {
                stage,
                status: StageStatus::Done,
                artifact: stage.artifact().into(),
                sha256: None,
                at: "fixed".into(),
                detail: None,
                members: BTreeMap::new(),
            }
        })
        .collect();
    research_program::state::save(
        store,
        &ProgramState {
            run_id: "run".into(),
            question: "question".into(),
            brief_sha256: "hash".into(),
            stages,
            complete: true,
        },
    )
    .unwrap();
    // The optional adjudication is itself a dossier, as required by the reader.
    dossier::write(
        &dir.join("07-adjudication.json"),
        &serde_json::from_str(DOSSIER).unwrap(),
    )
    .unwrap();
    let before = snapshot(&dir);
    let gate = research_program::state::gate(store);
    assert!(gate.is_ok());
    let b = c.build();
    evidence::save(store, &b).unwrap();
    let adjudication = dir.join("07-adjudication.json");
    let artifact = b
        .artifacts
        .iter()
        .find(|a| c.inputs.project_root.join(&a.path) == adjudication)
        .unwrap();
    assert_eq!(
        artifact.sha256,
        axiom::seams::sha256_hex(&fs::read(adjudication).unwrap())
    );
    let s = record(store, Step::Evidence, &[], &b.hash, true);
    state::save(store, &s).unwrap();
    assert_eq!(state::load(store).unwrap(), Some(s));
    assert_eq!(evidence::load(store).unwrap(), Some(b));
    assert_eq!(snapshot(&dir), before);
    assert_eq!(research_program::state::gate(store), gate);
}
