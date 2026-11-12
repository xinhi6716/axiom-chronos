use crate::{
    base::Res,
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        model::{JournalRecord, SourceTag},
    },
};
use std::path::Path;
#[allow(dead_code)]
pub struct OpenAlex<'a> {
    cache: &'a Cache,
    transport: &'a dyn Transport,
    mode: FetchMode,
    mailto: Option<String>,
}
#[derive(Debug)]
pub struct WorkHit {
    pub id: String,
    pub title: String,
    pub abstract_words: Vec<String>,
    pub year: Option<i32>,
    pub source_id: Option<String>,
    pub topics: Vec<String>,
    pub doi: Option<String>,
}
#[derive(Debug)]
pub struct WorkMeta {
    pub id: String,
    pub doi: Option<String>,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub first_page: Option<String>,
    pub last_page: Option<String>,
    pub article_number: Option<String>,
    pub source: SourceTag,
}
impl<'a> OpenAlex<'a> {
    pub fn new(
        _cache: &'a Cache,
        _transport: &'a dyn Transport,
        _mode: FetchMode,
        _data_home: &Path,
    ) -> Res<Self> {
        unimplemented!()
    }
    pub fn search_works(&self, _query: &str, _from_year: i32, _max: usize) -> Res<Vec<WorkHit>> {
        unimplemented!()
    }
    pub fn source(&self, _id: &str) -> Res<JournalRecord> {
        unimplemented!()
    }
    pub fn work(&self, _id_or_doi: &str) -> Res<Option<WorkMeta>> {
        unimplemented!()
    }
}
