use axiom::paper::{
    integrity::{self, IntegrityKind, IntegrityReport},
    quantities, statistics, *,
};
use std::{fs, path::Path};
use tempfile::{tempdir, TempDir};

const DATA: &[u8] = include_bytes!("fixtures/paper/integrity/data.csv");
const MANUSCRIPT: &str = include_str!("fixtures/paper/integrity/manuscript.json");

struct Case {
    dir: TempDir,
    bundle: EvidenceBundle,
    ms: Manuscript,
}
impl Case {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("data.csv"), DATA).unwrap();
        let q = Quantity {
            id: "q1".into(),
            claim_ids: vec!["C1".into()],
            value: 2.5,
            unit: Some("s".into()),
            n: None,
            uncertainty: None,
            test: None,
            p_value: None,
            precision: 1,
            locator: DataLocator {
                artifact: "data.csv".into(),
                column: Some("time".into()),
                row: None,
                aggregate: Some(Aggregate::Mean),
            },
        };
        let mut bundle = EvidenceBundle {
            version: 1,
            program: "study".into(),
            question: "Time?".into(),
            artifacts: vec![ArtifactRef {
                path: "data.csv".into(),
                sha256: axiom::seams::sha256_hex(DATA),
                kind: ArtifactKind::Dataset,
                producer: "study".into(),
            }],
            claims: vec![
                Claim {
                    id: "C1".into(),
                    text: "Time".into(),
                    evidence: vec![],
                    confidence: Confidence::High,
                },
                Claim {
                    id: "C2".into(),
                    text: "Other claim".into(),
                    evidence: vec![],
                    confidence: Confidence::High,
                },
            ],
            evidence: vec![],
            quantities: vec![q],
            references: vec![ReferenceSeed {
                key: "study2020".into(),
                doi: None,
                openalex: None,
                url: None,
                evidence: vec![],
            }],
            hash: String::new(),
        };
        bundle.hash = bundle.compute_hash();
        let ms = serde_json::from_str(MANUSCRIPT.trim_start_matches('\u{feff}')).unwrap();
        Self { dir, bundle, ms }
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn report(&self, tables: &[RenderedTable]) -> IntegrityReport {
        integrity::check(&self.ms, &self.bundle, tables, self.root()).unwrap()
    }
}
fn issue(report: &IntegrityReport, kind: IntegrityKind, location: &str) {
    assert!(!report.passed, "an integrity issue must block: {report:?}");
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.kind == kind && i.location == location),
        "missing {kind:?} at {location}: {:?}",
        report.issues
    );
}
fn semantic_issue(q: &Quantity, sentence: &str, kind: IntegrityKind) {
    let issues = statistics::semantics(q, sentence);
    assert!(
        issues.iter().any(|i| i.kind == kind),
        "missing {kind:?}: {issues:?}"
    );
}

#[test]
fn seeded_percentage_is_unsourced_located_and_blocking() {
    let mut c = Case::new();
    c.ms.sections[0].paragraphs[0].text = "The intervention improved outcomes by 73%.".into();
    issue(
        &c.report(&[]),
        IntegrityKind::UnsourcedNumber,
        "sec:s1/p:p1",
    );
}

#[test]
fn dataset_presence_does_not_source_an_unrelated_literal() {
    let mut c = Case::new();
    // 87 is in the rate column, but q1 locates the mean of time, not that value.
    c.ms.abstract_text.text = "The time was {{q:q1}}; the result was 87%.".into();
    issue(&c.report(&[]), IntegrityKind::UnsourcedNumber, "abstract");
}

#[test]
fn correct_value_attached_to_wrong_claim_is_blocking() {
    let mut c = Case::new();
    c.ms.abstract_text.claims[0].claim = "C2".into();
    issue(&c.report(&[]), IntegrityKind::WrongClaim, "abstract");
}

#[test]
fn swapped_sd_ci_and_se_as_sd_are_statistical_errors() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    q.uncertainty = Some(Uncertainty::Sd(0.3));
    semantic_issue(
        &q,
        "The mean and confidence interval were {{q:q1}}.",
        IntegrityKind::StatSemantics,
    );
    q.uncertainty = Some(Uncertainty::Ci {
        level: 0.95,
        low: 2.1,
        high: 2.9,
    });
    semantic_issue(
        &q,
        "The mean and standard deviation were {{q:q1}}.",
        IntegrityKind::StatSemantics,
    );
    q.uncertainty = Some(Uncertainty::Se(0.3));
    semantic_issue(
        &q,
        "The mean and SD were {{q:q1}}.",
        IntegrityKind::StatSemantics,
    );
}

