use crate::base::{schema, Failure, Res};
use crate::council::Status;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
    thread,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberCall {
    pub member: String,
    pub stage: String,
    pub prompt: String,
    pub readable: Vec<PathBuf>,
    pub out_dir: PathBuf,
    pub schema: Value,
    pub timeout_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberReply {
    pub status: Status,
    pub output: Option<Value>,
    pub raw: Option<String>,
    pub model: Option<String>,
    pub provider: String,
    pub usd: Option<f64>,
    pub tokens: u64,
    pub detail: Option<String>,
}

pub trait ResearchMember: Sync {
    fn name(&self) -> &str;
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply;
}

pub fn findings_schema() -> Value {
    serde_json::from_str(include_str!("../../engine/research/dossier.schema.json"))
        .expect("embedded findings schema must be valid JSON")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationRequest {
    pub question: String,
    pub sources: Vec<String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberOutcome {
    pub member: String,
    pub status: Status,
    pub findings: Option<PathBuf>,
    pub detail: Option<String>,
    pub reply: MemberReply,
}

fn io_error(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn investigate(
    dir: &Path,
    req: &InvestigationRequest,
    members: &[&dyn ResearchMember],
    stop: &AtomicBool,
) -> Res<Vec<MemberOutcome>> {
    let schema = findings_schema();
    let prompt = format!(
        "Question:\n{}\n\nAllowed sources:\n{}\n\nFindings schema:\n{}\n",
        req.question,
        req.sources.join("\n"),
        schema
    );
    let mut readable = vec![req.cwd.clone()];
    for source in &req.sources {
        let path = Path::new(source);
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(Failure::new(
                "invalid_argument",
                format!("source must be relative to cwd: {source}"),
            ));
        }
        readable.push(req.cwd.join(path));
    }
    let mut names = HashSet::new();
    let mut calls = Vec::with_capacity(members.len());
    for member in members {
        let name = member.name();
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || !names.insert(name.to_lowercase())
        {
            return Err(Failure::new(
                "invalid_argument",
                format!("invalid or duplicate member name: {name}"),
            ));
        }
        let out_dir = dir.join(name);
        fs::create_dir_all(&out_dir).map_err(io_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        }
        fs::write(out_dir.join("prompt.txt"), &prompt).map_err(io_error)?;
        let findings_path = out_dir.join("findings.json");
        match fs::remove_file(findings_path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(io_error(e)),
        }
        calls.push(MemberCall {
            member: name.into(),
            stage: "investigate".into(),
            prompt: prompt.clone(),
            readable: readable.clone(),
            out_dir,
            schema: schema.clone(),
            timeout_s: 900,
        });
    }
    let replies = thread::scope(|scope| {
        let handles: Vec<_> = members
            .iter()
            .zip(&calls)
            .map(|(member, call)| scope.spawn(move || member.call(call, stop)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| Failure::new("member_failed", "research member panicked"))
            })
            .collect::<Res<Vec<_>>>()
    })?;
    let mut outcomes = Vec::with_capacity(calls.len());
    for (call, reply) in calls.into_iter().zip(replies) {
        fs::write(
            call.out_dir.join("raw.txt"),
            reply.raw.as_deref().unwrap_or(""),
        )
        .map_err(io_error)?;
        let mut status = reply.status;
        let mut detail = reply.detail.clone();
        let mut findings = None;
        if status == Status::Ok {
            match reply
                .output
                .as_ref()
                .ok_or_else(|| "missing output".to_string())
                .and_then(|output| schema::validate(&schema, output))
            {
                Ok(()) => {
                    let path = call.out_dir.join("findings.json");
                    let data = serde_json::to_vec_pretty(reply.output.as_ref().unwrap())
                        .map_err(|e| Failure::new("parse", e.to_string()))?;
                    fs::write(&path, data).map_err(io_error)?;
                    findings = Some(path);
                }
                Err(error) => {
                    status = Status::Malformed;
                    detail = Some(error);
                }
            }
        }
        outcomes.push(MemberOutcome {
            member: call.member,
            status,
            findings,
            detail,
            reply,
        });
    }
    if outcomes.iter().filter(|o| o.findings.is_some()).count() < 2 {
        return Err(Failure::new(
            "too_few_members",
            "fewer than two members delivered valid findings",
        ));
    }
    Ok(outcomes)
}
