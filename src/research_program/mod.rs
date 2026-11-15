pub mod adjudication;
pub mod budget;
pub mod dossier;
pub mod history;
pub mod investigation;
pub mod runner;
pub mod state;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Investigation,
    Dossier,
    Council,
    RedTeam,
    PreMortem,
    Synthesis,
    Adjudication,
}

impl Stage {
    pub const ALL: [Stage; 7] = [
        Self::Investigation,
        Self::Dossier,
        Self::Council,
        Self::RedTeam,
        Self::PreMortem,
        Self::Synthesis,
        Self::Adjudication,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Investigation => "investigation",
            Self::Dossier => "dossier",
            Self::Council => "council",
            Self::RedTeam => "red_team",
            Self::PreMortem => "pre_mortem",
            Self::Synthesis => "synthesis",
            Self::Adjudication => "adjudication",
        }
    }
    pub fn artifact(self) -> &'static str {
        match self {
            Self::Investigation => "01-investigation/",
            Self::Dossier => "02-dossier.json",
            Self::Council => "03-council/",
            Self::RedTeam => "04-red_team.json",
            Self::PreMortem => "05-pre_mortem.json",
            Self::Synthesis => "06-synthesis.json",
            Self::Adjudication => "07-adjudication.json",
        }
    }
}
pub const PROGRAM_TYPE: &str = "wish_program";
pub fn program_dir(store: &Path) -> PathBuf {
    store.join("research").join("program")
}
pub use state::gate;
