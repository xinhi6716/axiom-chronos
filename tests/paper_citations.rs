use axiom::{
    base::Res,
    paper::{
        bibliography::{self, BibEntry, Bibliography},
        cache::{Cache, FetchMode},
        citations::{self, CitationCheck, CitationReport, FieldCheck},
        crossref,
        http::{CannedTransport, ClockPause},
        journals::openalex::{OpenAlex, WorkMeta},
        model::{Manuscript, Paragraph, ReferenceSeed, Section, SourceKind},
        provenance,
    },
    seams::FakeClock,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::TempDir;

struct Rig {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    cache: Cache,
    transport: CannedTransport,
}
impl Rig {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let home = temp.path().join("home");
        let clock = Arc::new(FakeClock::new(1_700_000_000_000));
        let cache = Cache::open(&root, clock.clone(), Arc::new(ClockPause(clock))).unwrap();
        let transport = CannedTransport::from_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paper/citations"),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            home,
            cache,
            transport,
        }
    }
    fn crossref(&self, id: &str) -> Res<Option<WorkMeta>> {
        crossref::work(&self.cache, &self.transport, FetchMode::Online, id)
    }
    fn openalex(&self, id: &str) -> Res<Option<WorkMeta>> {
        OpenAlex::new(&self.cache, &self.transport, FetchMode::Online, &self.home)?.work(id)
    }
    fn check(&self, entry: &BibEntry) -> CitationCheck {
        citations::verify_entry(entry, &|id| self.crossref(id), &|id| self.openalex(id))
    }
    fn report(&self, ms: &Manuscript, bib: &Bibliography) -> CitationReport {
        citations::verify_manuscript(ms, bib, &|id| self.crossref(id), &|id| self.openalex(id))
    }
}
fn entry(
    key: &str,
    doi: Option<&str>,
    title: &str,
    authors: &[&str],
    year: i32,
    venue: &str,
    pages: Option<&str>,
) -> BibEntry {
    BibEntry {
        key: key.into(),
        kind: "article".into(),
        doi: doi.map(str::to_owned),
        openalex: None,
        title: title.into(),
        authors: authors.iter().map(|s| (*s).into()).collect(),
        year: Some(year),
        venue: Some(venue.into()),
        volume: None,
        issue: None,
        pages: pages.map(str::to_owned),
        article_number: None,
    }
}
fn article() -> BibEntry {
    entry(
        "article",
        Some("10.1038/nature14539"),
        "Deep learning",
        &["LeCun, Yann", "Yoshua Bengio", "Geoffrey Hinton"],
        2015,
        "Nature",
        Some("436-444"),
    )
}
fn manuscript(abstract_text: &str, paragraph_text: &str) -> Manuscript {
    Manuscript {
        title: "Test".into(),
        abstract_text: Paragraph {
            id: "a".into(),
            text: abstract_text.into(),
            claims: vec![],
        },
        keywords: vec![],
        sections: vec![Section {
            id: "s".into(),
            heading: "Section".into(),
            paragraphs: vec![Paragraph {
                id: "p".into(),
                text: paragraph_text.into(),
                claims: vec![],
            }],
        }],
        figures: vec![],
        tables: vec![],
    }
}
fn assert_mismatch(check: &CitationCheck, field: &str) {
    assert!(!check.verified, "{check:?}");
    assert!(
        matches!(check.fields.get(field), Some(FieldCheck::Mismatch { .. })),
        "{check:?}"
    );
}
fn assert_unknown(check: &CitationCheck, field: &str) {
    assert!(!check.verified, "{check:?}");
    assert!(
        matches!(check.fields.get(field), Some(FieldCheck::Unknown { reason }) if !reason.trim().is_empty()),
        "{check:?}"
    );
}

