use super::digest::{self, Digest, MAX_PROBLEMS};
use std::path::Path;

pub const NO_MERGE_PUSH: &str = "Do not merge or push any changes.";
pub const EVIDENCE_RULE: &str = "For every change, include an evidence -> change line naming the problem id(s) in unit reports and decisions.";

pub fn wish_text(d: &Digest, repo: &Path) -> String {
    let problems = d
        .problems
        .iter()
        .take(MAX_PROBLEMS)
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Fix the top problems in Chronos only at {}. Address these problem IDs: {}.\n\n{}\n{}\n{}",
        repo.display(),
        problems,
        digest::render(d),
        EVIDENCE_RULE,
        NO_MERGE_PUSH
    )
}
