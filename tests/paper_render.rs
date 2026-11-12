use axiom::base::Res;
use axiom::paper::{
    bibliography::{BibEntry, Bibliography},
    evidence::DataTable,
    figures::{
        audit::{audit_figure, audit_table, AuditIssue, IssueKind},
        fonts::{FixedMeasure, FontMeasure, TextMeasure},
        layout::{FigureLayout, TextBox, TextRole},
        pages::check_pdf,
        plot,
        presets::Preset,
        tikz,
    },
    latex,
    model::*,
    tables, tectonic,
    templates::TemplateEntry,
};
use flate2::{write::ZlibEncoder, Compression};
use std::{fs, io::Write, path::Path, time::Duration};
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
        rows: vec![vec!["0".into(), "2".into()], vec!["5".into(), "3".into()]],
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
    tectonic::compile_figure(t, &tex, true, TIMEOUT).unwrap_or_else(|error| {
        let log = fs::read_to_string(tex.with_extension("log"))
            .unwrap_or_else(|e| format!("could not read figure log: {e}"));
        panic!("{error:?}\nfigure.log:\n{log}")
    })
}

fn preset() -> Preset {
    Preset {
        id: "test".into(),
        width_mm: 90.0,
        height_mm: 65.0,
        font_family: "Arial".into(),
        font_files: vec!["absent.ttf".into()],
        cjk_font_family: Some("Noto Sans CJK SC".into()),
        cjk_font_files: vec!["absent-cjk.ttf".into()],
        font_pt: 8.0,
        min_font_pt: 6.0,
        line_pt: 0.5,
        palette: vec!["0072B2".into()],
        raster_dpi: 300,
    }
}
fn fixed() -> FixedMeasure {
    FixedMeasure { em_ratio: 0.5 }
}
struct Missing(Vec<char>);
impl TextMeasure for Missing {
    fn measure(&self, text: &str, family: &str, pt: f64) -> Res<(f64, f64)> {
        fixed().measure(text, family, pt)
    }
    fn covers(&self, _: &str, _: &str) -> Res<Vec<char>> {
        Ok(self.0.clone())
    }
}
fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/paper/render")
        .join(name)
}
fn layout_fixture(name: &str) -> FigureLayout {
    serde_json::from_slice(&fs::read(fixture(name)).unwrap()).unwrap()
}
fn one(issues: &[AuditIssue], kind: IssueKind, target: &str) {
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].kind, kind);
    assert!(issues[0].target.starts_with(target), "{issues:?}");
    assert!(!issues[0].detail.trim().is_empty(), "{issues:?}");
}
#[test]
fn good_layout_is_clean() {
    assert!(audit_figure(&layout_fixture("good_layout.json"), &preset(), &fixed()).is_empty());
}
#[test]
fn figure_fixture_negative_controls() {
    for (file, kind) in [
        ("overlap_legend.json", IssueKind::Overlap),
        ("oob_label.json", IssueKind::OutOfCanvas),
        ("small_font.json", IssueKind::FontTooSmall),
        ("wrong_unit.json", IssueKind::MissingUnit),
    ] {
        one(
            &audit_figure(&layout_fixture(file), &preset(), &fixed()),
            kind,
            "fig:",
        );
    }
}
#[test]
fn figure_glyph_coverage() {
    let l = layout_fixture("cjk_text.json");
    let p = preset();
    let empty = tempfile::tempdir().unwrap();
    let m = FontMeasure::with_dirs(&p, vec![empty.path().into()]);
    one(&audit_figure(&l, &p, &m), IssueKind::FontMissing, "fig:");
    one(
        &audit_figure(&l, &p, &Missing(vec!['中'])),
        IssueKind::MissingGlyph,
        "fig:",
    );
    assert!(audit_figure(&l, &p, &fixed()).is_empty());
}
#[test]
fn metadata_and_zero_area_boxes_are_not_overlaps() {
    let mut l = layout_fixture("good_layout.json");
    l.texts.push(TextBox {
        role: TextRole::Annotation,
        text: "axiom:plot:metadata".into(),
        x_mm: -1.0,
        y_mm: -1.0,
        w_mm: 20.0,
        h_mm: 20.0,
        font_pt: 1.0,
        family: "absent".into(),
    });
    l.texts.push(TextBox {
        role: TextRole::Annotation,
        text: "".into(),
        x_mm: 6.0,
        y_mm: 6.0,
        w_mm: 0.0,
        h_mm: 0.0,
        font_pt: 8.0,
        family: "Arial".into(),
    });
    assert!(audit_figure(&l, &preset(), &fixed()).is_empty());
}
#[test]
fn table_audit_negative_controls_and_booktabs() {
    let mut t = RenderedTable {
        id: "table".into(),
        latex: String::new(),
        cells: vec![],
        headers: vec!["Time [ms]".into()],
        units: vec![Some("ms".into())],
        width_mm: 101.0,
    };
    one(&audit_table(&t, 100.0), IssueKind::TableTooWide, "tab:");
    t.width_mm = 50.0;
    t.headers[0] = "Time [s]".into();
    one(&audit_table(&t, 100.0), IssueKind::MissingUnit, "tab:");
    let ms = manuscript();
    let table = tables::booktabs(&ms.tables[0], &data(), 160.0, &fixed(), "Arial").unwrap();
    assert!(audit_table(&table, 160.0).is_empty());
}
#[test]
fn all_plot_kinds_and_chinese_title_are_clean() {
    let p = preset();
    let mut d = data();
    d.columns.push("err".into());
    d.units.push(None);
    for row in &mut d.rows {
        row.push("0.1".into());
    }
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::ErrorBar,
        PlotKind::Bar,
    ] {
        let mut spec = manuscript().figures.remove(0);
        spec.kind = kind.clone();
        if kind == PlotKind::ErrorBar {
            spec.error = "err".into();
        }
        let l = plot::layout(&spec, &d, &p, &fixed()).unwrap();
        assert!(
            audit_figure(&l, &p, &fixed()).is_empty(),
            "{kind:?}: {:?}",
            audit_figure(&l, &p, &fixed())
        );
    }
    let mut l = plot::layout(&manuscript().figures[0], &d, &p, &fixed()).unwrap();
    l.texts.push(TextBox {
        role: TextRole::Title,
        text: "中文標題".into(),
        x_mm: 35.0,
        y_mm: 60.0,
        w_mm: 10.0,
        h_mm: 2.0,
        font_pt: 8.0,
        family: "Noto Sans CJK SC".into(),
    });
    assert!(audit_figure(&l, &p, &fixed()).is_empty());
}