#[test]
fn invalid_ci_level_and_missing_test_or_n_are_reported() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    q.uncertainty = Some(Uncertainty::Ci {
        level: 1.1,
        low: 2.1,
        high: 2.9,
    });
    semantic_issue(
        &q,
        "The confidence interval was {{q:q1}}.",
        IntegrityKind::StatSemantics,
    );
    q.uncertainty = None;
    q.p_value = Some(0.03);
    q.test = Some("t-test".into());
    semantic_issue(&q, "The result was {{q:q1}}.", IntegrityKind::MissingN);
    q.n = Some(20);
    q.test = None;
    semantic_issue(&q, "The result was {{q:q1}}.", IntegrityKind::MissingTest);
}

#[test]
fn statistical_issues_propagate_into_report_at_paragraph_location() {
    let mut c = Case::new();
    c.bundle.quantities[0].uncertainty = Some(Uncertainty::Se(0.3));
    c.bundle.hash = c.bundle.compute_hash();
    c.ms.abstract_text.text = "The mean and SD were {{q:q1}}.".into();
    issue(&c.report(&[]), IntegrityKind::StatSemantics, "abstract");
}

#[test]
fn disagreement_between_text_and_table_is_located_and_blocking() {
    let mut c = Case::new();
    c.ms.tables.push(TableSpec {
        id: "t1".into(),
        artifact: "data.csv".into(),
        columns: vec![TableColumn {
            column: "time".into(),
            header: "Time".into(),
            precision: 1,
        }],
        caption: Paragraph {
            id: "tc".into(),
            text: "Time results".into(),
            claims: vec![],
        },
    });
    let table = RenderedTable {
        id: "t1".into(),
        latex: String::new(),
        cells: vec![vec!["2.8".into()]],
        headers: vec!["Time".into()],
        units: vec![Some("s".into())],
        width_mm: 20.0,
    };
    let report = c.report(&[table]);
    assert!(!report.passed);
    issue(&report, IntegrityKind::Inconsistent, "tab:t1/r0c0");
}

#[test]
fn incorrect_unit_fails_verification_and_integrity() {
    let mut c = Case::new();
    c.bundle.quantities[0].unit = Some("kg".into());
    c.bundle.hash = c.bundle.compute_hash();
    let result = quantities::verify(&c.bundle, c.root(), &c.bundle.quantities[0]).unwrap();
    assert!(!result.ok, "wrong unit must fail: {result:?}");
    issue(&c.report(&[]), IntegrityKind::WrongUnit, "abstract");
}

#[test]
fn fill_replaces_known_quantities_and_rejects_dangling_tokens() {
    let c = Case::new();
    let filled = quantities::fill("Mean: {{q:q1}}.", &c.bundle).unwrap();
    assert_eq!(
        filled,
        format!("Mean: {}.", quantities::render(&c.bundle.quantities[0]))
    );
}

#[test]
fn fill_rejects_a_dangling_quantity() {
    let c = Case::new();
    assert_eq!(
        quantities::fill("{{q:unknown}}", &c.bundle)
            .unwrap_err()
            .kind,
        "dangling"
    );
}

#[test]
fn clean_manuscript_accepts_rounding_ordinals_and_citation_digits() {
    let c = Case::new();
    let report = c.report(&[]);
    assert!(report.passed, "{report:?}");
    assert!(report.issues.is_empty(), "{report:?}");
    assert_eq!(report.manuscript_hash, c.ms.hash());
    assert_eq!(report.evidence_hash, c.bundle.hash);
}