#[test]
fn normalization_valid_and_invalid() {
    for (raw, expected) in [
        (
            "  HTTPS://DOI.ORG/10.1038/NATURE14539.) ",
            "10.1038/nature14539",
        ),
        ("http://DX.DOI.ORG/10.1234/ABC;", "10.1234/abc"),
        ("DoI:10.1/A,", "10.1/a"),
    ] {
        assert_eq!(citations::normalize_doi(raw).as_deref(), Some(expected));
    }
    for raw in ["", "11.123/a", "10.123", "doi:garbage"] {
        assert_eq!(citations::normalize_doi(raw), None);
    }
    for raw in ["W123", "w123", "https://openalex.org/W123"] {
        assert_eq!(citations::normalize_openalex(raw).as_deref(), Some("W123"));
    }
    for raw in ["", "W", "W12x", "S123", "https://openalex.org/W"] {
        assert_eq!(citations::normalize_openalex(raw), None);
    }
}

#[test]
fn crossref_mapping_provenance_and_404() {
    let r = Rig::new();
    let w = r
        .crossref("HTTPS://DOI.ORG/10.1038/NATURE14539")
        .unwrap()
        .unwrap();
    assert_eq!(w.id, "10.1038/nature14539");
    assert_eq!(w.doi.as_deref(), Some("10.1038/nature14539"));
    assert_eq!(w.title.as_deref(), Some("Deep learning"));
    assert_eq!(
        w.authors,
        ["Yann LeCun", "Yoshua Bengio", "Geoffrey Hinton"]
    );
    assert_eq!(w.year, Some(2015));
    assert_eq!(w.venue.as_deref(), Some("Nature"));
    assert_eq!(w.volume.as_deref(), Some("1"));
    assert_eq!(w.issue.as_deref(), Some("2"));
    assert_eq!(w.first_page.as_deref(), Some("436"));
    assert_eq!(w.last_page.as_deref(), Some("444"));
    assert_eq!(w.article_number, None);
    assert_eq!(w.source.kind, SourceKind::Crossref);
    assert!(w.source.raw.ends_with('#'));
    assert_eq!(
        provenance::resolve(&r.root, &w.source).unwrap()["message"]["title"][0],
        "Deep learning"
    );
    assert!(r.crossref("10.9999/fake").unwrap().is_none());
    assert!(r.openalex("10.9999/fake").unwrap().is_none());
    assert!(r
        .transport
        .requests()
        .contains(&"https://api.crossref.org/works/10.1038%2Fnature14539".to_string()));
}

#[test]
fn real_work_verified_and_wrong_title_authors_and_pages_block() {
    let good = article();
    let control = Rig::new().check(&good);
    assert!(control.verified, "{control:?}");
    assert_eq!(control.sources.len(), 2);
    for (field, altered) in [
        ("title", {
            let mut e = good.clone();
            e.title = "A different title".into();
            e
        }),
        ("authors", {
            let mut e = good.clone();
            e.authors = vec!["Wrong Author".into()];
            e
        }),
        ("pages", {
            let mut e = good.clone();
            e.pages = Some("900-901".into());
            e
        }),
    ] {
        let r = Rig::new();
        let check = r.check(&altered);
        assert_mismatch(&check, field);
        let report = r.report(
            &manuscript("[@article]", ""),
            &Bibliography {
                entries: vec![altered],
            },
        );
        assert!(
            report
                .blocking
                .contains(&"unverified citation article".to_string()),
            "{report:?}"
        );
    }
}

#[test]
fn mismatches_from_either_source_win() {
    let r = Rig::new();
    for (doi, field, title, authors, venue, pages) in [
        (
            "10.1038/nature12373",
            "title",
            "Wrong title",
            vec!["Georg Kucsko"],
            "Nature",
            "54-58",
        ),
        (
            "10.1038/nature12374",
            "authors",
            "A published article",
            vec!["Wrong Person"],
            "Journal of Machine Learning Research",
            "10-20",
        ),
        (
            "10.1038/nature12375",
            "pages",
            "Another published article",
            vec!["Grace Hopper"],
            "Journal of Machine Learning Research",
            "101-110",
        ),
    ] {
        let e = entry("bad", Some(doi), title, &authors, 2013, venue, Some(pages));
        assert_mismatch(&r.check(&e), field);
    }
    let r = Rig::new();
    let mut e = article();
    e.title = "Deep learning".into();
    let check = citations::verify_entry(&e, &|id| r.crossref(id), &|id| {
        let mut work = r.openalex(id)?.unwrap();
        work.title = Some("Contradictory title".into());
        Ok(Some(work))
    });
    assert_mismatch(&check, "title");
}

