use super::{
    budgets::{self, Role},
    kinds::prompt_head,
};
use crate::{
    base::{self, Failure, Res},
    codex::{AppServer, RunRequest},
    council,
    usage::price::{self, CodexUsage},
};
use serde_json::Value;
use std::{path::Path, time::Duration};
pub struct EngineCall<'a> {
    pub role: Role,
    pub case: &'a str,
    pub pack: String,
    pub cwd: &'a Path,
    pub schema: Value,
    pub timeout_s: u64,
}
fn codex_call(
    codex: &AppServer,
    store: &Path,
    c: &EngineCall,
    family: &str,
    effort: &str,
) -> Res<Value> {
    budgets::check(store, c.role, c.case)?;
    let model = codex.resolve_model(family)?;
    let req = RunRequest {
        model: model.clone(),
        effort: Some(effort.into()),
        sandbox: "read-only".into(),
        developer_instructions: None,
        lean: true,
        web_search: false,
        output_schema: Some(c.schema.clone()),
        brief: prompt_head(c.role.name(), c.case) + &c.pack,
        cwd: c.cwd.to_string_lossy().into_owned(),
        timeout_s: c.timeout_s,
        keep_plugins: vec![],
    };
    let report = codex.run(&req)?;
    let u = &report["usage"];
    let input = u["inputTokens"].as_u64();
    let cached = u["cachedInputTokens"].as_u64();
    let output = u["outputTokens"].as_u64();
    let usd = match (input, cached, output) {
        (Some(input), Some(cached_input), Some(output)) => price::codex_usd(
            &model,
            &CodexUsage {
                input,
                cached_input,
                output,
            },
        ),
        _ => None,
    };
    budgets::record(
        store,
        c.role,
        &model,
        usd,
        input.unwrap_or(0),
        output.unwrap_or(0),
        c.case,
    )?;
    if report["status"] != "completed" {
        return Err(Failure::new(
            report["error"]["kind"].as_str().unwrap_or("codex_call"),
            report["error"]["message"]
                .as_str()
                .unwrap_or("Codex call failed"),
        ));
    }
    Ok(report["structured_output"].clone())
}
pub fn astra_max(codex: &AppServer, store: &Path, c: &EngineCall) -> Res<Value> {
    codex_call(codex, store, c, "astra", "max")
}
pub fn codex_low(codex: &AppServer, store: &Path, c: &EngineCall) -> Res<Value> {
    codex_call(codex, store, c, "terra", "low")
}
pub fn claude_quick(store: &Path, c: &EngineCall, model: &str) -> Res<Value> {
    budgets::check(store, c.role, c.case)?;
    let command = base::resolve_command("AXIOM_CLAUDE_CMD", "claude");
    let (answer, usage) = council::claude_call(
        &command,
        model,
        prompt_head(c.role.name(), c.case) + &c.pack,
        c.cwd,
        &c.schema,
        Duration::from_secs(c.timeout_s),
    )?;
    budgets::record(
        store,
        c.role,
        &usage.model,
        usage.usd,
        usage.input_tokens,
        usage.output_tokens,
        c.case,
    )?;
    Ok(answer)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::{kinds, policy},
        orch::events,
    };
    use serde_json::json;
    fn python() -> String {
        std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
            let names = if cfg!(windows) {
                ["python", "python3"]
            } else {
                ["python3", "python"]
            };
            names
                .into_iter()
                .find(|p| {
                    crate::base::proc::cmd(p)
                        .arg("--version")
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
                .unwrap()
                .into()
        })
    }
    fn fake(name: &str, script: &Path, log: &Path) -> Vec<String> {
        vec![
            python(),
            "-I".into(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fakes")
                .join(format!("{name}.py"))
                .to_string_lossy()
                .into_owned(),
            format!("--fake-script={}", script.display()),
            format!("--fake-log={}", log.display()),
        ]
    }
    fn lines(log: &Path) -> Vec<Value> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    #[test]
    fn codex_checks_budget_and_starts_fresh_read_only_threads() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let script = p.join("script.json");
        let log = p.join("log.jsonl");
        std::fs::write(
            &script,
            json!([{"match":"AXIOM-ENGINE verifier test","reply":{"answer":"ok"}}]).to_string(),
        )
        .unwrap();
        let server =
            AppServer::new(fake("fake_codex", &script, &log)).with_slots_home(p.join("slots"));
        let c = EngineCall {
            role: Role::Verifier,
            case: "test",
            pack: "evidence".into(),
            cwd: p,
            schema: json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}),
            timeout_s: 10,
        };
        policy::set(p, "budget.verifier", "0").unwrap();
        assert_eq!(
            astra_max(&server, p, &c).unwrap_err().kind,
            "budget_exceeded"
        );
        assert!(lines(&log).is_empty());
        policy::set(p, "budget.verifier", "none").unwrap();
        assert_eq!(astra_max(&server, p, &c).unwrap(), json!({"answer":"ok"}));
        assert_eq!(codex_low(&server, p, &c).unwrap(), json!({"answer":"ok"}));
        let log = lines(&log);
        let starts: Vec<_> = log
            .iter()
            .filter(|x| x["method"] == "thread/start")
            .collect();
        let turns: Vec<_> = log.iter().filter(|x| x["method"] == "turn/start").collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(turns.len(), 2);
        assert_ne!(
            turns[0]["params"]["threadId"],
            turns[1]["params"]["threadId"]
        );
        assert_eq!(starts[0]["params"]["sandbox"], "read-only");
        assert_eq!(starts[1]["params"]["sandbox"], "read-only");
        assert_eq!(turns[0]["params"]["effort"], "max");
        assert_eq!(turns[1]["params"]["effort"], "low");
        assert_eq!(
            turns[0]["params"]["input"][0]["text"],
            "AXIOM-ENGINE verifier test\nevidence"
        );
        let spends: Vec<_> = events::read_events(p, 0)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == kinds::ROLE_SPEND)
            .collect();
        assert_eq!(spends.len(), 2);
        assert_eq!(spends[0].data["model"], "gpt-6-astra");
        assert_eq!(spends[1].data["model"], "gpt-6.10-terra");
        assert!(spends[1].data["usd"].is_null());
    }
    #[test]
    fn claude_checks_budget_and_records_unpriced_usage() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let script = p.join("script.json");
        let log = p.join("log.jsonl");
        std::fs::write(
            &script,
            json!([{"match":"AXIOM-ENGINE reviewer test","reply":{"answer":"ok"}}]).to_string(),
        )
        .unwrap();
        let previous = std::env::var_os("AXIOM_CLAUDE_CMD");
        std::env::set_var(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&fake("fake_member", &script, &log)).unwrap(),
        );
        let c = EngineCall {
            role: Role::Reviewer,
            case: "test",
            pack: "evidence".into(),
            cwd: p,
            schema: json!({"type":"object","required":["answer"]}),
            timeout_s: 10,
        };
        policy::set(p, "budget.reviewer", "0").unwrap();
        assert_eq!(
            claude_quick(p, &c, "claude-experimental-x")
                .unwrap_err()
                .kind,
            "budget_exceeded"
        );
        assert!(lines(&log).is_empty());
        policy::set(p, "budget.reviewer", "none").unwrap();
        assert_eq!(
            claude_quick(p, &c, "claude-experimental-x").unwrap(),
            json!({"answer":"ok"})
        );
        match previous {
            Some(v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
            None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
        };
        let log = lines(&log);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["prompt"], "AXIOM-ENGINE reviewer test\nevidence");
        let rows: Vec<_> = events::read_events(p, 0)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == kinds::ROLE_SPEND)
            .collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data["usd"].is_null());
        assert_eq!(rows[0].data["input_tokens"], 7000);
    }
}
