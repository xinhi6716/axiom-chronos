use axiom::base::Res;
use axiom::paper::{
    evidence::DataTable,
    figures::{
        fonts::{FixedMeasure, FontMeasure, TextMeasure},
        layout::{AxisInfo, FigureLayout, TextBox, TextRole},
        plot,
        presets::{self, Preset},
        tikz,
    },
    model::*,
    tables,
};
use std::path::Path;

fn preset() -> Preset {
    Preset {
        id: "elsevier-single".into(),
        width_mm: 90.0,
        height_mm: 65.0,
        font_family: "Test Sans".into(),
        font_files: vec![],
        cjk_font_family: Some("Noto Sans CJK SC".into()),
        cjk_font_files: vec![],
        font_pt: 8.0,
        min_font_pt: 6.0,
        line_pt: 0.5,
        palette: ["E69F00", "56B4E9", "009E73", "0072B2"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        raster_dpi: 300,
    }
}
fn measure() -> FixedMeasure {
    FixedMeasure { em_ratio: 0.5 }
}
fn caption() -> Paragraph {
    Paragraph {
        id: "caption".into(),
        text: "Example".into(),
        claims: vec![],
    }
}
fn spec(kind: PlotKind) -> FigureSpec {
    FigureSpec {
        id: "fig".into(),
        kind,
        artifact: "data.csv".into(),
        x: "time".into(),
        y: vec!["value".into(), "other".into()],
        error: String::new(),
        preset: "elsevier-single".into(),
        caption: caption(),
    }
}
fn data() -> DataTable {
    DataTable {
        columns: vec![
            "time".into(),
            "value".into(),
            "other".into(),
            "err".into(),
            "category".into(),
        ],
        units: vec![
            Some("s".into()),
            Some("m".into()),
            Some("m".into()),
            Some("m".into()),
            None,
        ],
        rows: vec![
            vec!["0", "1.5", "3", "0.1", "A"],
            vec!["2", "-2", "4.25", "0.2", "B"],
            vec!["5", "8", "0", "0.3", "C"],
        ]
        .into_iter()
        .map(|r| r.into_iter().map(String::from).collect())
        .collect(),
    }
}
fn table_spec() -> TableSpec {
    TableSpec {
        id: "table".into(),
        artifact: "table.csv".into(),
        columns: vec![
            TableColumn {
                column: "value".into(),
                header: "value".into(),
                precision: 2,
            },
            TableColumn {
                column: "count".into(),
                header: "count".into(),
                precision: 0,
            },
            TableColumn {
                column: "text".into(),
                header: "text".into(),
                precision: 2,
            },
        ],
        caption: caption(),
    }
}
fn table_data() -> DataTable {
    DataTable {
        columns: vec!["value".into(), "count".into(), "text".into()],
        units: vec![Some("m".into()), None, None],
        rows: vec![vec![
            "3.14159".into(),
            "2.5".into(),
            "& % $ # _ { } ~ ^ \\".into(),
        ]],
    }
}
fn blank_layout() -> FigureLayout {
    FigureLayout {
        id: "fig".into(),
        width_mm: 90.0,
        height_mm: 65.0,
        texts: vec![],
        axes: vec![],
        series: vec![],
        errors: vec![],
        paths: 0,
    }
}
fn box_at(x: f64, y: f64, font_pt: f64) -> TextBox {
    TextBox {
        role: TextRole::AxisLabel,
        text: "x".into(),
        x_mm: x,
        y_mm: y,
        w_mm: 5.0,
        h_mm: 5.0,
        font_pt,
        family: "Test Sans".into(),
    }
}
fn audit(layout: &FigureLayout, p: &Preset) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in layout.texts.iter().enumerate() {
        if a.x_mm < 0.0
            || a.y_mm < 0.0
            || a.x_mm + a.w_mm > layout.width_mm
            || a.y_mm + a.h_mm > layout.height_mm
        {
            out.push("outside".into());
        }
        if a.font_pt < p.min_font_pt {
            out.push("font".into());
        }
        for b in &layout.texts[i + 1..] {
            if a.w_mm > 0.0
                && a.h_mm > 0.0
                && b.w_mm > 0.0
                && b.h_mm > 0.0
                && a.x_mm < b.x_mm + b.w_mm
                && b.x_mm < a.x_mm + a.w_mm
                && a.y_mm < b.y_mm + b.h_mm
                && b.y_mm < a.y_mm + a.h_mm
            {
                out.push("overlap".into());
            }
        }
    }
    for a in &layout.axes {
        if let Some(unit) = &a.data_unit {
            if !a.label.contains(unit) {
                out.push("unit".into());
            }
        }
    }
    out
}
fn has_raster(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    [
        "\\includegraphics",
        "\\pgfimage",
        "\\pgfuseimage",
        ".png",
        ".jpg",
        ".jpeg",
        ".bmp",
    ]
    .iter()
    .any(|x| s.contains(x))
}