#[test]
fn render_uncertainty_and_metadata_in_order() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    q.n = Some(20);
    q.test = Some("t-test".into());
    q.p_value = Some(0.03);
    for (uncertainty, expected) in [
        (Uncertainty::Sd(0.3), "SD"),
        (Uncertainty::Se(0.3), "SE"),
        (
            Uncertainty::Ci {
                level: 0.95,
                low: 2.1,
                high: 2.9,
            },
            "95% CI [2.1, 2.9]",
        ),
    ] {
        q.uncertainty = Some(uncertainty);
        let rendered = quantities::render(&q).replace("\\%", "%");
        let value = rendered.find("2.5").expect(&rendered);
        let unit = rendered.find('s').expect(&rendered);
        let uncertainty = rendered.find(expected).expect(&rendered);
        if !matches!(q.uncertainty, Some(Uncertainty::Ci { .. })) {
            assert!(
                (rendered.contains('±') || rendered.contains("\\pm")) && rendered.contains("0.3"),
                "{rendered}"
            );
        }
        let n = rendered.find("20").expect(&rendered);
        let test = rendered.find("t-test").expect(&rendered);
        let p = rendered.find("0.03").expect(&rendered);
        assert!(
            value < unit && unit < uncertainty && uncertainty < n && n < test && test < p,
            "{rendered}"
        );
    }
}

#[test]
fn verification_recomputes_mean_and_uses_half_unit_precision() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    let result = quantities::verify(&c.bundle, c.root(), &q).unwrap();
    assert_eq!(result.id, q.id);
    assert!((result.recomputed.unwrap() - 2.54).abs() < 1e-10);
    assert!(result.ok, "2.54 legitimately rounds to 2.5: {result:?}");
    q.value = 2.4;
    let result = quantities::verify(&c.bundle, c.root(), &q).unwrap();
    assert!(
        !result.ok,
        "2.54 is beyond half a tenth from 2.4: {result:?}"
    );
}

fn table_case(c: &mut Case, cells: Vec<Vec<&str>>) -> RenderedTable {
    c.ms.tables.push(TableSpec {
        id: "t1".into(),
        artifact: "data.csv".into(),
        columns: vec![TableColumn {
            column: "time".into(),
            header: "Time".into(),
            precision: 1,
        }],
        caption: Paragraph {
            id: "tc".into(),
            text: "Time results".into(),
            claims: vec![],
        },
    });
    RenderedTable {
        id: "t1".into(),
        latex: String::new(),
        cells: cells
            .into_iter()
            .map(|row| row.into_iter().map(str::to_owned).collect())
            .collect(),
        headers: vec!["Time".into()],
        units: vec![Some("s".into())],
        width_mm: 20.0,
    }
}

#[test]
fn table_cells_are_sourced_only_by_their_data_cell_or_verified_quantity() {
    let mut c = Case::new();
    let mut table = table_case(&mut c, vec![vec!["2.4"], vec!["2.6"]]);
    let report = c.report(&[table.clone()]);
    assert!(report.passed, "{report:?}");
    table.cells[0][0] = "2.6".into(); // The other row does not source this cell.
    table.cells[1][0] = "87.0".into(); // A different data column does not source it either.
    let report = c.report(&[table]);
    issue(&report, IntegrityKind::UnsourcedNumber, "tab:t1/r0c0");
    issue(&report, IntegrityKind::UnsourcedNumber, "tab:t1/r1c0");
    let mut c = Case::new();
    let mut table = table_case(&mut c, vec![vec!["2.5"]]);
    assert!(c.report(&[table.clone()]).passed); // Matches verified mean, not row 0.
    c.bundle.quantities[0].value = 9.0; // A used but unverified quantity cannot source it.
    table.cells[0][0] = "9.0".into();
    issue(
        &c.report(&[table]),
        IntegrityKind::UnsourcedNumber,
        "tab:t1/r0c0",
    );
}

#[test]
fn caption_and_table_cell_literals_are_located() {
    let mut c = Case::new();
    let table = table_case(&mut c, vec![vec!["observed 73"]]);
    c.ms.tables[0].caption.text = "Results from 73 participants".into();
    let report = c.report(&[table]);
    issue(&report, IntegrityKind::UnsourcedNumber, "tab:t1/caption");
    issue(&report, IntegrityKind::UnsourcedNumber, "tab:t1/r0c0");
}

