use crate::{
    base::{self, Failure, Res},
    codex::{self, AppServer, RunRequest},
    council,
    engine::budgets::{self, Role},
    modes::{
        context::{truncate_utf8, DEFAULT_CONTEXT_BYTES},
        plaza::router::{self, Router, Routing},
        providers::{Ask, Member, Provider, Reply},
        readonly::{ToolRequest, READ_TOOLS},
    },
    usage::pools,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct MemberCommands {
    pub claude: Res<Vec<String>>,
    pub agy: Res<Vec<String>>,
    pub codex: Res<Vec<String>>,
}
impl MemberCommands {
    pub fn from_env(home: &Path) -> Self {
        let codex = match codex::slots::load_runtime(home) {
            Ok(Some((runtime, _))) if !runtime.codex_command.is_empty() => {
                Ok(runtime.codex_command)
            }
            Ok(_) => base::resolve_command("AXIOM_CODEX_CMD", "codex"),
            Err(e) => Err(e),
        };
        Self {
            claude: base::resolve_command("AXIOM_CLAUDE_CMD", "claude"),
            agy: base::resolve_command("AXIOM_AGY_CMD", "agy"),
            codex,
        }
    }
}
pub struct ModelProvider {
    home: PathBuf,
    cmds: MemberCommands,
}
impl ModelProvider {
    pub fn new(home: PathBuf) -> Self {
        let cmds = MemberCommands::from_env(&home);
        Self { home, cmds }
    }
    pub fn with_commands(home: PathBuf, cmds: MemberCommands) -> Self {
        Self { home, cmds }
    }
    fn invoke(
        &self,
        member: Member,
        prompt: String,
        schema: Value,
        role: Role,
        store: &Path,
    ) -> Res<Value> {
        let command = match member {
            Member::Opus | Member::Fable => &self.cmds.claude,
            Member::Argon => &self.cmds.agy,
            Member::Astra => &self.cmds.codex,
        };
        let command = command.as_ref().map_err(|e| {
            Failure::new("model_missing", format!("{}: {}", member.name(), e.message))
        })?;
        if command.is_empty() || command[0].is_empty() {
            return Err(Failure::new(
                "model_missing",
                format!("{} command is empty", member.name()),
            ));
        }
        let executable = Path::new(&command[0]);
        let found = if executable.is_absolute() || executable.components().count() > 1 {
            let path = if executable.is_absolute() {
                executable.to_path_buf()
            } else {
                self.home.join(executable)
            };
            path.is_file()
        } else {
            base::on_path(&command[0]).is_some()
        };
        if !found {
            return Err(Failure::new(
                "model_missing",
                format!("{} command not found: {}", member.name(), command[0]),
            ));
        }
        let model = match member {
            Member::Opus => "opus",
            Member::Fable => council::FABLE_MODEL,
            Member::Argon => council::ARGON_MODEL,
            Member::Astra => crate::paper::models::DEFAULT_ASTRA_MODEL,
        };
        budgets::check_model(store, &self.home, role, "modes", model).map_err(map_error)?;
        let timeout = Duration::from_secs(180);
        let (result, usd, input, output, tokens) = match member {
            Member::Opus | Member::Fable => {
                let (value, usage) = council::claude_call(
                    &Ok(command.clone()),
                    model,
                    prompt,
                    &self.home,
                    &json!({"type":"object"}),
                    timeout,
                )
                .map_err(map_error)?;
                let count = usage.input_tokens.saturating_add(usage.output_tokens);
                (
                    value,
                    usage.usd,
                    usage.input_tokens,
                    usage.output_tokens,
                    (count > 0 || usage.usd.is_some()).then_some(count),
                )
            }
            Member::Argon => {
                let path =
                    std::env::temp_dir().join(format!("axiom-modes-{}.json", base::unique()));
                std::fs::write(&path, schema.to_string())
                    .map_err(|e| Failure::new("io", e.to_string()))?;
                let reply = council::ask_agy(
                    &Ok(command.clone()),
                    &prompt,
                    &self.home,
                    &schema,
                    &path,
                    timeout,
                    &AtomicBool::new(false),
                );
                let _ = std::fs::remove_file(path);
                let tokens = (reply.tokens > 0
                    || reply.raw.as_deref().is_some_and(has_token_usage))
                .then_some(reply.tokens);
                if reply.status != council::Status::Ok {
                    if tokens.is_some() {
                        self.charge(store, role, model, None, 0, reply.tokens, tokens)?;
                    }
                    if reply.status == council::Status::Quota {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                            .min(i64::MAX as u128) as i64;
                        let until = reply
                            .raw
                            .as_deref()
                            .and_then(reset_time)
                            .unwrap_or_else(|| now.saturating_add(3_600_000));
                        pools::mark_exhausted(
                            &self.home,
                            pools::Pool::Argon,
                            until,
                            reply.detail.as_deref().unwrap_or("quota"),
                        )?;
                    }
                    return Err(status_error(
                        reply.status,
                        format!("{}: {}", member.name(), reply.detail.unwrap_or_default()),
                    ));
                }
                (
                    reply
                        .output
                        .ok_or_else(|| Failure::new("malformed_reply", "missing output"))?,
                    None,
                    0,
                    reply.tokens,
                    tokens,
                )
            }
            Member::Astra => {
                let server = AppServer::new(command.clone()).with_slots_home(self.home.clone());
                let report = server
                    .run(&RunRequest {
                        brief: prompt,
                        cwd: self.home.to_string_lossy().into_owned(),
                        model: model.into(),
                        sandbox: "read-only".into(),
                        effort: None,
                        output_schema: Some(schema.clone()),
                        developer_instructions: Some(
                            "Use no tools; return the requested JSON only.".into(),
                        ),
                        timeout_s: timeout.as_secs(),
                        lean: true,
                        keep_plugins: vec![],
                        web_search: false,
                    })
                    .map_err(map_error)?;
                let usage = &report["usage"];
                let input = usage["inputTokens"].as_u64().unwrap_or(0);
                let mut output = usage["outputTokens"].as_u64().unwrap_or(0);
                let tokens = usage["totalTokens"]
                    .as_u64()
                    .or_else(|| (input + output > 0).then_some(input + output));
                if input == 0 && output == 0 {
                    output = tokens.unwrap_or(0);
                }
                if report["status"] != "completed" {
                    if tokens.is_some() {
                        self.charge(store, role, model, None, input, output, tokens)?;
                    }
                    return Err(map_error(Failure::new(
                        report["error"]["kind"].as_str().unwrap_or("model_failed"),
                        report["error"]["message"]
                            .as_str()
                            .unwrap_or("model failed"),
                    )));
                }
                (
                    report["structured_output"].clone(),
                    None,
                    input,
                    output,
                    tokens,
                )
            }
        };
        self.charge(store, role, model, usd, input, output, tokens)?;
        base::schema::validate(&schema, &result).map_err(|e| Failure::new("malformed_reply", e))?;
        Ok(json!({"value":result,"usd":usd,"tokens":tokens}))
    }
    fn charge(
        &self,
        store: &Path,
        role: Role,
        model: &str,
        usd: Option<f64>,
        input: u64,
        output: u64,
        tokens: Option<u64>,
    ) -> Res<()> {
        if tokens.is_some() {
            budgets::record_model(store, &self.home, role, model, usd, input, output, "modes")?;
        } else {
            budgets::record(store, role, model, usd, input, output, "modes")?;
            pools::charge(&self.home, model, None, usd, "modes")?;
        }
        Ok(())
    }
}
fn reset_time(raw: &str) -> Option<i64> {
    fn find(value: &Value) -> Option<i64> {
        match value {
            Value::Object(map) => {
                for key in [
                    "until_ms",
                    "reset_at_ms",
                    "resetAtMs",
                    "reset_at",
                    "resetAt",
                ] {
                    if let Some(n) = map.get(key).and_then(Value::as_i64) {
                        return Some(if key.ends_with("_ms") || key.ends_with("Ms") {
                            n
                        } else {
                            n.saturating_mul(1000)
                        });
                    }
                }
                map.values().find_map(find)
            }
            Value::Array(values) => values.iter().find_map(find),
            _ => None,
        }
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| find(&v))
}
fn has_token_usage(raw: &str) -> bool {
    fn found(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(k, v)| {
                ([
                    "totalTokenCount",
                    "total_tokens",
                    "totalTokens",
                    "promptTokenCount",
                    "input_tokens",
                    "inputTokens",
                    "candidatesTokenCount",
                    "output_tokens",
                    "outputTokens",
                ]
                .contains(&k.as_str())
                    && v.as_u64().is_some())
                    || found(v)
            }),
            Value::Array(values) => values.iter().any(found),
            _ => false,
        }
    }
    serde_json::from_str::<Value>(raw).is_ok_and(|value| found(&value))
}
fn status_error(status: council::Status, detail: String) -> Failure {
    let kind = match status {
        council::Status::Missing => "model_missing",
        council::Status::Quota => "quota_exhausted",
        council::Status::BudgetCut => "budget_exceeded",
        council::Status::Malformed => "malformed_reply",
        _ => "model_failed",
    };
    Failure::new(kind, detail)
}
fn map_error(e: Failure) -> Failure {
    let msg = format!("{} {}", e.kind, e.message).to_lowercase();
    let kind = if e.kind == "pool_exhausted"
        || msg.contains("quota")
        || msg.contains("rate limit")
        || msg.contains("rate-limit")
        || msg.contains("usage limit")
        || msg.contains("session limit")
        || msg.contains("credit balance")
        || msg.contains("resource exhausted")
        || msg.contains("resource_exhausted")
        || msg.contains("429")
    {
        "quota_exhausted"
    } else if e.kind == "budget_exceeded" || e.kind == "token_cap" {
        "budget_exceeded"
    } else if msg.contains("malformed")
        || msg.contains("schema")
        || msg.contains("json")
        || msg.contains("structured_output")
        || (e.kind == "claude_call"
            && (msg.contains("required")
                || msg.contains("expected")
                || msg.contains("additional property")))
    {
        "malformed_reply"
    } else if e.kind == "unavailable" {
        "model_failed"
    } else {
        return e;
    };
    Failure::new(kind, e.message)
}
fn reply_schema() -> Value {
    json!({"type":"object","required":["text","tool_requests"],"additionalProperties":false,"properties":{"text":{"type":"string"},"tool_requests":{"type":"array","items":{"type":"object","required":["tool","args"],"additionalProperties":false,"properties":{"tool":{"type":"string"},"args":{"type":"object"}}}}}})
}
impl Provider for ModelProvider {
    fn ask(&self, ask: &Ask) -> Res<Reply> {
        let allowed: Vec<_> = ask
            .tools
            .iter()
            .filter(|t| READ_TOOLS.contains(&t.as_str()))
            .cloned()
            .collect();
        let prompt = format!("Reply as {}. Return JSON with text and tool_requests. Request only these read-only tools: {}. Never execute tools yourself.\nContext:\n{}\nUser:\n{}", ask.member.name(), allowed.join(", "), truncate_utf8(&ask.context, DEFAULT_CONTEXT_BYTES), ask.prompt);
        let store = match crate::modes::runs::load_session(&self.home, &ask.session) {
            Ok(session) => match session.project {
                Some(project) => crate::orch::store_for(&self.home, &base::slug_for_dir(&project))?,
                None => self.home.join("modes"),
            },
            Err(e) if e.kind == "not_found" => self.home.join("modes"),
            Err(e) => return Err(e),
        };
        let role = if ask.member == Member::Opus {
            Role::Core
        } else {
            Role::CouncilMember
        };
        let result = self.invoke(ask.member, prompt, reply_schema(), role, &store)?;
        let value = &result["value"];
        let requests: Vec<ToolRequest> = serde_json::from_value(value["tool_requests"].clone())
            .map_err(|e| Failure::new("malformed_reply", e.to_string()))?;
        if requests.len() > crate::modes::providers::MAX_CALLS_PER_TURN
            || requests.iter().any(|r| !allowed.contains(&r.tool))
        {
            return Err(Failure::new(
                "malformed_reply",
                "invalid or excessive tool requests",
            ));
        }
        Ok(Reply {
            text: value["text"].as_str().unwrap_or_default().into(),
            tool_requests: requests,
            cost_usd: result["usd"].as_f64(),
            tokens: result["tokens"].as_u64(),
        })
    }
}
pub struct ModelRouter {
    provider: ModelProvider,
}
impl ModelRouter {
    pub fn new(home: PathBuf) -> Self {
        Self {
            provider: ModelProvider::new(home),
        }
    }
    pub fn with_commands(home: PathBuf, cmds: MemberCommands) -> Self {
        Self {
            provider: ModelProvider::with_commands(home, cmds),
        }
    }
}
impl Router for ModelRouter {
    fn route(&self, text: &str, context: &str) -> Res<Routing> {
        let schema = json!({"type":"object","required":["picks"],"additionalProperties":false,"properties":{"picks":{"type":"array","items":{"type":"object","required":["member","reason"],"additionalProperties":false,"properties":{"member":{"type":"string","enum":["opus","astra","fable","argon"]},"reason":{"type":"string"}}}},"note":{"type":"string"}}});
        let prompt = format!("Route the request to at most three distinct members (opus, astra, fable, argon). Return JSON with picks (member and reason) and optional note.\nContext:\n{}\nRequest:\n{}", truncate_utf8(context, DEFAULT_CONTEXT_BYTES), text);
        let value = self.provider.invoke(
            Member::Opus,
            prompt,
            schema,
            Role::Dispatcher,
            &self.provider.home.join("modes"),
        )?;
        router::parse_routing(&value["value"])
            .map_err(|e| Failure::new("malformed_reply", e.message))
    }
}