#[test]
fn line_scatter_errorbar_values_match_data() {
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::ErrorBar,
        PlotKind::Bar,
    ] {
        let mut s = spec(kind.clone());
        if kind == PlotKind::ErrorBar {
            s.error = "err".into();
            s.y.truncate(1);
        }
        if kind == PlotKind::Bar {
            s.x = "category".into();
        }
        let l = plot::layout(&s, &data(), &preset(), &measure()).unwrap();
        assert_eq!(l.series.len(), s.y.len());
        let xs = if kind == PlotKind::Bar {
            vec![0.0, 1.0, 2.0]
        } else {
            vec![0.0, 2.0, 5.0]
        };
        assert_eq!(
            l.series[0],
            (
                "value".into(),
                xs.iter().copied().zip([1.5, -2.0, 8.0]).collect()
            )
        );
        if kind != PlotKind::ErrorBar {
            assert_eq!(
                l.series[1],
                (
                    "other".into(),
                    xs.iter().copied().zip([3.0, 4.25, 0.0]).collect()
                )
            );
        }
        if kind == PlotKind::ErrorBar {
            assert_eq!(
                l.errors,
                vec![(0.0, 1.5, 0.1), (2.0, -2.0, 0.2), (5.0, 8.0, 0.3)]
            );
        }
        assert!(l.paths >= l.series.len());
    }
    let mut missing = spec(PlotKind::Line);
    missing.x = "absent".into();
    assert_eq!(
        plot::layout(&missing, &data(), &preset(), &measure())
            .unwrap_err()
            .kind,
        "missing_column"
    );
    let mut invalid = data();
    invalid.rows[0][1] = "not-a-number".into();
    assert_eq!(
        plot::layout(&spec(PlotKind::Line), &invalid, &preset(), &measure())
            .unwrap_err()
            .kind,
        "bad_value"
    );
}
#[test]
fn axis_labels_carry_units() {
    let l = plot::layout(&spec(PlotKind::Line), &data(), &preset(), &measure()).unwrap();
    assert_eq!(
        l.axes[0],
        AxisInfo {
            label: "time [s]".into(),
            column: "time".into(),
            data_unit: Some("s".into())
        }
    );
    assert_eq!(
        l.axes[1],
        AxisInfo {
            label: "value, other [m]".into(),
            column: "value, other".into(),
            data_unit: Some("m".into())
        }
    );
    let mut d = data();
    d.units[2] = None;
    let l = plot::layout(&spec(PlotKind::Line), &d, &preset(), &measure()).unwrap();
    assert_eq!(l.axes[1].label, "value, other");
    assert_eq!(l.axes[1].data_unit, None);
    let mut s = spec(PlotKind::Bar);
    s.x = "category".into();
    let l = plot::layout(&s, &d, &preset(), &measure()).unwrap();
    assert_eq!(
        l.axes[0],
        AxisInfo {
            label: "category".into(),
            column: "category".into(),
            data_unit: None
        }
    );
}
#[test]
fn physical_size_matches_preset() {
    let p = preset();
    let l = plot::layout(&spec(PlotKind::Line), &data(), &p, &measure()).unwrap();
    assert_eq!((l.width_mm, l.height_mm), (p.width_mm, p.height_mm));
    let t = tikz::standalone(&l, &p);
    assert!(t.lines().any(|line| line.trim()
        == format!(
            "\\useasboundingbox (0,0) rectangle ({},{});",
            p.width_mm, p.height_mm
        )));
    let options = t
        .lines()
        .find(|line| line.contains("\\begin{tikzpicture}"))
        .unwrap()
        .replace(' ', "");
    assert!(options.contains("x=1mm") && options.contains("y=1mm"));
}
#[test]
fn booktabs_table_rules_units_precision() {
    let s = table_spec();
    let d = table_data();
    let m = measure();
    let t = tables::booktabs(&s, &d, 100.0, &m, "Test Sans").unwrap();
    for rule in ["\\toprule", "\\midrule", "\\bottomrule"] {
        assert_eq!(t.latex.matches(rule).count(), 1);
    }
    assert!(t.latex.contains("value [m]"));
    assert!(t.latex.contains("3.14"));
    assert!(t.latex.contains(&format!("{:.0}", 2.5_f64)));
    assert_eq!(
        t.cells[0],
        vec!["3.14", &format!("{:.0}", 2.5_f64), "& % $ # _ { } ~ ^ \\"]
    );
    for escaped in [
        "\\&",
        "\\%",
        "\\$",
        "\\#",
        "\\_",
        "\\{",
        "\\}",
        "\\textasciitilde",
        "\\textasciicircum",
        "\\textbackslash",
    ] {
        assert!(t.latex.contains(escaped), "missing {escaped}: {}", t.latex);
    }
    let expected: f64 = ["value [m]", "count", "& % $ # _ { } ~ ^ \\"]
        .iter()
        .map(|x| m.measure(x, "Test Sans", 8.0).unwrap().0 + 2.0 * 2.1167)
        .sum();
    assert!(
        t.width_mm > 0.0 && (t.width_mm - expected).abs() < 1e-8,
        "{} != {}",
        t.width_mm,
        expected
    );
    assert_eq!(
        tables::booktabs(&s, &d, expected - 0.01, &m, "Test Sans")
            .unwrap_err()
            .kind,
        "table_too_wide"
    );
    let mut bad = s.clone();
    bad.columns[0].column = "absent".into();
    assert_eq!(
        tables::booktabs(&bad, &d, 100.0, &m, "Test Sans")
            .unwrap_err()
            .kind,
        "missing_column"
    );
}
struct Missing;
impl TextMeasure for Missing {
    fn measure(&self, text: &str, family: &str, pt: f64) -> Res<(f64, f64)> {
        measure().measure(text, family, pt)
    }
    fn covers(&self, _text: &str, _family: &str) -> Res<Vec<char>> {
        Ok(vec!['X'])
    }
}
#[test]
fn cjk_text_in_tikz_uses_cjk_family() {
    let mut d = data();
    d.columns[0] = "时间".into();
    let mut s = spec(PlotKind::Line);
    s.x = "时间".into();
    let p = preset();
    let l = plot::layout(&s, &d, &p, &measure()).unwrap();
    assert!(l
        .texts
        .iter()
        .any(|b| b.text.contains("时间") && b.family == "Noto Sans CJK SC"));
    let t = tikz::standalone(&l, &p);
    for x in ["xeCJK", "\\setCJKmainfont{Noto Sans CJK SC}", "fontspec"] {
        assert!(t.contains(x));
    }
    let ascii = plot::layout(&spec(PlotKind::Line), &data(), &p, &measure()).unwrap();
    assert!(!tikz::standalone(&ascii, &p).contains("xeCJK"));
    let mut no_cjk = p.clone();
    no_cjk.cjk_font_family = None;
    assert_eq!(
        plot::layout(&s, &d, &no_cjk, &measure()).unwrap_err().kind,
        "glyph_missing"
    );
    assert_eq!(
        plot::layout(&spec(PlotKind::Line), &data(), &p, &Missing)
            .unwrap_err()
            .kind,
        "glyph_missing"
    );
}
#[test]
fn tikz_has_no_raster() {
    assert!(has_raster("\\includegraphics{plot.pdf}"));
    let p = preset();
    let l = plot::layout(&spec(PlotKind::Line), &data(), &p, &measure()).unwrap();
    let t = tikz::standalone(&l, &p);
    assert!(
        t.lines().next().unwrap().contains("\\documentclass")
            && t.lines().next().unwrap().contains("standalone")
    );
    assert!(!has_raster(&t));
    assert!(t.matches("\\draw").count() >= l.series.len());
}
#[test]
fn presets_palette_colour_blind_safe() {
    let ok = [
        "E69F00", "56B4E9", "009E73", "F0E442", "0072B2", "D55E00", "CC79A7", "000000",
    ];
    let all = presets::presets().unwrap();
    for (id, width) in [
        ("elsevier-single", Some(90.0)),
        ("elsevier-double", Some(190.0)),
        ("ieee-single", Some(88.9)),
        ("ieee-double", Some(181.9)),
        ("springer-single", None),
        ("acs-single", None),
    ] {
        let p = all.iter().find(|p| p.id == id).unwrap();
        if let Some(w) = width {
            assert_eq!(p.width_mm, w);
        }
        let colors: std::collections::HashSet<_> = p
            .palette
            .iter()
            .map(|c| c.trim_start_matches('#').to_ascii_uppercase())
            .collect();
        assert!(colors.len() >= 4 && colors.iter().all(|c| ok.contains(&c.as_str())));
        assert!(p.font_pt >= p.min_font_pt && p.min_font_pt >= 6.0 && p.raster_dpi >= 300);
    }
    assert_eq!(presets::preset("nope").unwrap_err().kind, "unknown_preset");
    let mut value = serde_json::to_value(preset()).unwrap();
    value.as_object_mut().unwrap().remove("min_font_pt");
    assert_eq!(
        serde_json::from_value::<Preset>(value).unwrap().min_font_pt,
        6.0
    );
}
#[test]
fn quality_checks_per_plot_type() {
    let p = preset();
    let mut bad = blank_layout();
    bad.texts = vec![
        box_at(1.0, 1.0, 8.0),
        box_at(2.0, 2.0, 8.0),
        box_at(89.0, 10.0, 8.0),
        box_at(10.0, 10.0, 5.0),
    ];
    bad.axes.push(AxisInfo {
        label: "time".into(),
        column: "time".into(),
        data_unit: Some("s".into()),
    });
    let violations = audit(&bad, &p);
    for x in ["overlap", "outside", "font", "unit"] {
        assert!(violations.iter().any(|v| v == x), "{x}");
    }
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::ErrorBar,
        PlotKind::Bar,
    ] {
        let mut s = spec(kind.clone());
        if kind == PlotKind::ErrorBar {
            s.error = "err".into();
            s.y.truncate(1);
        }
        if kind == PlotKind::Bar {
            s.x = "category".into();
        }
        let l = plot::layout(&s, &data(), &p, &measure()).unwrap();
        assert!(audit(&l, &p).is_empty(), "{:?}", audit(&l, &p));
    }
    let mut small = p.clone();
    small.font_pt = 5.0;
    assert_eq!(
        plot::layout(&spec(PlotKind::Line), &data(), &small, &measure())
            .unwrap_err()
            .kind,
        "font_too_small"
    );
}
#[test]
fn fonts_missing_font_kind() {
    let mut p = preset();
    p.font_files = vec!["nonexistent-paper-font-18f7.ttf".into()];
    let m = FontMeasure::with_dirs(&p, vec![]);
    assert_eq!(
        m.measure("A", &p.font_family, 8.0).unwrap_err().kind,
        "font_missing"
    );
    assert_eq!(
        m.covers("A", &p.font_family).unwrap_err().kind,
        "font_missing"
    );
}
#[test]
fn fonts_measure_real_font_if_present() {
    let paths = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/Library/Fonts/Arial.ttf",
    ];
    let Some(path) = paths.iter().find(|path| Path::new(path).is_file()) else {
        eprintln!("No candidate real font available; skipping font metric checks");
        return;
    };
    let mut p = preset();
    p.font_files = vec![(*path).into()];
    let m = FontMeasure::new(&p);
    let i = m.measure("iiii", &p.font_family, 8.0).unwrap();
    let wide = m.measure("MMMM", &p.font_family, 8.0).unwrap();
    assert!(i.0 < wide.0 && i.1 > 0.0);
    assert!(m.covers("A", &p.font_family).unwrap().is_empty());
    assert!(!m.covers("\u{10FFFF}", &p.font_family).unwrap().is_empty());
}


