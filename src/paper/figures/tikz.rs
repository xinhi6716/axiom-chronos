use super::{
    layout::{escape_latex, FigureLayout, TextRole},
    presets::Preset,
};
use crate::paper::model::PlotKind;
use std::fmt::Write;

pub fn standalone(layout: &FigureLayout, preset: &Preset) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "\\documentclass[tikz]{{standalone}}\n\\usepackage{{fontspec}}\n\\setmainfont{{{}}}",
        preset.font_family
    )
    .unwrap();
    if let Some(family) = &preset.cjk_font_family {
        if layout.texts.iter().any(|t| {
            &t.family == family
                && !(t.role == TextRole::Annotation && t.text.starts_with("axiom:plot:"))
        }) {
            writeln!(out, "\\usepackage{{xeCJK}}\n\\setCJKmainfont{{{family}}}").unwrap();
        }
    }
    for (i, color) in preset.palette.iter().enumerate() {
        writeln!(
            out,
            "\\definecolor{{p{i}}}{{HTML}}{{{}}}",
            color.trim_start_matches('#')
        )
        .unwrap();
    }
    writeln!(
        out,
        "\\begin{{document}}\n\\begin{{tikzpicture}}[x=1mm,y=1mm,line width={}pt]",
        preset.line_pt
    )
    .unwrap();
    writeln!(
        out,
        "\\useasboundingbox (0,0) rectangle ({},{});",
        layout.width_mm, layout.height_mm
    )
    .unwrap();
    if let Some(g) = layout.geometry() {
        let x = |v: f64| g.left + (v - g.xmin) / (g.xmax - g.xmin) * (g.right - g.left);
        let y = |v: f64| g.bottom + (v - g.ymin) / (g.ymax - g.ymin) * (g.top - g.bottom);
        writeln!(
            out,
            "\\draw ({},{}) -- ({},{});",
            g.left, g.bottom, g.right, g.bottom
        )
        .unwrap();
        writeln!(
            out,
            "\\draw ({},{}) -- ({},{});",
            g.left, g.bottom, g.left, g.top
        )
        .unwrap();
        for v in &g.xticks {
            writeln!(
                out,
                "\\draw ({},{}) -- ({},{});",
                x(*v),
                g.bottom,
                x(*v),
                g.bottom - 0.8
            )
            .unwrap();
        }
        for v in &g.yticks {
            writeln!(
                out,
                "\\draw ({},{}) -- ({},{});",
                g.left,
                y(*v),
                g.left - 0.8,
                y(*v)
            )
            .unwrap();
        }
        for (i, (_, points)) in layout.series.iter().enumerate() {
            let color = if preset.palette.is_empty() {
                "black".to_owned()
            } else {
                format!("p{}", i % preset.palette.len())
            };
            match g.kind {
                PlotKind::Line => {
                    write!(out, "\\draw[{color}]").unwrap();
                    if points.is_empty() {
                        write!(out, " ({},{})", g.left, g.bottom).unwrap();
                    }
                    for (j, (px, py)) in points.iter().enumerate() {
                        write!(
                            out,
                            " {} ({},{})",
                            if j == 0 { "" } else { "--" },
                            x(*px),
                            y(*py)
                        )
                        .unwrap();
                    }
                    writeln!(out, ";").unwrap();
                }
                PlotKind::Scatter | PlotKind::ErrorBar => {
                    for (px, py) in points {
                        writeln!(out, "\\draw[{color}] ({},{}) circle (0.8);", x(*px), y(*py))
                            .unwrap();
                    }
                }
                PlotKind::Bar => {
                    let n = layout.series.len().max(1) as f64;
                    for (px, py) in points {
                        let a = *px - 0.4 + 0.8 * i as f64 / n;
                        let b = a + 0.8 / n;
                        writeln!(
                            out,
                            "\\draw[{color}] ({},{}) rectangle ({},{});",
                            x(a),
                            y(0.0),
                            x(b),
                            y(*py)
                        )
                        .unwrap();
                    }
                }
            }
        }
        let color = if preset.palette.is_empty() {
            "black"
        } else {
            "p0"
        };
        for (px, py, e) in &layout.errors {
            let xx = x(*px);
            let low = y(py - e.abs());
            let high = y(py + e.abs());
            writeln!(out,"\\draw[{color}] ({xx},{low}) -- ({xx},{high}) ({},{low}) -- ({},{low}) ({},{high}) -- ({},{high});",xx-0.8,xx+0.8,xx-0.8,xx+0.8).unwrap();
        }
    }
    let mut axis_index = 0;
    for t in &layout.texts {
        if t.role == TextRole::Annotation && t.text.starts_with("axiom:plot:") {
            continue;
        }
        let rotated = if t.role == TextRole::AxisLabel {
            let r = axis_index == 1;
            axis_index += 1;
            r
        } else {
            false
        };
        let options = if rotated {
            "anchor=south west,inner sep=0,rotate=90"
        } else {
            "anchor=south west,inner sep=0"
        };
        let xpos = if rotated { t.x_mm + t.w_mm } else { t.x_mm };
        writeln!(
            out,
            "\\node[{options}] at ({},{}) {{\\fontsize{{{}pt}}{{{}pt}}\\selectfont {}}};",
            xpos,
            t.y_mm,
            t.font_pt,
            t.font_pt * 1.2,
            escape_latex(&t.text)
        )
        .unwrap();
    }
    out.push_str("\\end{tikzpicture}\n\\end{document}\n");
    out
}
