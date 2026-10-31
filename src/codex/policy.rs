use crate::base::{Failure, Res};
fn invalid(s: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", s)
}
pub(crate) fn role(name: &str) -> Res<(&'static str, &'static str)> {
    match name {
        "core" => Ok(("astra", "medium")),
        "astra" => Ok(("astra", "max")),
        "architect" => Ok(("astra", "high")),
        "astra-verify" | "tester" => Ok(("astra", "high")),
        "dispatcher" => Ok(("terra", "high")),
        "reviewer" | "critic" => Ok(("sol", "high")),
        "memory-keeper" => Ok(("terra", "medium")),
        "content" => Ok(("luna", "low")),
        _ => Err(invalid(format!("unknown chronos agent: {name}"))),
    }
}
pub(crate) fn worker_effort(model: &str) -> &'static str {
    if crate::codex::role_matches("astra", model) {
        role("astra").unwrap().1
    } else if crate::codex::role_matches("luna", model) {
        role("content").unwrap().1
    } else if crate::codex::role_matches("terra", model) {
        role("dispatcher").unwrap().1
    } else {
        role("reviewer").unwrap().1
    }
}