fn copied_system_font() -> Option<(tempfile::TempDir, String)> {
    let paths = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/Library/Fonts/Arial.ttf",
    ];
    let Some(path) = paths.iter().map(Path::new).find(|path| path.is_file()) else {
        eprintln!("No candidate real font available; skipping font lookup checks");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let name = path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    std::fs::copy(path, dir.path().join(&name)).unwrap();
    Some((dir, name))
}

#[test]
fn fonts_lookup_cjk_whitespace_and_cache() {
    let Some((dir, name)) = copied_system_font() else {
        return;
    };
    let mut p = preset();
    p.font_files = vec![name.to_ascii_lowercase()];
    p.cjk_font_files = p.font_files.clone();
    let m = FontMeasure::with_dirs(&p, vec![dir.path().into()]);
    assert!(m.measure("Hello", &p.font_family, 8.0).unwrap().0 > 0.0);
    assert_eq!(
        m.measure("a", "unknown family", 8.0).unwrap_err().kind,
        "font_missing"
    );
    assert!(m.covers("a b\t\n", &p.font_family).unwrap().is_empty());
    let cjk = p.cjk_font_family.as_deref().unwrap();
    assert!(m.measure("a", cjk, 8.0).unwrap().0 > 0.0);
    std::fs::remove_file(dir.path().join(name)).unwrap();
    assert!(m.covers("a", &p.font_family).unwrap().is_empty());
    assert!(m.measure("a", cjk, 8.0).unwrap().0 > 0.0);
}

