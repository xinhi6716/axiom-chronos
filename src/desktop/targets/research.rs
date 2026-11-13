use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("research", "research", Opening::Page("research")),
 entry("research.map", "research", Opening::ResearchTab("map")),
 entry("research.sources", "research", Opening::ResearchTab("sources")),
 entry("research.evidence", "research", Opening::ResearchTab("evidence")),
 entry("research.dataset", "research", Opening::ResearchTab("dataset")),
 entry("research.exports", "research", Opening::ResearchTab("exports"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