#[test]
fn fake_doi_and_unknown_key_block_manuscript() {
    let mut fake = article();
    fake.key = "fake".into();
    fake.doi = Some("10.9999/fake".into());
    let ms = manuscript("[@fake; @missing]", "[@missing]");
    let report = Rig::new().report(
        &ms,
        &Bibliography {
            entries: vec![fake],
        },
    );
    assert_eq!(report.manuscript_hash, ms.hash());
    assert_eq!(report.checks.len(), 1);
    assert_unknown(&report.checks[0], "title");
    assert!(report.checks[0].sources.is_empty());
    assert_eq!(
        report.unknown_keys,
        vec![
            ("abstract".into(), "missing".into()),
            ("sec:s/p:p".into(), "missing".into())
        ]
    );
    for expected in [
        "unverified citation fake",
        "unknown bib key missing at abstract",
        "unknown bib key missing at sec:s/p:p",
    ] {
        assert!(report.blocking.contains(&expected.to_string()));
    }
    let good = article();
    let clean = manuscript("[@article]", "[@article]");
    let report = Rig::new().report(
        &clean,
        &Bibliography {
            entries: vec![good],
        },
    );
    assert!(report.blocking.is_empty(), "{report:?}");
    assert!(report.unknown_keys.is_empty());
    assert_eq!(report.checks.len(), 1);
    assert!(report.checks[0].verified);
    assert_eq!(report.manuscript_hash, clean.hash());
}

#[test]
fn doi_less_openalex_and_iso_abbreviation_verify_with_negative_control() {
    let mut e = entry(
        "openalex",
        None,
        "A Work Without a DOI",
        &["Novák, Mária"],
        2020,
        "J. Mach. Learn. Res.",
        Some("1-9"),
    );
    e.openalex = Some("https://openalex.org/W123".into());
    let r = Rig::new();
    let check = r.check(&e);
    assert!(check.verified, "{check:?}");
    assert_eq!(check.id.as_deref(), Some("W123"));
    assert_eq!(check.sources.len(), 1);
    assert!(!r
        .transport
        .requests()
        .iter()
        .any(|url| url.contains("crossref")));
    let mut bad = e.clone();
    bad.venue = Some("J. Wrong Res.".into());
    assert_mismatch(&Rig::new().check(&bad), "venue");
    e.title = "A work without a DOI!".into();
    e.authors = vec!["Mária Novák".into()];
    assert!(Rig::new().check(&e).verified);
}

#[test]
fn article_number_and_legal_abbreviation_verify_with_negative_controls() {
    let mut electronic = entry(
        "electronic",
        Some("10.1371/journal.pone.0000308"),
        "A paper with an article number",
        &["Jane Smith"],
        2007,
        "PLoS ONE",
        None,
    );
    electronic.article_number = Some("e308".into());
    assert!(Rig::new().check(&electronic).verified);
    let mut bad = electronic.clone();
    bad.article_number = Some("e999".into());
    assert_mismatch(&Rig::new().check(&bad), "article_number");
    electronic.article_number = None;
    electronic.pages = Some("e308".into());
    assert!(Rig::new().check(&electronic).verified);
    electronic.pages = Some("e999".into());
    assert_mismatch(&Rig::new().check(&electronic), "pages");
    let law = entry(
        "law",
        Some("10.2307/1330360"),
        "The Right to Privacy",
        &["Samuel Warren", "Louis Brandeis"],
        1890,
        "Harv. L. Rev.",
        Some("193-220"),
    );
    assert!(Rig::new().check(&law).verified);
    let mut bad = law;
    bad.venue = Some("Harv. X. Rev.".into());
    assert_mismatch(&Rig::new().check(&bad), "venue");
}