#[test]
fn fonts_directory_listing_is_cached() {
    let Some((dir, name)) = copied_system_font() else {
        return;
    };
    let mut p = preset();
    p.font_files = vec![name.to_ascii_lowercase()];
    p.cjk_font_files = vec!["later.ttf".into()];
    let m = FontMeasure::with_dirs(&p, vec![dir.path().into()]);
    m.measure("a", &p.font_family, 8.0).unwrap();
    std::fs::copy(dir.path().join(name), dir.path().join("LATER.TTF")).unwrap();
    assert_eq!(
        m.measure("a", p.cjk_font_family.as_deref().unwrap(), 8.0)
            .unwrap_err()
            .kind,
        "font_missing"
    );
}

#[test]
fn fonts_coverage_unions_multiple_files() {
    let Some((dir, name)) = copied_system_font() else {
        return;
    };
    let mut bytes = std::fs::read(dir.path().join(&name)).unwrap();
    let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let cmap_offset = (0..tables)
        .find_map(|i| {
            let record = 12 + i * 16;
            (&bytes[record..record + 4] == b"cmap").then(|| {
                u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize
            })
        })
        .unwrap();
    // Keep a valid font but remove every character mapping from its cmap table.
    bytes[cmap_offset + 2..cmap_offset + 4].copy_from_slice(&0u16.to_be_bytes());
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    assert!(face.glyph_index('a').is_none());
    std::fs::write(dir.path().join("EMPTY.TTF"), bytes).unwrap();
    let mut p = preset();
    p.font_files = vec!["empty.ttf".into(), name.to_ascii_lowercase()];
    let m = FontMeasure::with_dirs(&p, vec![dir.path().into()]);
    assert_eq!(
        m.covers("a\u{10ffff}", &p.font_family).unwrap(),
        vec!['\u{10ffff}']
    );
}

