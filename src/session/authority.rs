//! Caller authority and durable intent review. Identity is supplied by Host,
//! never by a model's `actor` argument.
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Research, ResearchRunning, IntentReview, Planning, PlanningRunning, Execution, ReadySubmit, Submitted }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub revision: u64,
    pub research: Option<Value>,
    pub reason: String,
    #[serde(default)]
    pub owner_pid: Option<u32>,
    #[serde(default)]
    pub plan_revision:u64,
    #[serde(default)]
    pub plan_hash:Option<String>,
    #[serde(default)]
    pub submitted_source:Option<String>,
    #[serde(default)]
    pub delivery_paths:Vec<String>,
    #[serde(default)]
    pub delivery_written:Vec<String>,
    #[serde(default)]
    pub submitted_evidence:Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self { phase: Phase::Research, revision: 0, research: None, reason: String::new(), owner_pid: None,plan_revision:0,plan_hash:None,submitted_source:None,delivery_paths:vec![],delivery_written:vec![],submitted_evidence:None }
    }
}
/// Called only while Attachment holds the exclusive caller-command OS lock.
/// A dead PID alone is insufficient: that lock must also exclude the old host.
/// Only host-mediated research/planning are replayable; engineering is never reset.
pub fn recover_abandoned_stage(state: &mut State) -> Res<()> {
    if !matches!(state.phase, Phase::ResearchRunning | Phase::PlanningRunning) { return Ok(()); }
    let owner = state.owner_pid.ok_or_else(|| Failure::new("recovery_required", "legacy running checkpoint has no owner evidence; automatic restart refused"))?;
    if owner == 0 || crate::base::proc::pid_alive(owner) {
        return Err(Failure::new("busy", "specialist owner is alive; do not redispatch"));
    }
    state.phase = if state.phase == Phase::ResearchRunning { Phase::Research } else { Phase::Planning };
    state.owner_pid = None;
    state.reason = format!("Recovered abandoned host-mediated stage: owner {owner} exited and caller-command OS lock acquired; engineering untouched");
    Ok(())
}
fn denied(message: &str) -> Failure { Failure::new("permission_denied", message) }
fn directory(home: &Path, session: &str) -> Res<PathBuf> {
    base::require_slug("session", session)?;
    Ok(home.join("authority").join(session))
}
pub fn read(home: &Path, session: &str) -> Res<State> {
    let path = directory(home, session)?.join("state.json");
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| denied(&format!("invalid authority state: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if directory(home,session)?.join("enabled").exists() { return Err(denied("authority checkpoint is missing; refusing to restart workflow")); }
            Ok(State::default())
        }
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}
pub fn update<T>(home: &Path, session: &str, f: impl FnOnce(&mut State) -> Res<T>) -> Res<T> {
    let dir = directory(home, session)?;
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    crate::orch::with_lock(&dir, true, || {
        let mut state = read(home, session)?;
        let result = f(&mut state)?;
        let bytes = serde_json::to_vec_pretty(&state).map_err(|e| Failure::new("io", e.to_string()))?;
        base::atomic_write(&dir.join("state.json"), &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
        base::atomic_write(&dir.join("enabled"), b"1\n").map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(result)
    })
}
pub fn guard(state: &State, caller: &str, tool: &str, args: &Value) -> Res<()> {
    if caller == "core" {
        match tool {
            "workflow" | "goal" => return Ok(()),
            "Agent" if args["subagent_type"] == "chronos:dispatcher"
                && matches!(state.phase, Phase::Research | Phase::Planning | Phase::Execution | Phase::Submitted) => return Ok(()),
            _ => return Err(denied("core may relay intent to dispatcher, judge intent compatibility, and submit only; engineering and direct specialist calls are forbidden")),
        }
    }
    if tool == "Agent" {
        if caller != "dispatcher" { return Err(denied("only dispatcher routes specialists")); }
        let child = args["subagent_type"].as_str().unwrap_or("");
        let allowed = match child {
            "chronos:astra" => state.phase == Phase::Research,
            "chronos:architect" => state.phase == Phase::Planning,
            "chronos:reviewer" | "chronos:critic" | "chronos:tester" => state.phase == Phase::Execution,
            "chronos:content" => matches!(state.phase, Phase::Research | Phase::Planning | Phase::Execution | Phase::Submitted),
            "chronos:memory-keeper" => matches!(state.phase, Phase::Research | Phase::Planning | Phase::Execution),
            _ => false,
        };
        if !allowed { return Err(denied("specialist is forbidden in the current workflow phase")); }
    }
    let measurement = args["role"] == "astra-verify" && args["task_kind"] == "measurement" && args["sandbox"] == "read-only";
    if tool == "codex_run" && (caller != "dispatcher" || !(state.phase == Phase::Execution || (measurement && state.phase == Phase::Planning))) {
        return Err(denied("engineering requires approved research and completed Astra HIGH planning"));
    }
    if caller == "dispatcher" && matches!(tool, "orch" | "run_on_linux" | "review_code")
        && state.phase != Phase::Execution {
        let read_only = tool == "orch" && args["args"].as_array().is_some_and(|args| {
            args.iter().filter_map(Value::as_str).filter(|s| !s.starts_with("--"))
                .next().is_some_and(|s| matches!(s,"init" | "status" | "ready" | "evidence"))
        });
        if !read_only { return Err(denied("engineering commands require completed planning")); }
    }
    if caller == "dispatcher" && matches!(state.phase, Phase::IntentReview | Phase::ReadySubmit | Phase::Submitted)
        && !matches!(tool, "workflow" | "Read" | "Glob" | "Grep") {
        if state.phase==Phase::Submitted && tool=="Agent" && args["subagent_type"]=="chronos:content" {return Ok(());}
        return Err(denied("workflow is waiting at the core boundary; no further dispatch or mutation"));
    }
    Ok(())
}
/// Submitted code evidence is not relabelled as coverage for later documents.
/// Only exact Markdown outputs authorized by core may differ at delivery.
pub(crate) fn check_delivery(home:&Path,session:&str,repo:&Path)->Res<()> {
    let state=read(home,session)?;
    if state.phase!=Phase::Submitted {return Err(denied("round not submitted"));}
    let goal=crate::goal::load(home,session)?.ok_or_else(||denied("missing goal"))?;
    let store=crate::orch::store_for(home,goal.project.as_deref().ok_or_else(||denied("missing project"))?)?;
    if state.submitted_evidence.as_deref()!=Some(evidence_identity(&store)?.as_str()) {return Err(denied("submitted evidence changed"));}
    let before=state.submitted_source.as_deref().and_then(|s|s.strip_prefix("tree:")).ok_or_else(||denied("missing submitted source identity"))?;
    let current=crate::orch::snapshot(repo,&[".".into()]);
    let after=current.strip_prefix("tree:").ok_or_else(||denied("missing delivery source"))?;
    let output=std::process::Command::new("git").current_dir(repo).args(["diff","--name-only","-z",before,after,"--"]).output().map_err(|e|Failure::new("io",e.to_string()))?;
    if !output.status.success() {return Err(denied("cannot verify delivery diff"));}
    for path in output.stdout.split(|b|*b==0).filter(|b|!b.is_empty()) {
        let path=std::str::from_utf8(path).map_err(|_|denied("invalid delivery path"))?;
        let resolved=super::tools::resolved(&repo.join(path))?;
        if !state.delivery_paths.iter().any(|allowed|Path::new(allowed)==resolved) {return Err(denied("product source changed after submission; documents cannot bypass engineering acceptance"));}
    }
    Ok(())
}
pub(crate) fn check_delivery_complete(home:&Path,session:&str,repo:&Path)->Res<()> {
    check_delivery(home,session,repo)?;
    let state=read(home,session)?;
    for path in &state.delivery_paths {
        if !state.delivery_written.contains(path) || !std::fs::metadata(path).is_ok_and(|m|m.is_file() && m.len()>0) {
            return Err(Failure::new("delivery_incomplete",format!("authorized document has no confirmed nonempty Write: {path}")));
        }
    }
    Ok(())
}
pub(super) fn evidence_identity(store:&Path)->Res<String> {
    use sha2::{Digest,Sha256};
    let mut files=std::fs::read_dir(store).map_err(|e|Failure::new("io",e.to_string()))?.filter_map(Result::ok)
        .map(|e|e.path()).filter(|p|p.is_file() && p.file_name().and_then(|s|s.to_str()).is_some_and(|s|s.ends_with(".evidence.tsv") || ["modules.md","preflight.json","units.tsv","core-wait.json","core-wait.v1.json","wait-required"].contains(&s))).collect::<Vec<_>>();
    files.sort();let mut hash=Sha256::new();
    for file in files {hash.update(file.file_name().unwrap().to_string_lossy().as_bytes());hash.update(std::fs::read(file).map_err(|e|Failure::new("io",e.to_string()))?);}
    Ok(format!("{:x}",hash.finalize()))
}
pub fn check_research(state: &State) -> Res<()> {
    use sha2::{Digest, Sha256};
    let r=state.research.as_ref().ok_or_else(|| denied("missing approved research"))?;
    let path=r["artifact"].as_str().ok_or_else(|| denied("missing research artifact"))?;
    let bytes=std::fs::read(path).map_err(|e| Failure::new("io",e.to_string()))?;
    if r["sha256"] != format!("{:x}",Sha256::digest(bytes)) {
        return Err(denied("research changed after submission; a new intent review is required"));
    }
    Ok(())
}
pub fn verdict(state: &mut State, caller: &str, args: &Value) -> Res<Value> {
    if caller != "core" || state.phase != Phase::IntentReview || args["revision"].as_u64() != Some(state.revision) {
        return Err(denied("intent verdict requires core and the current research revision"));
    }
    let reason = args["reason"].as_str().unwrap_or("");
    if reason.chars().count() > 160 { return Err(denied("intent verdict reason must be at most 160 characters")); }
    match args["decision"].as_str() {
        Some("pass") if reason.is_empty() => { state.phase = Phase::Planning; state.reason.clear(); }
        Some("reject") if !reason.trim().is_empty() => { state.phase = Phase::Research; state.reason = reason.into(); }
        _ => return Err(denied("return pass only, or reject with a short intent-conflict reason")),
    }
    Ok(json!({"status":"completed","phase":state.phase,"revision":state.revision}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_allows_only_authorized_docs_and_sealed_evidence() {
        let t=tempfile::tempdir().unwrap();let repo=t.path().join("repo");let home=t.path().join("home");
        std::fs::create_dir(&repo).unwrap();
        let git=|args:&[&str]| {assert!(std::process::Command::new("git").current_dir(&repo).args(args).output().unwrap().status.success());};
        git(&["init"]);git(&["config","user.email","test@example.invalid"]);git(&["config","user.name","Test"]);
        std::fs::write(repo.join("code.rs"),"original").unwrap();std::fs::write(repo.join("manual.md"),"old").unwrap();git(&["add","."]);git(&["commit","-m","fixture"]);
        crate::goal::open(&home,"delivery","tiny",repo.to_str()).unwrap();let g=crate::goal::bind_session(&home,"delivery").unwrap();
        let store=crate::orch::store_for(&home,g.project.as_deref().unwrap()).unwrap();std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("test.evidence.tsv"),"sealed").unwrap();
        update(&home,"delivery",|s| {s.phase=Phase::Submitted;s.submitted_source=Some(crate::orch::snapshot(&repo,&[".".into()]));s.submitted_evidence=Some(evidence_identity(&store)?);s.delivery_paths=vec![super::super::tools::resolved(&repo.join("manual.md"))?.to_string_lossy().into_owned()];Ok(())}).unwrap();
        std::fs::write(repo.join("manual.md"),"release result").unwrap();check_delivery(&home,"delivery",&repo).unwrap();
        // Existing files and text-only model promises cannot satisfy delivery.
        assert_eq!(check_delivery_complete(&home,"delivery",&repo).unwrap_err().kind,"delivery_incomplete");
        update(&home,"delivery",|s| {s.delivery_written=s.delivery_paths.clone();Ok(())}).unwrap();
        check_delivery_complete(&home,"delivery",&repo).unwrap();
        std::fs::write(repo.join("manual.md"),"").unwrap();assert!(check_delivery_complete(&home,"delivery",&repo).is_err());
        std::fs::write(repo.join("manual.md"),"release result").unwrap();
        std::fs::write(repo.join("code.rs"),"changed").unwrap();assert!(check_delivery(&home,"delivery",&repo).is_err());
        std::fs::write(repo.join("code.rs"),"original").unwrap();std::fs::write(store.join("test.evidence.tsv"),"forged").unwrap();assert!(check_delivery(&home,"delivery",&repo).is_err());
    }
    #[test]
    fn recovery_requires_dead_owner_and_only_resets_pre_engineering() {
        let mut s=State {phase:Phase::ResearchRunning,owner_pid:Some(std::process::id()),..State::default()};
        assert_eq!(recover_abandoned_stage(&mut s).unwrap_err().kind,"busy");
        s.owner_pid=None;
        assert_eq!(recover_abandoned_stage(&mut s).unwrap_err().kind,"recovery_required");
        let mut child=std::process::Command::new(if cfg!(windows) {"cmd"} else {"sh"})
            .args(if cfg!(windows) {["/c","exit 0"]} else {["-c","exit 0"]}).spawn().unwrap();
        let pid=child.id();child.wait().unwrap();
        for (from,to) in [(Phase::ResearchRunning,Phase::Research),(Phase::PlanningRunning,Phase::Planning)] {
            s.phase=from;s.owner_pid=Some(pid);
            recover_abandoned_stage(&mut s).unwrap();assert_eq!(s.phase,to);
        }
        s.phase=Phase::Execution;s.owner_pid=Some(pid);
        recover_abandoned_stage(&mut s).unwrap();assert_eq!(s.phase,Phase::Execution);
    }
    #[test]
    fn core_cannot_engineer_or_bypass_dispatcher() {
        for phase in [Phase::Research, Phase::IntentReview, Phase::Planning, Phase::Execution, Phase::ReadySubmit] {
            let state = State { phase, ..State::default() };
            for tool in ["Write", "Bash", "orch", "codex_run", "run_on_linux", "review_code"] {
                assert!(guard(&state,"core",tool,&json!({"actor":"dispatcher"})).is_err());
            }
            assert!(guard(&state,"core","Agent",&json!({"subagent_type":"chronos:astra"})).is_err());
        }
    }
    #[test]
    fn verdict_is_bound_and_rejection_returns_to_research() {
        let mut s = State { phase: Phase::IntentReview, revision: 7, ..State::default() };
        assert!(verdict(&mut s,"dispatcher",&json!({"revision":7,"decision":"pass"})).is_err());
        assert!(verdict(&mut s,"core",&json!({"revision":6,"decision":"pass"})).is_err());
        verdict(&mut s,"core",&json!({"revision":7,"decision":"reject","reason":"違反使用者指定範圍"})).unwrap();
        assert_eq!(s.phase,Phase::Research);
        assert!(guard(&s,"dispatcher","codex_run",&json!({})).is_err());
        s.phase=Phase::IntentReview;
        verdict(&mut s,"core",&json!({"revision":7,"decision":"pass"})).unwrap();
        assert_eq!(s.phase,Phase::Planning);
        assert!(guard(&s,"dispatcher","codex_run",&json!({})).is_err());
        assert!(guard(&s,"dispatcher","Agent",&json!({"subagent_type":"chronos:architect"})).is_ok());
    }
    #[test]
    fn restart_preserves_review_and_corruption_fails_closed() {
        let temp=tempfile::tempdir().unwrap();
        update(temp.path(),"session",|s| { s.phase=Phase::IntentReview;s.revision=3;Ok(()) }).unwrap();
        assert_eq!(read(temp.path(),"session").unwrap().phase,Phase::IntentReview);
        std::fs::write(temp.path().join("authority/session/state.json"),b"broken").unwrap();
        assert!(read(temp.path(),"session").is_err());
    }
    #[test]
    fn baseline_measurement_does_not_need_engineering_release_or_allow_core_work() {
        let request=json!({"role":"astra-verify","task_kind":"measurement","sandbox":"read-only"});
        for phase in [Phase::Planning, Phase::Execution] {
            let state=State { phase, ..State::default() };
            assert!(guard(&state,"dispatcher","codex_run",&request).is_ok());
            assert!(guard(&state,"core","codex_run",&request).is_err());
        }
        for phase in [Phase::Research,Phase::IntentReview,Phase::ReadySubmit,Phase::Submitted] {
            assert!(guard(&State { phase, ..State::default() },"dispatcher","codex_run",&request).is_err());
        }
        assert!(guard(&State {phase:Phase::Planning,..State::default()},"dispatcher","codex_run",&json!({"role":"terra"})).is_err());
    }
}