#[test]
fn unavailable_and_absent_data_stay_unknown_and_block() {
    let mut e = article();
    e.doi = Some("10.9999/unavailable".into());
    let r = Rig::new();
    assert_eq!(r.crossref("10.9999/unavailable").unwrap_err().kind, "http");
    assert_eq!(r.openalex("10.9999/unavailable").unwrap_err().kind, "http");
    // Canned responses are consumed, so use a fresh rig for the actual verification.
    let r = Rig::new();
    let check = r.check(&e);
    for field in ["title", "authors", "year", "venue", "pages"] {
        assert_unknown(&check, field);
    }
    assert_eq!(check.fields["article_number"], FieldCheck::NotApplicable);
    assert!(check.sources.is_empty());
    let report = Rig::new().report(
        &manuscript("[@article]", ""),
        &Bibliography { entries: vec![e] },
    );
    assert!(report
        .blocking
        .contains(&"unverified citation article".to_string()));
    let mut no_id = article();
    no_id.doi = None;
    let r = Rig::new();
    let check = r.check(&no_id);
    assert_unknown(&check, "title");
    assert!(r.transport.requests().is_empty());
    let mut missing = article();
    missing.title.clear();
    missing.authors.clear();
    missing.year = None;
    missing.venue = None;
    missing.pages = None;
    let check = Rig::new().check(&missing);
    for field in ["title", "authors", "year"] {
        assert_unknown(&check, field);
    }
    for field in ["venue", "pages", "article_number"] {
        assert_eq!(check.fields[field], FieldCheck::NotApplicable);
    }
    assert!(Rig::new().check(&article()).verified);
}

#[test]
fn seeds_keep_identifiers_but_invent_nothing_for_missing_or_error() {
    let seeds = vec![
        ReferenceSeed {
            key: "missing".into(),
            doi: Some("DOI:10.9999/FAKE".into()),
            openalex: Some("w123".into()),
            url: Some("https://irrelevant.example/title".into()),
            evidence: vec!["do not infer".into()],
        },
        ReferenceSeed {
            key: "error".into(),
            doi: Some("10.9999/UNAVAILABLE".into()),
            openalex: None,
            url: None,
            evidence: vec![],
        },
        ReferenceSeed {
            key: "found".into(),
            doi: Some("10.1038/nature14539".into()),
            openalex: Some("W999".into()),
            url: None,
            evidence: vec![],
        },
    ];
    let r = Rig::new();
    let bib = bibliography::from_seeds(&seeds, &|seed| r.crossref(seed.doi.as_deref().unwrap()));
    assert_eq!(
        bib.entries
            .iter()
            .map(|e| e.key.as_str())
            .collect::<Vec<_>>(),
        ["missing", "error", "found"]
    );
    for e in &bib.entries[..2] {
        assert!(e.title.is_empty() && e.authors.is_empty());
        assert!(
            e.year.is_none()
                && e.venue.is_none()
                && e.volume.is_none()
                && e.issue.is_none()
                && e.pages.is_none()
                && e.article_number.is_none()
        );
        assert_eq!(e.kind, "misc");
    }
    assert_eq!(bib.entries[0].doi.as_deref(), Some("10.9999/fake"));
    assert_eq!(bib.entries[0].openalex.as_deref(), Some("W123"));
    assert_eq!(bib.entries[1].doi.as_deref(), Some("10.9999/unavailable"));
    let found = &bib.entries[2];
    assert_eq!(found.title, "Deep learning");
    assert_eq!(found.kind, "article");
    assert_eq!(found.pages.as_deref(), Some("436-444"));
    assert_eq!(found.openalex, None); // Never copy an identifier absent from the record.
}

