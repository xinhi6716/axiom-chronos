use crate::{
    base::{atomic_write, Failure, Res},
    paper::PassId,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const DEFAULT_ASTRA_MODEL: &str = "gpt-6-astra";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PassModels {
    pub astra_draft: String,
    pub fable_revise: String,
    pub gemini_revise: String,
    pub fable_style: String,
}
impl Default for PassModels {
    fn default() -> Self {
        Self {
            astra_draft: DEFAULT_ASTRA_MODEL.into(),
            fable_revise: crate::council::FABLE_MODEL.into(),
            gemini_revise: crate::usage::pools::HYDROGEN_XHIGH.into(),
            fable_style: crate::council::FABLE_MODEL.into(),
        }
    }
}
impl PassModels {
    pub fn get(&self, pass: PassId) -> &str {
        match pass {
            PassId::AstraDraft => &self.astra_draft,
            PassId::FableRevise => &self.fable_revise,
            PassId::GeminiRevise => &self.gemini_revise,
            PassId::FableStyle => &self.fable_style,
        }
    }
    pub(crate) fn display(&self) -> String {
        PassId::ALL
            .into_iter()
            .map(|p| format!("{:?}={}", p, self.get(p)))
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub fn validate(&self) -> Res<()> {
        for pass in PassId::ALL {
            let model = self.get(pass);
            if model.is_empty() || model.chars().any(char::is_whitespace) {
                return Err(Failure::new(
                    "invalid_model",
                    format!("invalid model for {pass:?}"),
                ));
            }
        }
        Ok(())
    }
}
pub fn load(home: &Path) -> Res<PassModels> {
    let path = home.join("settings/paper-models.json");
    let m = match fs::read(path) {
        Ok(data) => {
            serde_json::from_slice(&data).map_err(|e| Failure::new("json", e.to_string()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PassModels::default(),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    m.validate()?;
    Ok(m)
}
pub fn save(home: &Path, m: &PassModels) -> Res<()> {
    m.validate()?;
    let dir = home.join("settings");
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| Failure::new("json", e.to_string()))?;
    atomic_write(&dir.join("paper-models.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}
pub fn estimate_models() -> Vec<String> {
    ["astra", "fable", "gemini"]
        .into_iter()
        .map(String::from)
        .collect()
}