// Generate complete PDFs with actual byte offsets and stream lengths.
fn table_pdf(n: u32, image_size: Option<u32>) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()];
    let kids = (0..n)
        .map(|i| format!("{} 0 R", 3 + i))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!("<< /Type /Pages /Count {n} /Kids [{kids}] >>").into_bytes());
    for _ in 0..n {
        let page = if image_size.is_some() {
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>", 3+n, 4+n)
        } else {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> >>".into()
        };
        objects.push(page.into_bytes());
    }
    if let Some(size) = image_size {
        objects.push(b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n\xff\xff\xff\nendstream".to_vec());
        let content = format!("q {size} 0 0 {size} 0 0 cm /Im0 Do Q\n");
        let compressed = zlib(content.as_bytes());
        let mut stream = format!(
            "<< /Filter /FlateDecode /Length {} >>\nstream\n",
            compressed.len()
        )
        .into_bytes();
        stream.extend(compressed);
        stream.extend(b"\nendstream");
        objects.push(stream);
    }
    let mut pdf = b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(obj);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    pdf
}
fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}
fn object_stream_pdf(n: u32) -> Vec<u8> {
    // Catalog and page-tree nodes are normal objects; all page objects live in object 3.
    let kids = (0..n)
        .map(|i| format!("{} 0 R", i + 4))
        .collect::<Vec<_>>()
        .join(" ");
    let mut header = String::new();
    let mut bodies = String::new();
    for i in 0..n {
        header.push_str(&format!("{} {} ", i + 4, bodies.len()));
        bodies
            .push_str("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> >> ");
    }
    let first = header.len();
    let compressed = zlib(format!("{header}{bodies}").as_bytes());
    let mut pdf = b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![0usize];
    for (id, body) in [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (
            2,
            format!("<< /Type /Pages /Count {n} /Kids [{kids}] >>").into_bytes(),
        ),
        (3, {
            let mut b = format!("<< /Type /ObjStm /N {n} /First {first} /Filter /FlateDecode /Length {} >>\nstream\n", compressed.len()).into_bytes();
            b.extend(&compressed);
            b.extend(b"\nendstream");
            b
        }),
    ] {
        offsets.push(pdf.len());
        pdf.extend(format!("{id} 0 obj\n").as_bytes());
        pdf.extend(body);
        pdf.extend(b"\nendobj\n");
    }
    let xref_obj = n + 4;
    let xref_offset = pdf.len();
    let mut entries = Vec::new();
    fn entry(out: &mut Vec<u8>, kind: u8, a: u32, b: u16) {
        out.push(kind);
        out.extend(a.to_be_bytes());
        out.extend(b.to_be_bytes());
    }
    entry(&mut entries, 0, 0, 65535);
    for off in offsets.iter().skip(1) {
        entry(&mut entries, 1, *off as u32, 0);
    }
    for i in 0..n {
        entry(&mut entries, 2, 3, i as u16);
    }
    entry(&mut entries, 1, xref_offset as u32, 0);
    let compressed = zlib(&entries);
    pdf.extend(format!("{xref_obj} 0 obj\n<< /Type /XRef /Size {} /Root 1 0 R /W [1 4 2] /Filter /FlateDecode /Length {} >>\nstream\n", xref_obj+1, compressed.len()).as_bytes());
    pdf.extend(compressed);
    pdf.extend(format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes());
    pdf
}
fn write_pdf(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}
#[test]
fn pdf_xref_table_page_count() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_pdf(dir.path(), "table.pdf", &table_pdf(5, None));
    let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
    assert_eq!(r.pages, 5);
    assert!(r.issues.is_empty());
    assert!(r.vector_figures);
}
#[test]
fn pdf_xref_stream_and_object_stream_page_count() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_pdf(dir.path(), "stream.pdf", &object_stream_pdf(4));
    let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
    assert_eq!(r.pages, 4);
    assert!(r.issues.is_empty());
    assert!(r.vector_figures);
}
#[test]
fn pdf_full_page_image_and_small_image() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_pdf(dir.path(), "full.pdf", &table_pdf(1, Some(100)));
    let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
    assert_eq!(r.pages, 1);
    one(&r.issues, IssueKind::PageRaster, "page:1");
    assert!(!r.vector_figures);
    let p = write_pdf(dir.path(), "small.pdf", &table_pdf(1, Some(20)));
    let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
    assert_eq!(r.pages, 1);
    assert!(r.issues.is_empty());
    assert!(r.vector_figures);
}
#[test]
fn pdf_page_count_range() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_pdf(dir.path(), "five.pdf", &table_pdf(5, None));
    let r = check_pdf(&p, &fixture("clean.log"), Some(2..=3)).unwrap();
    assert_eq!(r.pages, 5);
    one(&r.issues, IssueKind::PageCount, "page:5");
}
#[test]
fn unreadable_pdf_never_passes() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = table_pdf(2, None);
    let truncated = write_pdf(dir.path(), "truncated.pdf", &bytes[..bytes.len() / 2]);
    for p in [
        fixture("garbage.pdf"),
        dir.path().join("missing.pdf"),
        truncated,
    ] {
        let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
        assert_eq!(r.pages, 0);
        one(&r.issues, IssueKind::Unreadable, "page:0");
        assert!(!r.vector_figures);
    }
}
#[test]
fn pdf_missing_character_log() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_pdf(dir.path(), "one.pdf", &table_pdf(1, None));
    let r = check_pdf(&p, &fixture("missing_char.log"), None).unwrap();
    assert_eq!(r.issues.len(), 2);
    let lines = fs::read_to_string(fixture("missing_char.log")).unwrap();
    let missing_lines: Vec<_> = lines
        .lines()
        .filter(|s| s.contains("Missing character:"))
        .collect();
    for (issue, line) in r.issues.iter().zip(missing_lines) {
        assert_eq!(issue.kind, IssueKind::MissingCharacter);
        assert_eq!(issue.target, "page:0");
        assert!(issue.detail.contains(line));
    }
    let r = check_pdf(&p, &fixture("clean.log"), None).unwrap();
    assert!(r.issues.is_empty());
    let r = check_pdf(&p, &dir.path().join("missing.log"), None).unwrap();
    assert!(r.issues.is_empty());
}
#[test]
#[cfg_attr(
    not(paper_tectonic),
    ignore = "no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH"
)]
fn real_vector_figure_and_manuscript() {
    let dir = tempfile::tempdir().unwrap();
    let ms = manuscript();
    let engine = tectonic::find_for_tests().expect("build probe found engine");
    let fig = vector_figure(dir.path(), &ms, &engine);
    let fig_log = dir.path().join("figure.log");
    let r = check_pdf(&fig, &fig_log, Some(1..=1)).unwrap();
    assert_eq!(r.pages, 1);
    assert!(r.issues.is_empty(), "{r:?}");
    assert!(r.vector_figures);
    one(
        &check_pdf(&fig, &fig_log, Some(2..=3)).unwrap().issues,
        IssueKind::PageCount,
        "page:1",
    );
    let project = latex::assemble(
        dir.path(),
        &ms,
        &bundle(),
        &entry("elsarticle"),
        &[("plot".into(), fig)],
        &[rendered(&ms)],
        &bib(),
        None,
    )
    .unwrap();
    let compiled = tectonic::compile(&engine, &project.main, true, TIMEOUT).unwrap();
    assert!(compiled.ok, "{compiled:?}");
    let pdf = compiled.pdf.unwrap();
    let r = check_pdf(&pdf, &compiled.log, None).unwrap();
    assert!(r.pages >= 1);
    assert!(
        !r.issues.iter().any(|i| i.kind == IssueKind::PageRaster),
        "{r:?}"
    );
    assert!(r.vector_figures);
    assert!(check_pdf(&pdf, &compiled.log, Some(r.pages..=r.pages))
        .unwrap()
        .issues
        .is_empty());
    one(
        &check_pdf(&pdf, &compiled.log, Some(r.pages + 1..=r.pages + 2))
            .unwrap()
            .issues,
        IssueKind::PageCount,
        &format!("page:{}", r.pages),
    );
}
#[test]
fn documentation_records_quality_for_each_render_kind() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/paper-figures.md");
    let doc = fs::read_to_string(path).expect("paper figure quality documentation");
    assert!(doc.contains("TikZ") && doc.contains("Tectonic"));
    assert!(
        doc.lines().any(|line| {
            let s = line.to_ascii_lowercase();
            s.contains("plotters")
                && (s.contains("not used") || s.contains("not use") || s.contains("unused"))
        }),
        "Plotters non-use statement missing"
    );
    let cells = |line: &str| -> Vec<String> {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_owned())
            .collect()
    };
    let quality_column = doc
        .lines()
        .filter(|line| line.contains('|'))
        .find_map(|line| {
            cells(line)
                .iter()
                .position(|cell| cell.to_ascii_lowercase().contains("quality"))
        });
    for kind in ["line", "scatter", "error_bar", "bar", "booktabs", "cjk"] {
        assert!(
            doc.lines().any(|line| {
                let row = cells(line);
                let quality_column = quality_column.unwrap_or_else(|| row.len().saturating_sub(1));
                quality_column != 0
                    && row.first().is_some_and(|s| s.eq_ignore_ascii_case(kind))
                    && row.get(quality_column).is_some_and(|s| !s.is_empty())
            }),
            "missing quality row for {kind}"
        );
    }
}
