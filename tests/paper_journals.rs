use axiom::{
    paper::{
        cache::{Cache, FetchMode},
        http::{CannedTransport, ClockPause},
        journals::{
            doaj,
            jcr::{JcrImport, JcrRecord},
            matching::{
                self, JournalSources, FIT_WEIGHT_ABSTRACT, FIT_WEIGHT_KEYWORDS, FIT_WEIGHT_RELATED,
                FIT_WEIGHT_TOPICS, RELATED_FROM_YEARS,
            },
            openalex::{OpenAlex, WorkHit},
            requirements, sjr,
        },
        model::{
            Apc, Bucket, FilterResult, JournalRecord, Known, Money, Quartile, QuartileScheme,
            ReqStatus, Requirements, SourceKind,
        },
    },
    seams::FakeClock,
};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

const NOW: &str = "2025-01-01T00:00:00Z";
fn requirements_fixture() -> Requirements {
    serde_json::from_str(include_str!("fixtures/paper/journals/requirements.json")).unwrap()
}
fn journals_fixture() -> Vec<JournalRecord> {
    serde_json::from_str(include_str!("fixtures/paper/journals/journals.json")).unwrap()
}
#[derive(Deserialize)]
struct WorkFixture {
    id: String,
    title: String,
    abstract_words: Vec<String>,
    year: Option<i32>,
    source_id: Option<String>,
    topics: Vec<String>,
    doi: Option<String>,
}
fn related_fixture() -> Vec<WorkHit> {
    let works: Vec<WorkFixture> =
        serde_json::from_str(include_str!("fixtures/paper/journals/related.json")).unwrap();
    works
        .into_iter()
        .map(|w| WorkHit {
            id: w.id,
            title: w.title,
            abstract_words: w.abstract_words,
            year: w.year,
            source_id: w.source_id,
            topics: w.topics,
            doi: w.doi,
        })
        .collect()
}
#[derive(Deserialize)]
struct ExpectedOrder {
    ranked: Vec<ExpectedRanked>,
    excluded: Vec<String>,
}
#[derive(Deserialize)]
struct ExpectedRanked {
    id: String,
    fit: f64,
    terms: BTreeMap<String, ExpectedTerm>,
}
#[derive(Deserialize)]
struct ExpectedTerm {
    score: f64,
    evidence: String,
}
fn filter<'a>(results: &'a [FilterResult], name: &str) -> &'a FilterResult {
    results
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("missing {name}: {results:?}"))
}

