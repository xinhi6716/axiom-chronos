use super::{ready, Model, TokenUse};
use crate::{
    base::{proc, Failure, Res},
    codex::{AppServer, RunRequest},
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Real {
    codex: Arc<AppServer>,
    claude: Vec<String>,
}
impl Real {
    pub fn new(codex: Arc<AppServer>, claude: Vec<String>) -> Self {
        Self { codex, claude }
    }
}

pub fn haiku_args() -> Vec<String> {
    [
        "-p",
        "--model",
        "claude-haiku-5-5",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "local",
        "--output-format",
        "json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
pub fn luna_tokens(report: &Value) -> TokenUse {
    TokenUse {
        input: report["usage"]["inputTokens"].as_u64(),
        output: report["usage"]["outputTokens"].as_u64(),
    }
}
pub fn parse_haiku_output(text: &str) -> Res<(String, TokenUse)> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| Failure::new("bad_reply", e.to_string()))?;
    if value["is_error"] == true {
        return Err(Failure::new(
            "haiku_failed",
            value["result"]
                .as_str()
                .unwrap_or("Haiku returned an error"),
        ));
    }
    let result = value["result"]
        .as_str()
        .ok_or_else(|| Failure::new("bad_reply", "missing result"))?;
    let usage = &value["usage"];
    let inputs: Vec<u64> = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .filter_map(|key| usage[*key].as_u64())
    .collect();
    Ok((
        result.into(),
        TokenUse {
            input: if inputs.is_empty() {
                None
            } else {
                Some(inputs.into_iter().fold(0u64, u64::saturating_add))
            },
            output: usage["output_tokens"].as_u64(),
        },
    ))
}

impl Model for Real {
    fn luna(
        &self,
        prompt: &str,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Res<(String, TokenUse)> {
        ready(deadline, cancel)?;
        let codex = Arc::clone(&self.codex);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let prompt = prompt.to_owned();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| {
                let model = codex.resolve_model("luna")?;
                ready(deadline, &worker_cancelled)?;
                let req = RunRequest {
                    brief: prompt,
                    cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                    model,
                    sandbox: "read-only".into(),
                    effort: Some("low".into()),
                    output_schema: None,
                    developer_instructions: Some(
                        "Reply only in the requested two-line format.".into(),
                    ),
                    timeout_s: deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs()
                        .max(1),
                    lean: true,
                    keep_plugins: vec![],
                    web_search: false,
                };
                let report = codex.run_capped(&req, Some(4000))?;
                if report["status"] != "completed" {
                    let kind = if report["status"] == "timeout" {
                        "timeout"
                    } else {
                        report["error"]["kind"].as_str().unwrap_or("luna_failed")
                    };
                    return Err(Failure::new(
                        kind,
                        report["error"]["message"].as_str().unwrap_or("Luna failed"),
                    ));
                }
                Ok((
                    report["final_message"].as_str().unwrap_or("").to_owned(),
                    luna_tokens(&report),
                ))
            })();
            let _ = tx.send(result);
        });
        loop {
            if let Err(e) = ready(deadline, cancel) {
                cancelled.store(true, Ordering::Relaxed);
                self.codex.cancel_all();
                return Err(e);
            }
            match rx.recv_timeout(
                Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
            ) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Failure::new("luna_failed", "Luna worker stopped"))
                }
            }
        }
    }

    fn haiku(
        &self,
        prompt: &str,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Res<(String, TokenUse)> {
        ready(deadline, cancel)?;
        let mut child = proc::command(&self.claude)
            .args(haiku_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure::new("haiku_failed", e.to_string()))?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input = prompt.to_owned();
        thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (out_tx, out_rx) = mpsc::channel();
        let (err_tx, err_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut b = Vec::new();
            let r = stdout.read_to_end(&mut b).map(|_| b);
            let _ = out_tx.send(r);
        });
        thread::spawn(move || {
            let mut b = Vec::new();
            let r = stderr.read_to_end(&mut b).map(|_| b);
            let _ = err_tx.send(r);
        });
        let status = loop {
            if let Err(e) = ready(deadline, cancel) {
                proc::kill_tree(&mut child);
                return Err(e);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(
                    Duration::from_millis(50)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
                Err(e) => {
                    proc::kill_tree(&mut child);
                    return Err(Failure::new("haiku_failed", e.to_string()));
                }
            }
        };
        // Readers drain both pipes concurrently; continue enforcing cancellation while waiting for EOF.
        let read = |rx: mpsc::Receiver<std::io::Result<Vec<u8>>>| -> Res<Vec<u8>> {
            loop {
                ready(deadline, cancel)?;
                match rx.recv_timeout(
                    Duration::from_millis(50)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ) {
                    Ok(r) => return r.map_err(|e| Failure::new("haiku_failed", e.to_string())),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(Failure::new("haiku_failed", "output reader stopped")),
                }
            }
        };
        let out = read(out_rx)?;
        let err = read(err_rx)?;
        if !status.success() {
            return Err(Failure::new(
                "haiku_failed",
                String::from_utf8_lossy(&err).into_owned(),
            ));
        }
        let text =
            std::str::from_utf8(&out).map_err(|e| Failure::new("bad_reply", e.to_string()))?;
        parse_haiku_output(text)
    }
}
