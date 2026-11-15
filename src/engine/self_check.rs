use crate::{
    base::{Failure, Res},
    codex::{AppServer, RunRequest},
    engine::policy,
    orch,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: String,
    pub reference: String,
    pub detail: String,
    pub fixed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfCheck {
    pub status: String,
    pub changed: bool,
    pub fixed: u32,
    pub findings: Vec<Finding>,
    pub thread_id: String,
    pub tree: String,
}

pub fn run(
    codex: &AppServer,
    home: Option<&Path>,
    store: &Path,
    unit: &str,
    attempt_id: &str,
    repo: &Path,
) -> Res<SelfCheck> {
    let brief = orch::unit_brief(store, unit)?;
    let settings = policy::effective_models(home, store)?.models.self_check;
    let model = codex.resolve_model(&settings.tier)?;
    let all = vec![".".to_string()];
    let before = orch::snapshot(repo, &all);
    if before.is_empty() {
        return Err(Failure::new("selfcheck", "cannot snapshot repository"));
    }
    let diff = crate::base::proc::cmd("git")
        .current_dir(repo)
        .args(["diff", "HEAD", &before[5..], "--"])
        .output()
        .map_err(|e| Failure::new("selfcheck", e.to_string()))?;
    if !diff.status.success() {
        return Err(Failure::new("selfcheck", "cannot read diff"));
    }
    let schema = json!({"title":"axiom.self_check.v1","type":"object","additionalProperties":false,"required":["status","changed","fixed","findings"],"properties":{"status":{"type":"string","enum":["pass","fail"]},"changed":{"type":"boolean"},"fixed":{"type":"integer","minimum":0},"findings":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["kind","reference","detail","fixed"],"properties":{"kind":{"type":"string","enum":["untested","vacuous_check","contract_mismatch"]},"reference":{"type":"string"},"detail":{"type":"string"},"fixed":{"type":"boolean"}}}}}});
    let req = RunRequest {
        model: model.clone(),
        effort: Some(settings.effort.clone()),
        sandbox: "workspace-write".into(),
        output_schema: Some(schema),
        brief: format!(
            "{}\n{}\nDiff:\n{}",
            super::asset("roles/self-check.md").unwrap(),
            brief.text,
            String::from_utf8_lossy(&diff.stdout)
        ),
        cwd: repo.to_string_lossy().into_owned(),
        lean: true,
        web_search: false,
        keep_plugins: vec![],
        developer_instructions: None,
        timeout_s: 1800,
    };
    let report = codex.run(&req)?;
    if report["status"] != "completed" {
        return Err(Failure::new("selfcheck", report["error"].to_string()));
    }
    #[derive(Deserialize)]
    struct Answer {
        status: String,
        changed: bool,
        fixed: u32,
        findings: Vec<Finding>,
    }
    let answer: Answer = serde_json::from_value(report["structured_output"].clone())
        .map_err(|e| Failure::new("selfcheck", e.to_string()))?;
    let after = orch::snapshot(repo, &all);
    if after.is_empty() {
        return Err(Failure::new(
            "selfcheck",
            "cannot snapshot final repository",
        ));
    }
    let out = crate::base::proc::cmd("git")
        .current_dir(repo)
        .args([
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            "-z",
            &before[5..],
            &after[5..],
        ])
        .output()
        .map_err(|e| Failure::new("selfcheck", e.to_string()))?;
    if !out.status.success() {
        return Err(Failure::new("selfcheck", "cannot compare trees"));
    }
    let paths: Vec<_> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    let outside = paths.iter().any(|p| !orch::parse::covers(&brief.scope, p));
    let tree = orch::snapshot(repo, &brief.scope);
    if tree.is_empty() {
        return Err(Failure::new("selfcheck", "cannot snapshot scope"));
    }
    let result = SelfCheck {
        status: if outside || answer.status != "pass" || answer.findings.iter().any(|f| !f.fixed) {
            "fail".into()
        } else {
            "pass".into()
        },
        changed: answer.changed || !paths.is_empty(),
        fixed: answer.fixed,
        findings: answer.findings,
        thread_id: report["thread_id"].as_str().unwrap_or_default().into(),
        tree,
    };
    let data = json!({"unit":unit,"attempt_id":attempt_id,"tree":result.tree,"model":model,"effort":settings.effort,"thread_id":result.thread_id,"status":result.status,"changed":result.changed,"fixed":result.fixed,"findings":result.findings});
    let dir = store.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    crate::base::atomic_write(
        &dir.join(format!("{unit}.selfcheck.json")),
        &serde_json::to_vec_pretty(&data).map_err(|e| Failure::new("selfcheck", e.to_string()))?,
    )
    .map_err(|e| Failure::new("io", e.to_string()))?;
    orch::append_evidence(
        store,
        unit,
        &orch::Evidence {
            kind: "selfcheck".into(),
            command: format!("{attempt_id}|engine-selfcheck:{model}"),
            exit_code: if result.status == "pass" { 0 } else { 1 },
            commit: result.tree.clone(),
            ts: crate::base::now_iso(),
        },
    )?;
    Ok(result)
}
