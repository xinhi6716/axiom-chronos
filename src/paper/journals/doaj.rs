use crate::{
    base::Res,
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        model::{Access, Apc, Known, Sourced},
    },
};
#[derive(Debug)]
pub struct DoajRecord {
    pub apc: Known<Apc>,
    pub access: Sourced<Access>,
    pub review_weeks: Known<u32>,
    pub homepage: Known<String>,
    pub ai_policy: Known<String>,
}
pub fn journal(
    _cache: &Cache,
    _t: &dyn Transport,
    _mode: FetchMode,
    _issn: &str,
) -> Res<Option<DoajRecord>> {
    unimplemented!()
}
