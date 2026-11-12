use crate::{
    base::Res,
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        journals::openalex::WorkMeta,
    },
};

pub fn work(
    _cache: &Cache,
    _t: &dyn Transport,
    _mode: FetchMode,
    _doi: &str,
) -> Res<Option<WorkMeta>> {
    todo!()
}