#[test]
fn literal_years_heading_numbers_and_prose_ordinals_are_not_exempt() {
    for text in [
        "in 2020",
        "[@unknown]2020",
        "table 73 participants",
        "section 73 participants",
    ] {
        let mut c = Case::new();
        c.ms.sections[0].paragraphs[0].text = text.into();
        issue(
            &c.report(&[]),
            IntegrityKind::UnsourcedNumber,
            "sec:s1/p:p1",
        );
    }
    let mut c = Case::new();
    c.ms.sections[0].heading = "1 Results for 73 participants".into();
    issue(
        &c.report(&[]),
        IntegrityKind::UnsourcedNumber,
        "sec:s1/heading",
    );
}

#[test]
fn dangling_tokens_are_reported_at_the_paragraph() {
    for token in [
        "{{q:unknown}}",
        "[@unknown]",
        "{{fig:unknown}}",
        "{{tab:unknown}}",
        "{{sec:unknown}}",
    ] {
        let mut c = Case::new();
        c.ms.sections[0].paragraphs[0].text = token.into();
        issue(&c.report(&[]), IntegrityKind::Dangling, "sec:s1/p:p1");
    }
}

#[test]
fn missing_n_and_test_propagate_to_check() {
    let mut c = Case::new();
    c.bundle.quantities[0].p_value = Some(0.03);
    let report = c.report(&[]);
    issue(&report, IntegrityKind::MissingN, "abstract");
    issue(&report, IntegrityKind::MissingTest, "abstract");
}

#[test]
fn ci_level_boundaries_are_invalid_and_percentage_is_rounded() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    for level in [0.0, 1.0] {
        q.uncertainty = Some(Uncertainty::Ci {
            level,
            low: 2.1,
            high: 2.9,
        });
        semantic_issue(
            &q,
            "confidence interval {{q:q1}}",
            IntegrityKind::StatSemantics,
        );
    }
    for (level, expected) in [(0.07, "7\\%"), (0.95, "95\\%")] {
        q.uncertainty = Some(Uncertainty::Ci {
            level,
            low: 2.1,
            high: 2.9,
        });
        assert!(
            quantities::render(&q).contains(expected),
            "{}",
            quantities::render(&q)
        );
    }
}

#[test]
fn render_rounds_at_each_precision_and_escapes_latex() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    q.value = 2.54;
    for (precision, expected) in [(0, "3 s"), (1, "2.5 s"), (2, "2.54 s")] {
        q.precision = precision;
        assert_eq!(quantities::render(&q), expected);
    }
    q.unit = Some("%_&".into());
    q.test = Some("a%_&b".into());
    assert_eq!(quantities::render(&q), "2.54 \\%\\_\\& a\\%\\_\\&b");
}

#[test]
fn verify_aggregates_rows_and_invalid_data() {
    let c = Case::new();
    let mut q = c.bundle.quantities[0].clone();
    for (aggregate, value) in [
        (Aggregate::Count, 2.0),
        (Aggregate::Sum, 5.08),
        (Aggregate::Min, 2.44),
        (Aggregate::Max, 2.64),
        (Aggregate::Median, 2.54),
        (Aggregate::Sd, (0.02_f64).sqrt()),
        (Aggregate::Se, 0.1),
    ] {
        q.precision = 2;
        q.value = value;
        q.locator.aggregate = Some(aggregate);
        let result = quantities::verify(&c.bundle, c.root(), &q).unwrap();
        assert!(result.ok, "{result:?}");
        assert!((result.recomputed.unwrap() - value).abs() < 1e-10);
    }
    q.locator.aggregate = None;
    q.locator.row = Some(1);
    q.value = 2.64;
    assert!(quantities::verify(&c.bundle, c.root(), &q).unwrap().ok);
    q.locator.column = Some("missing".into());
    let result = quantities::verify(&c.bundle, c.root(), &q).unwrap();
    assert!(!result.ok && result.recomputed.is_none());
    let data = b"time [s]\nnot-a-number\n";
    fs::write(c.root().join("nonnumeric.csv"), data).unwrap();
    let mut bundle = c.bundle.clone();
    bundle.artifacts.push(ArtifactRef {
        path: "nonnumeric.csv".into(),
        sha256: axiom::seams::sha256_hex(data),
        kind: ArtifactKind::Dataset,
        producer: "study".into(),
    });
    q.locator.artifact = "nonnumeric.csv".into();
    q.locator.column = Some("time".into());
    q.locator.row = Some(0);
    let result = quantities::verify(&bundle, c.root(), &q).unwrap();
    assert!(!result.ok && result.recomputed.is_none());
}