#[test]
fn tikz_draw_count_equals_paths() {
    let p = preset();
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::ErrorBar,
        PlotKind::Bar,
    ] {
        let mut s = spec(kind.clone());
        if kind == PlotKind::ErrorBar {
            s.error = "err".into();
            s.y.truncate(1);
        }
        if kind == PlotKind::Bar {
            s.x = "category".into();
        }
        let layout = plot::layout(&s, &data(), &p, &measure()).unwrap();
        assert_eq!(
            tikz::standalone(&layout, &p)
                .lines()
                .filter(|l| l.starts_with("\\draw"))
                .count(),
            layout.paths
        );
    }
    let layout = blank_layout();
    assert_eq!(layout.path_count(), 0);
    assert_eq!(
        tikz::standalone(&layout, &p)
            .lines()
            .filter(|l| l.starts_with("\\draw"))
            .count(),
        layout.paths
    );
}

#[test]
fn table_non_finite_cells_kept() {
    let mut d = table_data();
    d.rows = vec![
        vec!["nan".into(), "inf".into(), "-inf".into()],
        vec!["NaN".into(), "+inf".into(), "1.25".into()],
    ];
    let result = tables::booktabs(&table_spec(), &d, 100.0, &measure(), "Test Sans").unwrap();
    assert_eq!(
        result.cells,
        vec![vec!["nan", "inf", "-inf"], vec!["NaN", "+inf", "1.25"]]
    );
}
