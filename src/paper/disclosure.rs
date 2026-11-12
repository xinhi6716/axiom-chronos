use super::{AiPolicy, Disclosure, JournalRecord, ModelUse, PassRecord};
use crate::base::Res;

pub fn policies() -> Res<Vec<AiPolicy>> {
    unimplemented!()
}
pub fn policy_for(_publisher: &str) -> Option<AiPolicy> {
    unimplemented!()
}
pub fn generate(
    _passes: &[PassRecord],
    _reviewer: Option<&ModelUse>,
    _journal: &JournalRecord,
) -> Res<Disclosure> {
    unimplemented!()
}
