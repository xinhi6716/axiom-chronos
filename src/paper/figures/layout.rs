use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TextRole {
    Title,
    AxisLabel,
    Tick,
    Legend,
    Annotation,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBox {
    pub role: TextRole,
    pub text: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub w_mm: f64,
    pub h_mm: f64,
    pub font_pt: f64,
    pub family: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisInfo {
    pub label: String,
    pub column: String,
    pub data_unit: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FigureLayout {
    pub id: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub texts: Vec<TextBox>,
    pub axes: Vec<AxisInfo>,
    pub series: Vec<(String, Vec<(f64, f64)>)>,
    pub errors: Vec<(f64, f64, f64)>,
    pub paths: usize,
}

/// Plot geometry travels in a zero-area `TextRole::Annotation` box prefixed
/// `axiom:plot:` to preserve serialization and existing public struct literals
/// without presenting the metadata as visible or overlapping text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlotGeometry {
    pub kind: crate::paper::model::PlotKind,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
    pub xmin: f64,
    pub xmax: f64,
    pub ymin: f64,
    pub ymax: f64,
    pub xticks: Vec<f64>,
    pub yticks: Vec<f64>,
}
const META: &str = "axiom:plot:";
impl FigureLayout {
    pub(crate) fn geometry(&self) -> Option<PlotGeometry> {
        self.texts.iter().find_map(|t| {
            (t.role == TextRole::Annotation)
                .then(|| t.text.strip_prefix(META))
                .flatten()
                .and_then(|s| serde_json::from_str(s).ok())
        })
    }
    pub(crate) fn set_geometry(&mut self, geometry: &PlotGeometry, font_pt: f64, family: &str) {
        self.texts.push(TextBox {
            role: TextRole::Annotation,
            text: format!(
                "{META}{}",
                serde_json::to_string(geometry).expect("finite geometry")
            ),
            x_mm: 0.0,
            y_mm: 0.0,
            w_mm: 0.0,
            h_mm: 0.0,
            font_pt,
            family: family.into(),
        });
    }
    pub fn path_count(&self) -> usize {
        use crate::paper::model::PlotKind;
        let Some(g) = self.geometry() else { return 0 };
        let plots = match g.kind {
            PlotKind::Line => self.series.len(),
            PlotKind::Scatter | PlotKind::ErrorBar | PlotKind::Bar => {
                self.series.iter().map(|(_, p)| p.len()).sum()
            }
        };
        2 + g.xticks.len() + g.yticks.len() + plots + self.errors.len()
    }
}
pub(crate) fn escape_latex(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(ch);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\\' => out.push_str("\\textbackslash{}"),
            _ => out.push(ch),
        }
    }
    out
}
