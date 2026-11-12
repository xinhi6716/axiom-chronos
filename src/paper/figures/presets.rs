use crate::base::Res;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub font_family: String,
    pub font_files: Vec<String>,
    pub cjk_font_family: Option<String>,
    pub cjk_font_files: Vec<String>,
    pub font_pt: f64,
    #[serde(default = "default_min_font_pt")]
    pub min_font_pt: f64,
    pub line_pt: f64,
    pub palette: Vec<String>,
    pub raster_dpi: u32,
}
fn default_min_font_pt() -> f64 {
    6.0
}
pub fn presets() -> Res<Vec<Preset>> {
    unimplemented!()
}
pub fn preset(_id: &str) -> Res<Preset> {
    unimplemented!()
}