#[test]
fn bibtex_roundtrip_sorted_escaped_types_and_malformed() {
    let input = r#"
@conference{z, author = {Doe, Jane and John Smith}, title = {Nested {Ideas} \& 50\%}, year=2020, booktitle="Symposium", number={2}, pages={1-9}, doi={HTTPS://DOI.ORG/10.1/ABC}, openalex={w123}}
@article{a, author={A Author}, title={A\_B \#1 \$5 \{ok\}}, year={2021}, journaltitle={J. Test}, eid={e42}}
@legal{law, title={Law}, howpublished={Reports}}
@book{book, title={Book}}
@unknown{misc, title={Other}, articleno={123}}
"#;
    let parsed = bibliography::parse_bibtex(input).unwrap();
    assert_eq!(
        parsed
            .entries
            .iter()
            .map(|e| e.key.as_str())
            .collect::<Vec<_>>(),
        ["z", "a", "law", "book", "misc"]
    );
    assert_eq!(parsed.entries[0].kind, "inproceedings");
    assert_eq!(parsed.entries[0].authors, ["Doe, Jane", "John Smith"]);
    assert_eq!(parsed.entries[0].doi.as_deref(), Some("10.1/abc"));
    assert_eq!(parsed.entries[0].openalex.as_deref(), Some("W123"));
    assert!(parsed.entries[0].title.contains('&') && parsed.entries[0].title.contains('%'));
    assert_eq!(parsed.entries[1].title, "A_B #1 $5 {ok}");
    assert_eq!(parsed.entries[1].article_number.as_deref(), Some("e42"));
    assert_eq!(parsed.entries[2].kind, "legal");
    assert_eq!(parsed.entries[3].kind, "book");
    assert_eq!(parsed.entries[4].kind, "misc");
    let rendered = bibliography::to_bibtex(&parsed);
    let roundtrip = bibliography::parse_bibtex(&rendered).unwrap();
    let mut expected = parsed.clone();
    expected.entries.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(roundtrip, expected);
    assert_eq!(bibliography::to_bibtex(&roundtrip), rendered);
    for bad in [
        "@article{x, title={unclosed}",
        "@article{x title={missing comma}}",
        "@article{x, year=\"unclosed}",
    ] {
        assert_eq!(bibliography::parse_bibtex(bad).unwrap_err().kind, "bibtex");
    }
}

#[test]
fn one_successful_source_can_verify_after_the_other_errors() {
    let r = Rig::new();
    let check =
        citations::verify_entry(&article(), &|_| r.crossref("10.9999/unavailable"), &|id| {
            r.openalex(id)
        });
    assert!(check.verified, "{check:?}");
    assert_eq!(check.sources.len(), 1);
    assert_eq!(check.sources[0].kind, SourceKind::Openalex);
}

#[test]
fn all_cited_entries_are_checked_once_in_key_order() {
    let mut a = article();
    a.key = "a".into();
    let mut z = a.clone();
    z.key = "z".into();
    let ms = manuscript("[@z; @a]", "[@z]");
    let report = Rig::new().report(
        &ms,
        &Bibliography {
            entries: vec![z, a],
        },
    );
    assert!(report.blocking.is_empty(), "{report:?}");
    assert_eq!(
        report
            .checks
            .iter()
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
}

#[test]
fn bibtex_writer_roundtrips_special_characters_and_field_order() {
    let mut e = article();
    e.title = "& % $ # _ { } ~ ^ \\".into();
    e.volume = Some("12".into());
    e.issue = Some("3".into());
    e.article_number = Some("e1".into());
    e.openalex = Some("W123".into());
    let bib = Bibliography { entries: vec![e] };
    let text = bibliography::to_bibtex(&bib);
    let fields = [
        "author", "title", "year", "journal", "volume", "number", "pages", "eid", "doi", "openalex",
    ];
    let emitted: Vec<_> = text
        .lines()
        .filter_map(|line| line.trim().split_once('=').map(|(name, _)| name.trim()))
        .collect();
    assert_eq!(emitted, fields);
    assert_eq!(bibliography::parse_bibtex(&text).unwrap(), bib);
}
