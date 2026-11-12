use crate::{
    base::Res,
    paper::{
        cache::Fetched,
        model::{SourceKind, SourceTag},
    },
};
use std::path::Path;
pub fn tag(
    _kind: SourceKind,
    _fetched: &Fetched,
    _url: &str,
    _pointer: &str,
    _data_year: Option<i32>,
) -> SourceTag {
    unimplemented!()
}
pub fn resolve(_cache_root: &Path, _tag: &SourceTag) -> Res<serde_json::Value> {
    unimplemented!()
}
