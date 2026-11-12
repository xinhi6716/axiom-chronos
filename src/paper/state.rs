use super::model::{PaperState, Step};
use crate::base::Res;
use std::{collections::BTreeMap, path::Path};

pub fn load(_store: &Path) -> Res<Option<PaperState>> {
    unimplemented!()
}
pub fn save(_store: &Path, _state: &PaperState) -> Res<()> {
    unimplemented!()
}
pub fn upstream(_step: Step) -> &'static [Step] {
    unimplemented!()
}
pub fn record(
    _store: &Path,
    _step: Step,
    _inputs: BTreeMap<String, String>,
    _output: &str,
    _ok: bool,
    _detail: Option<String>,
) -> Res<PaperState> {
    unimplemented!()
}
pub fn current(_state: &PaperState, _step: Step) -> bool {
    unimplemented!()
}
pub fn invalidate(_store: &Path, _step: Step) -> Res<PaperState> {
    unimplemented!()
}