#[test]
fn acceptance_fixture_order_scores_and_reasons() {
    // Wanted sets: 4 abstract words, 3 keyword words, and 6 combined topic words.
    // Scores are intersection sizes divided by those counts; related counts use a
    // maximum of 2. Expected fits are the corresponding weighted sums.
    let r = requirements_fixture();
    let journals = journals_fixture();
    let related = related_fixture();
    let expected: ExpectedOrder =
        serde_json::from_str(include_str!("fixtures/paper/journals/expected_order.json")).unwrap();
    let result = matching::rank(&r, &journals, &related, NOW);
    assert_eq!(
        result
            .ranked
            .iter()
            .map(|j| &j.journal.id)
            .collect::<Vec<_>>(),
        expected.ranked.iter().map(|j| &j.id).collect::<Vec<_>>()
    );
    assert_eq!(
        result
            .excluded
            .iter()
            .map(|j| &j.journal.id)
            .collect::<Vec<_>>(),
        expected.excluded.iter().collect::<Vec<_>>()
    );
    assert_eq!(result.requirements_hash, requirements::hash(&r));
    assert!(!result.hash.is_empty());
    assert_eq!(
        result.hash,
        matching::rank(&r, &journals, &related, NOW).hash
    );
    let changed = matching::rank(&r, &journals, &related, "2025-01-02T00:00:00Z");
    assert_ne!(result.hash, changed.hash);
    for (idx, (ranked, expected)) in result.ranked.iter().zip(&expected.ranked).enumerate() {
        assert_eq!(
            ranked.bucket,
            if idx < 2 {
                Bucket::Passes
            } else {
                Bucket::NeedsCheck
            }
        );
        assert_eq!(ranked.terms.len(), 4);
        assert_eq!(expected.terms.len(), 4);
        assert!(
            (ranked.fit - expected.fit).abs() <= 1e-9,
            "{} fit: actual {}, expected {}",
            ranked.journal.id,
            ranked.fit,
            expected.fit
        );
        for (name, weight) in [
            ("abstract", FIT_WEIGHT_ABSTRACT),
            ("keywords", FIT_WEIGHT_KEYWORDS),
            ("topics", FIT_WEIGHT_TOPICS),
            ("related", FIT_WEIGHT_RELATED),
        ] {
            let matching: Vec<_> = ranked.terms.iter().filter(|t| t.name == name).collect();
            assert_eq!(matching.len(), 1, "{name}");
            let t = matching[0];
            assert_eq!(t.weight, weight);
            let expected_term = &expected.terms[name];
            assert!(
                (t.score - expected_term.score).abs() <= 1e-9,
                "{} {name}: actual {}, expected {}",
                ranked.journal.id,
                t.score,
                expected_term.score
            );
            assert_eq!(
                t.evidence, expected_term.evidence,
                "{} {name}",
                ranked.journal.id
            );
        }
    }
    assert!(result.ranked[0].fit > result.ranked[1].fit);
    assert!(result.ranked[2].fit > result.ranked[3].fit);
    for (id, name) in [
        ("e-tier", "tier"),
        ("f-access", "access"),
        ("g-apc", "apc"),
        ("h-hybrid", "access"),
    ] {
        let excluded = result.excluded.iter().find(|j| j.journal.id == id).unwrap();
        let reason = filter(&excluded.filters, name);
        assert_eq!(reason.status, ReqStatus::Unmet, "{id}: {name}");
        assert!(!reason.detail.trim().is_empty(), "{id}: {name}");
    }
}

