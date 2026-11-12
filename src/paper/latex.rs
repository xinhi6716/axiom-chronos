use crate::{
    base::Res,
    paper::{
        bibliography::Bibliography,
        model::{Disclosure, EvidenceBundle, Manuscript, RenderedTable},
        templates::TemplateEntry,
    },
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LatexProject {
    pub dir: PathBuf,
    pub main: PathBuf,
    pub files: Vec<PathBuf>,
}

pub fn assemble(
    _dir: &Path,
    _ms: &Manuscript,
    _bundle: &EvidenceBundle,
    _template: &TemplateEntry,
    _figures: &[(String, PathBuf)],
    _tables: &[RenderedTable],
    _bib: &Bibliography,
    _disclosure: Option<&Disclosure>,
) -> Res<LatexProject> {
    todo!()
}
