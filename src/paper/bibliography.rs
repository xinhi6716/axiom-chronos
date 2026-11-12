use crate::{
    base::Res,
    paper::{journals::openalex::WorkMeta, model::ReferenceSeed},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BibEntry {
    pub key: String,
    pub kind: String,
    pub doi: Option<String>,
    pub openalex: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub article_number: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bibliography {
    pub entries: Vec<BibEntry>,
}

pub fn parse_bibtex(_input: &str) -> Res<Bibliography> {
    todo!()
}
pub fn to_bibtex(_bib: &Bibliography) -> String {
    todo!()
}
pub fn from_seeds(
    _seeds: &[ReferenceSeed],
    _lookup: &dyn Fn(&ReferenceSeed) -> Res<Option<WorkMeta>>,
) -> Bibliography {
    todo!()
}