#[test]
fn equal_fit_ties_break_by_id() {
    let r = requirements_fixture();
    let mut a = journals_fixture().remove(0);
    let mut b = a.clone();
    a.id = "z".into();
    b.id = "a".into();
    let result = matching::rank(&r, &[a, b], &[], NOW);
    assert_eq!(
        result
            .ranked
            .iter()
            .map(|j| j.journal.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(result.ranked[0].fit, result.ranked[1].fit);
}

#[test]
fn filter_cases_and_provenance() {
    let r = requirements_fixture();
    let js = journals_fixture();
    let f = requirements::check_filters(&r, &js[1]);
    assert_eq!(filter(&f, "apc").status, ReqStatus::Met);
    assert_eq!(
        filter(&f, "apc").source.as_ref().unwrap().kind,
        SourceKind::Doaj
    );
    let f = requirements::check_filters(&r, &js[2]);
    assert_eq!(filter(&f, "apc").status, ReqStatus::Unknown);
    let f = requirements::check_filters(&r, &js[3]);
    assert_eq!(filter(&f, "apc").status, ReqStatus::Unknown);
    assert_eq!(filter(&f, "apc").detail, "currency differs");
    assert_eq!(
        filter(&f, "apc").source.as_ref().unwrap().kind,
        SourceKind::Doaj
    );
    let f = requirements::check_filters(&r, &js[7]);
    assert_eq!(filter(&f, "access").status, ReqStatus::Unmet);
    assert_eq!(
        filter(&f, "access").source.as_ref().unwrap().kind,
        SourceKind::Doaj
    );
    let mut subscription = r.clone();
    subscription.access = axiom::paper::model::AccessWanted::Subscription;
    assert_eq!(
        filter(
            &requirements::check_filters(&subscription, &js[7]),
            "access"
        )
        .status,
        ReqStatus::Met
    );
    let mut j = js[0].clone();
    j.quartiles.clear();
    let f = requirements::check_filters(&r, &j);
    assert_eq!(filter(&f, "tier").status, ReqStatus::Unknown);
    assert!(filter(&f, "tier").source.is_none());
    j = js[0].clone();
    j.access = Known::Unknown {
        reason: "not reported".into(),
    };
    let f = requirements::check_filters(&r, &j);
    assert_eq!(filter(&f, "access").status, ReqStatus::Unknown);
    assert_eq!(filter(&f, "access").detail, "not reported");
    assert!(filter(&f, "access").source.is_none());
    j = js[0].clone();
    j.quartiles[0].value.quartile = Quartile::Q2;
    let mut other_category = j.quartiles[0].clone();
    other_category.value.quartile = Quartile::Q1;
    other_category.value.category = "Neurology".into();
    j.quartiles.push(other_category);
    assert_eq!(
        filter(&requirements::check_filters(&r, &j), "tier").status,
        ReqStatus::Unmet
    );
    let mut best = j.quartiles[0].clone();
    best.value.quartile = Quartile::Q1;
    best.value.category = "ONCOLOGY".into();
    j.quartiles.push(best.clone());
    let f = requirements::check_filters(&r, &j);
    assert_eq!(filter(&f, "tier").status, ReqStatus::Met);
    assert_eq!(filter(&f, "tier").source.as_ref(), Some(&best.source));
    let mut jcr = best;
    jcr.value.scheme = QuartileScheme::Jcr;
    jcr.value.quartile = Quartile::Q2;
    jcr.source.kind = SourceKind::JcrImport;
    j.quartiles.push(jcr.clone());
    let mut r = r;
    r.tier_scheme = QuartileScheme::Jcr;
    let f = requirements::check_filters(&r, &j);
    assert_eq!(filter(&f, "tier").status, ReqStatus::Unmet);
    assert_eq!(filter(&f, "tier").source.as_ref(), Some(&jcr.source));
    j.quartiles.last_mut().unwrap().value.quartile = Quartile::Q1;
    assert_eq!(
        filter(&requirements::check_filters(&r, &j), "tier").status,
        ReqStatus::Met
    );
}

#[test]
fn validate_and_hash_requirements() {
    let r = requirements_fixture();
    requirements::validate(&r).unwrap();
    let mut bad = r.clone();
    bad.abstract_text = "  ".into();
    assert_eq!(
        requirements::validate(&bad).unwrap_err().kind,
        "invalid_requirements"
    );
    let mut bad = r.clone();
    // Negative amounts are not representable in Money (u64); exercise a
    // representable invalid limit directly through validate instead.
    bad.apc_limit = Some(Money {
        amount_minor: 0,
        currency: "XYZ".into(),
    });
    assert_eq!(
        requirements::validate(&bad).unwrap_err().kind,
        "invalid_requirements"
    );
    let mut bad = r.clone();
    bad.require_no_fee = true;
    assert_eq!(
        requirements::validate(&bad).unwrap_err().kind,
        "invalid_requirements"
    );
    assert_eq!(requirements::hash(&r), requirements::hash(&r.clone()));
    let mut changed = r.clone();
    changed.keywords.push("different".into());
    assert_ne!(requirements::hash(&r), requirements::hash(&changed));
}

#[test]
fn free_text_requires_exact_sourced_fact() {
    let mut r = requirements_fixture();
    r.free_text = vec![
        "Example Publisher".into(),
        "Example".into(),
        "not reported".into(),
    ];
    let statuses = requirements::free_text_status(&r, &journals_fixture()[0]);
    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses[0].status, ReqStatus::Met);
    assert_eq!(
        statuses[0].source.as_ref().unwrap().kind,
        SourceKind::Openalex
    );
    assert_eq!(statuses[1].status, ReqStatus::Unknown);
    assert_eq!(statuses[2].status, ReqStatus::Unknown);
}
fn imported_jcr() -> JcrImport {
    JcrImport {
        file_name: "fixture.csv".into(),
        file_sha256: "a".repeat(64),
        imported_at: NOW.into(),
        warnings: vec![],
        records: vec![JcrRecord {
            title: "Example Journal".into(),
            issn: Some("12345678".into()),
            eissn: None,
            year: 2024,
            category: "Oncology".into(),
            quartile: Quartile::Q1,
            jif: Some(2.0),
            row: 1,
        }],
    }
}
fn known_kind<T>(value: &Known<T>) -> SourceKind {
    match value {
        Known::Known(v) => v.source.kind.clone(),
        Known::Unknown { reason } => panic!("missing fact: {reason}"),
    }
}
fn gather_fixture(with_jcr: bool) {
    let dir = tempfile::tempdir().unwrap();
    let canned_dir = dir.path().join("canned");
    fs::create_dir(&canned_dir).unwrap();
    let original =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paper/journals/canned");
    for file in [
        "works.json",
        "source.json",
        "sjr-2023.csv",
        "doaj-1234-5678.json",
    ] {
        fs::copy(original.join(file), canned_dir.join(file)).unwrap();
    }
    let year: i32 = axiom::base::now_iso()[..4].parse().unwrap();
    let index = fs::read_to_string(original.join("index.json"))
        .unwrap()
        .replace("{{year}}", &(year - RELATED_FROM_YEARS).to_string());
    fs::write(canned_dir.join("index.json"), index).unwrap();
    let clock = Arc::new(FakeClock::new(1_700_000_000_000));
    let cache = Cache::open(
        &dir.path().join("cache"),
        clock.clone(),
        Arc::new(ClockPause(clock)),
    )
    .unwrap();
    let transport = CannedTransport::from_dir(&canned_dir).unwrap();
    let api = OpenAlex::new(
        &cache,
        &transport,
        FetchMode::Online,
        &dir.path().join("home"),
    )
    .unwrap();
    let sjr = sjr::load_year(&cache, &transport, FetchMode::Online, 2023).unwrap();
    let doaj_lookup = |issn: &str| doaj::journal(&cache, &transport, FetchMode::Online, issn);
    let import = imported_jcr();
    let sources = JournalSources {
        openalex: &api,
        sjr: &sjr,
        doaj: &doaj_lookup,
        jcr: if with_jcr { Some(&import) } else { None },
    };
    let mut req = requirements_fixture();
    req.abstract_text = "cancer".into();
    req.keywords = vec!["oncology".into()];
    let (journals, works) = matching::gather(&req, &sources, 20).unwrap();
    assert!(!works.is_empty());
    assert!(works.iter().any(|w| w.id == "https://openalex.org/W1"));
    assert_eq!(journals.len(), 1);
    let j = &journals[0];
    assert_eq!(j.id, "https://openalex.org/S1");
    assert_eq!(known_kind(&j.publisher), SourceKind::Openalex);
    assert_eq!(known_kind(&j.works_count), SourceKind::Openalex);
    assert_eq!(known_kind(&j.sjr), SourceKind::Sjr);
    assert_eq!(known_kind(&j.apc), SourceKind::Doaj);
    assert!(matches!(
        j.apc.value(),
        Some(Apc::Fee(Money {
            amount_minor: 120000,
            ..
        }))
    ));
    assert_eq!(known_kind(&j.access), SourceKind::Doaj);
    assert!(j
        .quartiles
        .iter()
        .any(|q| q.source.kind == SourceKind::Sjr && q.value.label().starts_with("SJR Q1 (")));
    if with_jcr {
        assert!(j
            .quartiles
            .iter()
            .any(|q| q.source.kind == SourceKind::JcrImport
                && q.value.label() == "JCR Q1 (Oncology, 2024)"));
    } else {
        assert!(!j.quartiles.is_empty());
        assert!(j
            .quartiles
            .iter()
            .all(|q| q.value.label().starts_with("SJR")));
    }
    let requests = transport.requests();
    let searches: Vec<_> = requests
        .iter()
        .filter(|url| url.contains("/works?search="))
        .collect();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|url| url.contains(&format!(
        "from_publication_date:{}-01-01",
        year - RELATED_FROM_YEARS
    ))));
    assert!(searches.iter().any(|url| url.contains("cancer")));
    assert!(searches.iter().any(|url| url.contains("oncology")));
}
#[test]
fn gather_merges_sources_by_issn_without_jcr() {
    gather_fixture(false);
}
#[test]
fn gather_merges_jcr_without_losing_sjr() {
    gather_fixture(true);
}

#[test]
fn scan_matching_sources_for_choice_identifier() {
    let forbidden = ["Journal", "Choice"].concat();
    for source in [
        include_str!("../src/paper/journals/matching.rs"),
        include_str!("../src/paper/journals/requirements.rs"),
    ] {
        assert!(!source
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|word| word == forbidden));
    }
}
