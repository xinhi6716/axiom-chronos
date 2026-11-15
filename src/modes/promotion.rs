//! Promote a transcript snapshot into a child run.
use super::{
    launch::{LaunchKind, LaunchRequest, Launched, Launcher},
    providers::Member,
    store::Transcript,
    Run, Session, Snapshot, Source,
};
use crate::base::{Failure, Res};
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Promotion {
    pub from_run: String,
    pub child: Launched,
    pub snapshot_seq: u64,
    pub sources: Vec<Source>,
}

pub fn promote(
    home: &Path,
    session: &Session,
    from_run: &Run,
    kind: LaunchKind,
    goal: &str,
    verify: Option<&str>,
    snapshot: Snapshot,
    launcher: &dyn Launcher,
) -> Res<Promotion> {
    if kind == LaunchKind::Quick
        && (session.scope.is_empty() || verify.map_or(true, |v| v.trim().is_empty()))
    {
        return Err(Failure::new(
            "invalid_argument",
            "quick requires scope and verification",
        ));
    }
    let sources: Vec<Source> = snapshot
        .messages
        .iter()
        .filter(|m| {
            m.author == "person" || Member::ALL.iter().any(|member| m.author == member.name())
        })
        .map(|m| Source {
            session: session.id.clone(),
            seq: m.seq,
            author: m.author.clone(),
        })
        .collect();
    let snapshot_seq = snapshot.upto_seq;
    let request = LaunchRequest {
        kind,
        session: session.id.clone(),
        parent_run: from_run.id.clone(),
        goal: goal.into(),
        scope: session.scope.clone(),
        verify: verify.map(str::to_owned),
        review: false,
        context: Some(snapshot),
        sources: sources.clone(),
    };
    let child = launcher.launch(&request)?;
    let kind_name = match kind {
        LaunchKind::Quick => "quick",
        LaunchKind::Wish => "wish",
    };
    Transcript::open(home, &session.id)?.append(
        "person",
        "promotion",
        &format!("Promoted to {kind_name}: {goal}"),
        None,
        json!({"kind": kind_name, "child_run": child.run.id, "snapshot_seq": snapshot_seq}),
    )?;
    Ok(Promotion {
        from_run: from_run.id.clone(),
        child,
        snapshot_seq,
        sources,
    })
}
