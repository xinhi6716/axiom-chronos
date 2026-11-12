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
