use crate::base::{Failure, Res};
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
    #[derive(Deserialize)]
    struct Presets {
        presets: Vec<Preset>,
    }
    let asset = crate::engine::asset("paper/presets/journals.json")
        .ok_or_else(|| Failure::new("preset_asset", "journal presets asset is missing"))?;
    serde_json::from_str::<Presets>(asset)
        .map(|data| data.presets)
        .map_err(|error| Failure::new("preset_asset", error.to_string()))
}
pub fn preset(id: &str) -> Res<Preset> {
    presets()?
        .into_iter()
        .find(|preset| preset.id == id)
        .ok_or_else(|| Failure::new("unknown_preset", format!("unknown preset: {id}")))
}
