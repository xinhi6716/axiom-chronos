use axiom::paper::{
    bibliography::{BibEntry, Bibliography},
    evidence::DataTable,
    figures::{fonts::FixedMeasure, plot, presets::Preset, tikz},
    latex,
    model::*,
    tables, tectonic,
    templates::{self, TemplateEntry, TemplateStatus},
};
use std::{fs, path::Path, process::Command, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(300);
const REGISTRY: &str = include_str!("fixtures/paper/templates/registry.json");
fn entry(id: &str) -> TemplateEntry {
    let value: serde_json::Value = serde_json::from_str(REGISTRY).unwrap();
    serde_json::from_value(
        value["templates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id)
            .unwrap()
            .clone(),
    )
    .unwrap()
}
fn para(text: &str) -> Paragraph {
    Paragraph {
        id: "p".into(),
        text: text.into(),
        claims: vec![],
    }
}
fn data() -> DataTable {
    DataTable {
        columns: vec!["x".into(), "y".into()],
        units: vec![None, None],
        rows: vec![vec!["1".into(), "2".into()], vec!["2".into(), "3".into()]],
    }
}
fn manuscript() -> Manuscript {
    Manuscript {
        title: "Acceptance study".into(),
        abstract_text: para("Mean {{q:mean}} [@key]."),
        keywords: vec![],
        sections: vec![Section {
            id: "results".into(),
            heading: "Results".into(),
            paragraphs: vec![para(
                "See {{fig:plot}}, {{tab:table}}, {{sec:results}} and [@key; @other].",
            )],
        }],
        figures: vec![FigureSpec {
            id: "plot".into(),
            kind: PlotKind::Line,
            artifact: "data.csv".into(),
            x: "x".into(),
            y: vec!["y".into()],
            error: String::new(),
            preset: "test".into(),
            caption: para("Figure caption"),
        }],
        tables: vec![TableSpec {
            id: "table".into(),
            artifact: "data.csv".into(),
            columns: vec![TableColumn {
                column: "y".into(),
                header: "Value".into(),
                precision: 1,
            }],
            caption: para("Table caption"),
        }],
    }
}
fn bundle() -> EvidenceBundle {
    EvidenceBundle {
        version: 1,
        program: "test".into(),
        question: "question".into(),
        artifacts: vec![],
        claims: vec![],
        evidence: vec![],
        quantities: vec![Quantity {
            id: "mean".into(),
            claim_ids: vec![],
            value: 2.5,
            unit: None,
            n: None,
            uncertainty: None,
            test: None,
            p_value: None,
            precision: 1,
            locator: DataLocator {
                artifact: "data.csv".into(),
                column: Some("y".into()),
                row: None,
                aggregate: Some(Aggregate::Mean),
            },
        }],
        references: vec![],
        hash: String::new(),
    }
}
fn bib() -> Bibliography {
    Bibliography {
        entries: ["key", "other"]
            .into_iter()
            .map(|key| BibEntry {
                key: key.into(),
                kind: "article".into(),
                doi: None,
                openalex: None,
                title: "A study".into(),
                authors: vec!["A Author".into()],
                year: Some(2020),
                venue: Some("Journal".into()),
                volume: None,
                issue: None,
                pages: None,
                article_number: None,
            })
            .collect(),
    }
}
fn disclosure() -> Disclosure {
    let placement = "Statement at the end of the manuscript, immediately above the references, titled Declaration of Generative AI and AI-assisted technologies in the writing process";
    Disclosure {
        text: "Disclosure marker".into(),
        placement: placement.into(),
        uses: vec![],
        policy: AiPolicy {
            publisher: "Elsevier".into(),
            url: "https://example.org".into(),
            fetched_at: "2026-11-13".into(),
            sha256: "a".repeat(64),
            policy: PolicyKind::AllowedWithDisclosure,
            placement: placement.into(),
            required_items: vec![],
        },
    }
}
fn rendered(ms: &Manuscript) -> RenderedTable {
    tables::booktabs(
        &ms.tables[0],
        &data(),
        160.0,
        &FixedMeasure { em_ratio: 0.5 },
        "Latin Modern Roman",
    )
    .unwrap()
}
fn unknown<T>() -> Known<T> {
    Known::Unknown {
        reason: "missing".into(),
    }
}
fn journal(publisher: Known<String>) -> JournalRecord {
    JournalRecord {
        id: "j".into(),
        title: "Journal".into(),
        issns: vec![],
        issn_l: None,
        publisher,
        homepage: unknown(),
        quartiles: vec![],
        sjr: unknown(),
        access: unknown(),
        apc: unknown(),
        review_weeks: unknown(),
        topics: vec![],
        works_count: unknown(),
    }
}
fn known(s: &str) -> Known<String> {
    Known::Known(Sourced {
        value: s.into(),
        source: SourceTag {
            kind: SourceKind::OfficialPage,
            url: None,
            fetched_at: "2026-11-13".into(),
            data_year: None,
            raw: String::new(),
        },
    })
}
fn valid(e: &TemplateEntry) -> bool {
    [
        &e.author_page,
        &e.download_source,
        &e.version,
        &e.class,
        &e.bib_style,
    ]
    .iter()
    .all(|x| !x.trim().is_empty())
        && e.sha256.len() == 64
        && e.sha256.bytes().all(|c| c.is_ascii_hexdigit())
}
#[test]
fn registry_evidence_and_negative_control() {
    let entries = templates::registry().unwrap();
    let fixture: serde_json::Value = serde_json::from_str(REGISTRY).unwrap();
    for id in ["elsarticle", "IEEEtran", "sn-jnl", "achemso"] {
        let e = entries.iter().find(|e| e.id == id).unwrap();
        let expected: TemplateEntry = serde_json::from_value(
            fixture["templates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["id"] == id)
                .unwrap()
                .clone(),
        )
        .unwrap();
        if ["elsarticle", "IEEEtran"].contains(&id) {
            assert_eq!(e, &expected);
        }
        match &e.status {
            TemplateStatus::Verified => assert!(valid(e)),
            TemplateStatus::Unconfirmed { reason } => {
                assert!(!["elsarticle", "IEEEtran"].contains(&id));
                assert!(!reason.trim().is_empty());
            }
        }
    }
    let mut wrong = entry("elsarticle");
    wrong.sha256 = "not-a-hash".into();
    assert!(!valid(&wrong));
}
#[test]
fn journal_mapping_blocks_unknown_and_unconfirmed() {
    for (publisher, id) in [
        ("Elsevier", "elsarticle"),
        ("ELSEVIER", "elsarticle"),
        ("IEEE", "IEEEtran"),
        ("ieee", "IEEEtran"),
    ] {
        assert_eq!(
            templates::for_journal(&journal(known(publisher)))
                .unwrap()
                .id,
            id
        );
    }
    for p in [
        Known::Unknown {
            reason: "unknown".into(),
        },
        known("unmapped"),
    ] {
        assert_eq!(
            templates::for_journal(&journal(p)).unwrap_err().kind,
            "template_unconfirmed"
        );
    }
    let entries = templates::registry().unwrap();
    for (publisher, id) in [
        ("Springer Nature", "sn-jnl"),
        ("Springer", "sn-jnl"),
        ("American Chemical Society", "achemso"),
        ("ACS", "achemso"),
    ] {
        let e = entries.iter().find(|e| e.id == id).unwrap();
        let result = templates::for_journal(&journal(known(publisher)));
        match e.status {
            TemplateStatus::Verified => assert_eq!(result.unwrap().id, id),
            TemplateStatus::Unconfirmed { .. } => {
                assert_eq!(result.unwrap_err().kind, "template_unconfirmed")
            }
        }
    }
}
#[test]
fn assemble_tokens_assets_bibliography_and_disclosure() {
    let dir = tempfile::tempdir().unwrap();
    let ms = manuscript();
    let fig = dir.path().join("plot.pdf");
    fs::write(&fig, b"vector placeholder").unwrap();
    let project = latex::assemble(
        dir.path(),
        &ms,
        &bundle(),
        &entry("elsarticle"),
        &[("plot".into(), fig)],
        &[rendered(&ms)],
        &bib(),
        Some(&disclosure()),
    )
    .unwrap();
    let tex = fs::read_to_string(&project.main).unwrap();
    assert!(
        tex.contains(r"\documentclass[preprint,12pt]{elsarticle}"),
        "{tex}"
    );
    assert!(tex.contains(r"\begin{frontmatter}") && tex.contains("Anonymous author(s)"));
    assert!(tex.contains("2.5") && !tex.contains("{{q:mean}}"));
    assert!(tex.contains(r"\cite{key}") && tex.contains(r"\cite{key,other}"));
    for id in ["plot", "table", "results"] {
        let re = regex::Regex::new(&format!(r"\\ref\{{([^}}]*{id})\}}")).unwrap();
        let label = &re.captures(&tex).expect("reference")[1];
        assert!(tex.contains(&format!(r"\label{{{label}}}")), "{tex}");
    }
    assert!(
        tex.contains(r"\includegraphics")
            && tex.contains(r"\begin{tabular}")
            && tex.contains("Figure caption")
            && tex.contains("Table caption")
    );
    assert!(
        tex.contains(r"\bibliographystyle{elsarticle-num}") && tex.contains(r"\bibliography{refs}")
    );
    assert!(tex.find("Disclosure marker").unwrap() < tex.find(r"\bibliography{refs}").unwrap());
    assert!(fs::read_to_string(dir.path().join("refs.bib"))
        .unwrap()
        .contains("@article{key"));
    assert!(project.files.iter().any(|p| p.ends_with("refs.bib")));
}
#[test]
fn assemble_rejects_invalid_templates() {
    let dir = tempfile::tempdir().unwrap();
    let ms = manuscript();
    for (template, kind) in [
        (
            {
                let mut e = entry("elsarticle");
                e.class = "IEEEtran".into();
                e
            },
            "template_mismatch",
        ),
        (
            {
                let mut e = entry("elsarticle");
                e.bib_style.clear();
                e
            },
            "template_mismatch",
        ),
        (entry("sn-jnl"), "template_unconfirmed"),
    ] {
        assert_eq!(
            latex::assemble(
                dir.path(),
                &ms,
                &bundle(),
                &template,
                &[],
                &[],
                &bib(),
                None
            )
            .unwrap_err()
            .kind,
            kind
        );
    }
}
fn preset() -> Preset {
    Preset {
        id: "test".into(),
        width_mm: 90.0,
        height_mm: 65.0,
        font_family: "Latin Modern Roman".into(),
        font_files: vec![],
        cjk_font_family: None,
        cjk_font_files: vec![],
        font_pt: 8.0,
        min_font_pt: 6.0,
        line_pt: 0.5,
        palette: vec!["0072B2".into()],
        raster_dpi: 300,
    }
}
fn vector_figure(dir: &Path, ms: &Manuscript, t: &tectonic::Tectonic) -> std::path::PathBuf {
    let preset = preset();
    let layout = plot::layout(
        &ms.figures[0],
        &data(),
        &preset,
        &FixedMeasure { em_ratio: 0.5 },
    )
    .unwrap();
    let tex = dir.join("figure.tex");
    fs::write(&tex, tikz::standalone(&layout, &preset)).unwrap();
    tectonic::compile_figure(t, &tex, true, TIMEOUT).unwrap()
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn compile_verified_classes_offline() {
    let t = tectonic::find_for_tests().expect("build probe found engine");
    for id in ["elsarticle", "IEEEtran"] {
        let dir = tempfile::tempdir().unwrap();
        let mut ms = manuscript();
        ms.sections[0].paragraphs[0].text =
            "See {{fig:plot}}, {{tab:table}}, {{sec:results}} and [@key].".into();
        let figure = vector_figure(dir.path(), &ms, &t);
        assert!(figure.is_file());
        let project = latex::assemble(
            dir.path(),
            &ms,
            &bundle(),
            &entry(id),
            &[("plot".into(), figure)],
            &[rendered(&ms)],
            &bib(),
            None,
        )
        .unwrap();
        let tex = fs::read_to_string(&project.main).unwrap();
        if id == "IEEEtran" {
            assert!(tex.contains(r"\maketitle") && !tex.contains(r"\begin{frontmatter}"));
        }
        let report = tectonic::compile(&t, &project.main, true, TIMEOUT).unwrap();
        assert!(
            report.ok && report.pdf.as_ref().is_some_and(|p| p.is_file()),
            "{id}: {report:?}"
        );
        assert!(
            report.unresolved_citations.is_empty()
                && report.undefined_refs.is_empty()
                && report.missing_files.is_empty()
                && report.missing_chars.is_empty(),
            "{id}: {report:?}"
        );
    }
}
fn bad_compile(body: &str) -> tectonic::CompileReport {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.tex");
    fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\bibliographystyle{{plain}}\n\\bibliography{{refs}}\n\\end{{document}}\n")).unwrap();
    fs::write(
        dir.path().join("refs.bib"),
        axiom::paper::bibliography::to_bibtex(&bib()),
    )
    .unwrap();
    tectonic::compile(
        &tectonic::find_for_tests().expect("build probe found engine"),
        &main,
        true,
        TIMEOUT,
    )
    .unwrap()
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn compile_blocks_unresolved_citation() {
    let r = bad_compile(r"\cite{absentkey}");
    assert!(
        !r.ok
            && r.unresolved_citations
                .iter()
                .any(|s| s.contains("absentkey")),
        "{r:?}"
    );
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn compile_blocks_undefined_ref() {
    let r = bad_compile(r"\ref{absentlabel}");
    assert!(
        !r.ok && r.undefined_refs.iter().any(|s| s.contains("absentlabel")),
        "{r:?}"
    );
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn compile_blocks_missing_file() {
    let r = bad_compile(r"\input{missing-axiom-file.tex}");
    assert!(!r.ok && !r.missing_files.is_empty(), "{r:?}");
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn compile_blocks_syntax_error() {
    let r = bad_compile(r"\thiscommanddoesnotexist");
    assert!(!r.ok, "{r:?}");
}

fn isolated(name: &str) -> bool {
    if std::env::var_os("PAPER_TEMPLATE_CHILD").is_some() {
        return true;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PAPER_TEMPLATE_CHILD", "1")
        .env_remove("AXIOM_TECTONIC_CMD")
        .status()
        .unwrap();
    assert!(status.success(), "isolated {name} failed");
    false
}
#[test]
fn find_settings_command() {
    if !isolated("find_settings_command") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("engine.exe");
    fs::write(&exe, "").unwrap();
    fs::create_dir(dir.path().join("paper")).unwrap();
    let argv = vec![exe.to_string_lossy().into_owned(), "--test-arg".to_string()];
    fs::write(
        dir.path().join("paper/settings.json"),
        serde_json::to_vec(&serde_json::json!({"tectonic": argv})).unwrap(),
    )
    .unwrap();
    assert_eq!(tectonic::find(dir.path()).unwrap().argv, argv);
}
#[test]
#[cfg_attr(
    paper_tectonic,
    ignore = "engine on PATH: not_installed path untestable"
)]
fn find_not_installed() {
    if !isolated("find_not_installed") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        tectonic::find(dir.path()).unwrap_err().kind,
        "not_installed"
    );
}
