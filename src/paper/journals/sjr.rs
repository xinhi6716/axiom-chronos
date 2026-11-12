use crate::{
    base::Res,
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        model::{QuartileValue, Sourced},
    },
};
pub struct SjrTable;
#[derive(Debug)]
pub struct SjrRow {
    pub sjr: Sourced<f64>,
    pub quartiles: Vec<Sourced<QuartileValue>>,
    pub publisher: Option<Sourced<String>>,
    pub open_access: Option<Sourced<bool>>,
}
pub fn load_year(
    _cache: &Cache,
    _t: &dyn Transport,
    _mode: FetchMode,
    _year: i32,
) -> Res<SjrTable> {
    unimplemented!()
}
impl SjrTable {
    pub fn lookup(&self, _issns: &[String]) -> Option<SjrRow> {
        unimplemented!()
    }
}
