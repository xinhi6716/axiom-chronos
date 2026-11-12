use crate::{
    base::Res,
    paper::{
        http::{Pause, Transport},
        model::SourceKind,
    },
    seams::Clock,
};
use std::{path::Path, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchMode {
    Online,
    OfflineOnly,
}
#[derive(Debug)]
pub struct Fetched {
    pub body: Vec<u8>,
    pub locator: String,
    pub fetched_at: String,
    pub from_cache: bool,
    pub stale: bool,
}
pub struct Cache;
impl Cache {
    pub fn open(_root: &Path, _clock: Arc<dyn Clock>, _pause: Arc<dyn Pause>) -> Res<Self> {
        unimplemented!()
    }
    pub fn get(
        &self,
        _kind: SourceKind,
        _url: &str,
        _mode: FetchMode,
        _t: &dyn Transport,
    ) -> Res<Fetched> {
        unimplemented!()
    }
}
