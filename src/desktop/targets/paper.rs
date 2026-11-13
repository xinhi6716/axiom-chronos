use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("paper", "paper", Opening::Page("paper")),
 entry("paper.form", "paper", Opening::PaperStep("form")),
 entry("paper.shortlist", "paper", Opening::PaperStep("shortlist")),
 entry("paper.estimate", "paper", Opening::PaperStep("estimate")),
 entry("paper.progress", "paper", Opening::PaperStep("progress")),
 entry("paper.changelog", "paper", Opening::PaperStep("changelog")),
 entry("paper.preview", "paper", Opening::PaperStep("preview")),
 entry("paper.readiness", "paper", Opening::PaperStep("readiness"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
