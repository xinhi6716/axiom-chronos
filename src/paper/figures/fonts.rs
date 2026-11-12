use super::presets::Preset;
use crate::base::Res;
use std::path::PathBuf;

pub trait TextMeasure: Send + Sync {
    fn measure(&self, text: &str, family: &str, pt: f64) -> Res<(f64, f64)>;
    fn covers(&self, text: &str, family: &str) -> Res<Vec<char>>;
}
#[derive(Debug, Clone, PartialEq)]
pub struct FontMeasure {
    _preset: Preset,
    _dirs: Vec<PathBuf>,
}
impl FontMeasure {
    pub fn new(_preset: &Preset) -> Self {
        unimplemented!()
    }
    pub fn with_dirs(_preset: &Preset, _dirs: Vec<PathBuf>) -> Self {
        unimplemented!()
    }
}
impl TextMeasure for FontMeasure {
    fn measure(&self, _text: &str, _family: &str, _pt: f64) -> Res<(f64, f64)> {
        unimplemented!()
    }
    fn covers(&self, _text: &str, _family: &str) -> Res<Vec<char>> {
        unimplemented!()
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct FixedMeasure {
    pub em_ratio: f64,
}
impl TextMeasure for FixedMeasure {
    fn measure(&self, text: &str, _family: &str, pt: f64) -> Res<(f64, f64)> {
        Ok((
            text.chars().count() as f64 * pt * self.em_ratio * 25.4 / 72.0,
            pt * 25.4 / 72.0,
        ))
    }
    fn covers(&self, _text: &str, _family: &str) -> Res<Vec<char>> {
        Ok(vec![])
    }
}
