pub mod ort;
pub mod tokenizer;
pub mod infer;
pub mod router;
pub mod curate;
pub mod train;
pub mod shadow;
pub mod pipeline;

use std::path::{Path, PathBuf};

pub fn home_from_env() -> Option<PathBuf> {
    crate::base::data_home().ok()
}

pub fn laya_dir(home: &Path) -> PathBuf {
    home.join("laya")
}
