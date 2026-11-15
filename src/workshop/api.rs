use super::{
    host::{ModuleCtx, RequestHandler},
    permissions::{self, Permission, Provider},
    services::{self, DecisionRequest, ModelCall, Services},
};
use crate::{
    base::{self, Failure, Res},
    seams::sha256_hex,
    usage::extensions::{self, ExtRecord},
};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const VERSION: u64 = 1;
pub struct Api {
    pub services: Services,
    permits: Mutex<HashMap<String, usize>>,
}
static STORAGE: Mutex<()> = Mutex::new(());
struct Permit<'a> {
    permits: &'a Mutex<HashMap<String, usize>>,
    name: String,
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut map = self.permits.lock().unwrap();
        if let Some(n) = map.get_mut(&self.name) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.name);
            }
        }
    }
}
fn permit<'a>(permits: &'a Mutex<HashMap<String, usize>>, ctx: &ModuleCtx) -> Res<Permit<'a>> {
    loop {
        services::cancelled(&ctx.cancel)?;
        {
            let mut map = permits.lock().unwrap();
            let n = map.entry(ctx.name.clone()).or_default();
            if *n < ctx.grants.max_parallel() as usize {
                *n += 1;
                return Ok(Permit {
                    permits,
                    name: ctx.name.clone(),
                });
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn failed(e: impl std::fmt::Display) -> Failure {
    Failure::new("failed", e.to_string())
}
fn string<'a>(p: &'a Value, key: &str) -> Res<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{key} must be a string")))
}
fn key(p: &Value) -> Res<&str> {
    let k = string(p, "key")?;
    if k.is_empty()
        || k.len() > 128
        || !k
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    {
        return Err(invalid("invalid memory key"));
    }
    Ok(k)
}
fn append(path: &std::path::Path, value: &Value) -> Res<()> {
    let mut bytes = serde_json::to_vec(value).map_err(failed)?;
    bytes.push(b'\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(failed)?;
    f.write_all(&bytes).map_err(failed)
}
fn memory(ctx: &ModuleCtx) -> Res<Map<String, Value>> {
    match fs::read(ctx.dirs.memory.join("memory.json")) {
        Ok(b) => serde_json::from_slice(&b).map_err(failed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(failed(e)),
    }
}
fn write_memory(ctx: &ModuleCtx, map: &Map<String, Value>) -> Res<()> {
    base::atomic_write(
        &ctx.dirs.memory.join("memory.json"),
        &serde_json::to_vec(map).map_err(failed)?,
    )
    .map_err(failed)
}
impl RequestHandler for Api {
    fn handle(&self, ctx: &ModuleCtx, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if request.get("v").and_then(Value::as_u64) != Some(VERSION) {
            return json!({"v":VERSION,"id":id,"ok":false,"error":{"kind":"unsupported_version","message":"unsupported API version","supported":[VERSION]}});
        }
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = if request.get("module").is_some() || params.get("module").is_some() {
            Err(Failure::new(
                "identity_spoof",
                "module identity comes from the channel",
            ))
        } else if !params.is_object() {
            Err(invalid("params must be an object"))
        } else if let Some(method) = request.get("method").and_then(Value::as_str) {
            self.dispatch(ctx, method, &params)
        } else {
            Err(invalid("method must be a string"))
        };
        match result {
            Ok(result) => json!({"v":VERSION,"id":id,"ok":true,"result":result}),
            Err(e) => json!({"v":VERSION,"id":id,"ok":false,"error":e.to_json()}),
        }
    }
}
impl Api {
    pub fn new(services: Services) -> Self {
        Self {
            services,
            permits: Mutex::new(HashMap::new()),
        }
    }

    fn dispatch(&self, ctx: &ModuleCtx, method: &str, p: &Value) -> Res<Value> {
        services::cancelled(&ctx.cancel)?;
        match method {
            "api.hello" => Ok(
                json!({"versions":[VERSION],"module":ctx.name,"permissions":ctx.manifest.permissions,"max_parallel":ctx.grants.max_parallel()}),
            ),
            "log.write" => {
                let level = string(p, "level")?;
                let message = string(p, "message")?;
                let _guard = STORAGE.lock().unwrap();
                append(
                    &ctx.dirs.logs.join("module.log"),
                    &json!({"ts":base::now_iso(),"module":ctx.name,"level":level,"message":message}),
                )?;
                Ok(json!({}))
            }
            "receipt.write" => {
                let kind = string(p, "kind")?;
                let body = p.get("body").ok_or_else(|| invalid("body required"))?;
                let mut receipt =
                    json!({"ts":base::now_iso(),"module":ctx.name,"kind":kind,"body":body});
                let hash = sha256_hex(&serde_json::to_vec(&receipt).map_err(failed)?);
                receipt["sha256"] = json!(hash);
                let _guard = STORAGE.lock().unwrap();
                append(&ctx.dirs.receipts.join("receipts.jsonl"), &receipt)?;
                Ok(json!({"sha256":hash}))
            }
            "usage.get" => {
                let (records, _) = extensions::read(&self.services.engine_home).map_err(failed)?;
                Ok(json!(extensions::by_module(&records)
                    .remove(&ctx.name)
                    .unwrap_or_default()))
            }
            "memory.put" | "memory.get" | "memory.delete" | "memory.list" => {
                ctx.grants.check(&Permission::Memory)?;
                let _guard = STORAGE.lock().unwrap();
                let mut m = memory(ctx)?;
                match method {
                    "memory.list" => Ok(json!({"keys":m.keys().collect::<Vec<_>>() })),
                    "memory.get" => Ok(json!({"value":m.get(key(p)?).cloned()})),
                    "memory.delete" => {
                        m.remove(key(p)?);
                        write_memory(ctx, &m)?;
                        Ok(json!({}))
                    }
                    _ => {
                        let k = key(p)?;
                        let v = p.get("value").ok_or_else(|| invalid("value required"))?;
                        m.insert(k.into(), v.clone());
                        write_memory(ctx, &m)?;
                        Ok(json!({}))
                    }
                }
            }
            "file.read" | "file.write" | "file.list" => {
                ctx.grants.check(&Permission::Files)?;
                let path = permissions::confine(
                    &ctx.dirs.files,
                    if method == "file.list" {
                        p.get("path").and_then(Value::as_str).unwrap_or(".")
                    } else {
                        string(p, "path")?
                    },
                )?;
                match method {
                    "file.read" => Ok(json!({"content":fs::read_to_string(path).map_err(failed)?})),
                    "file.write" => {
                        let data = string(p, "content")?;
                        fs::write(path, data).map_err(failed)?;
                        Ok(json!({}))
                    }
                    _ => {
                        let mut names = fs::read_dir(path)
                            .map_err(failed)?
                            .map(|e| {
                                e.map(|x| x.file_name().to_string_lossy().into_owned())
                                    .map_err(failed)
                            })
                            .collect::<Res<Vec<_>>>()?;
                        names.sort();
                        Ok(json!({"entries":names}))
                    }
                }
            }
            "model.call" => {
                let provider_str = string(p, "provider")?;
                let provider = match provider_str {
                    "claude" => Provider::Claude,
                    "codex" => Provider::Codex,
                    _ => return Err(invalid("unknown provider")),
                };
                let model = string(p, "model")?;
                if model.is_empty() || model == "*" {
                    return Err(invalid("invalid model"));
                }
                ctx.grants.check(&Permission::Model {
                    provider: provider.clone(),
                    model: model.into(),
                })?;
                let prompt = string(p, "prompt")?;
                let timeout_s = p
                    .get("timeout_s")
                    .map(|v| {
                        v.as_u64()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| invalid("invalid timeout_s"))
                    })
                    .transpose()?
                    .unwrap_or(300);
                let _permit = permit(&self.permits, ctx)?;
                let _slot = self
                    .services
                    .slots
                    .acquire(provider.clone(), model, &ctx.cancel)?;
                services::cancelled(&ctx.cancel)?;
                let call = ModelCall {
                    provider: provider.clone(),
                    model: model.into(),
                    prompt: prompt.into(),
                    timeout_s,
                    cwd: ctx.dirs.files.clone(),
                };
                let backend = if provider == Provider::Codex {
                    &self.services.codex
                } else {
                    &self.services.claude
                };
                let reply = backend.call(&call, &ctx.cancel)?;
                services::cancelled(&ctx.cancel)?;
                let rec = ExtRecord {
                    ts_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .min(i64::MAX as u128) as i64,
                    module: Some(ctx.name.clone()),
                    provider: provider_str.into(),
                    model: Some(model.into()),
                    input_tokens: reply.input_tokens,
                    cached_input_tokens: reply.cached_input_tokens,
                    output_tokens: reply.output_tokens,
                    usd: reply.usd,
                    ..Default::default()
                };
                extensions::append(&self.services.engine_home, &rec).map_err(failed)?;
                Ok(
                    json!({"text":reply.text,"usage":{"input_tokens":reply.input_tokens,"cached_input_tokens":reply.cached_input_tokens,"output_tokens":reply.output_tokens,"usd":reply.usd}}),
                )
            }
            "process.spawn" => {
                ctx.grants.check(&Permission::Subprocess)?;
                let argv = p
                    .get("argv")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("argv must be an array"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("argv must contain non-empty strings"))
                    })
                    .collect::<Res<Vec<_>>>()?;
                if argv.is_empty() {
                    return Err(invalid("argv is empty"));
                }
                let timeout = p
                    .get("timeout_s")
                    .and_then(Value::as_u64)
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid("invalid timeout_s"))?;
                let mut cmd = base::proc::command(&argv);
                cmd.env_clear()
                    .envs(permissions::child_env(&ctx.name, &ctx.grants, &ctx.dirs))
                    .current_dir(&ctx.dirs.files);
                let output = services::run_process(&mut cmd, timeout, &ctx.cancel)?;
                Ok(json!({"exit_code":output.code,"stdout":output.stdout,"stderr":output.stderr}))
            }
            "net.request" => {
                ctx.grants.check(&Permission::Network)?;
                self.services.network.request(&ctx.name, p)
            }
            "decision.route" => {
                ctx.grants.check(&Permission::DecisionRouter)?;
                let question = string(p, "question")?.to_owned();
                let options = p
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("options must be an array"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("invalid option"))
                    })
                    .collect::<Res<Vec<_>>>()?;
                if options.is_empty() {
                    return Err(invalid("options must not be empty"));
                }
                let reply = self
                    .services
                    .router
                    .route(&ctx.name, &DecisionRequest { question, options })?;
                Ok(json!({"choice":reply.choice,"reason":reply.reason}))
            }
            _ => Err(Failure::new(
                "unknown_method",
                format!("unknown method: {method}"),
            )),
        }
    }
}
