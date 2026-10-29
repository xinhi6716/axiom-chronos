use crate::base::{self, Failure, Res};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

type SharedChild = Arc<Mutex<Child>>;
fn registry() -> &'static Mutex<Vec<SharedChild>> {
    static CHILDREN: OnceLock<Mutex<Vec<SharedChild>>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(Vec::new()))
}
pub fn cancel_all() {
    let children = registry().lock().unwrap_or_else(|e| e.into_inner()).clone();
    for child in children {
        base::proc::kill_tree(&mut child.lock().unwrap_or_else(|e| e.into_inner()));
    }
}
struct Cleanup {
    directory: Option<PathBuf>,
    schema: Option<PathBuf>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(path) = &self.schema {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = &self.directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
struct Registered(SharedChild);
impl Drop for Registered {
    fn drop(&mut self) {
        registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|c| !Arc::ptr_eq(c, &self.0));
    }
}
fn io_failure(e: impl std::fmt::Display) -> Failure {
    Failure::new("process", e.to_string())
}
pub fn run(
    command: &[String],
    prompt: &str,
    cwd: Option<&Path>,
    model: &str,
    json_schema: Option<&Value>,
    timeout_s: u64,
) -> Res<Value> {
    if prompt.trim().is_empty() {
        return Err(Failure::new("invalid_argument", "prompt is empty"));
    }
    if command.is_empty() || command[0].is_empty() {
        return Err(Failure::new("invalid_argument", "command is empty"));
    }
    let mut cleanup = Cleanup {
        directory: None,
        schema: None,
    };
    let dir = if let Some(path) = cwd {
        if !path.is_absolute() || !path.is_dir() {
            return Err(Failure::new(
                "invalid_argument",
                format!("not an absolute directory: {}", path.display()),
            ));
        }
        path.to_path_buf()
    } else {
        let path = std::env::temp_dir().join(format!("axiom-agy-{}", base::unique()));
        std::fs::create_dir(&path).map_err(io_failure)?;
        cleanup.directory = Some(path.clone());
        path
    };
    let mut cmd = base::proc::command(command);
    cmd.args([
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--model",
        model,
    ]);
    if let Some(schema) = json_schema {
        let path = std::env::temp_dir().join(format!("axiom-agy-{}.json", base::unique()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_failure)?;
        cleanup.schema = Some(path.clone());
        serde_json::to_writer(&mut file, schema).map_err(io_failure)?;
        cmd.arg("--json-schema").arg(path);
    }
    let mut child = cmd
        .arg("-p=")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io_failure)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Failure::new("process", "missing stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Failure::new("process", "missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Failure::new("process", "missing stderr"))?;
    let child = Arc::new(Mutex::new(child));
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(child.clone());
    let _registered = Registered(child.clone());
    let reader = |mut stream: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
        rx
    };
    let out = reader(Box::new(stdout));
    let err = reader(Box::new(stderr));
    let event = json!({"event":"user", "message":{"role":"user", "content":prompt}}).to_string();
    let start = Instant::now();
    let (write_tx, write_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = writeln!(stdin, "{event}");
        drop(stdin);
        let _ = write_tx.send(result);
    });
    let status = loop {
        let result = child.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
        match result {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(timeout_s) => {
                if let Ok(Err(e)) = write_rx.try_recv() {
                    base::proc::kill_tree(&mut child.lock().unwrap_or_else(|e| e.into_inner()));
                    return Err(io_failure(e));
                }
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                base::proc::kill_tree(&mut child.lock().unwrap_or_else(|e| e.into_inner()));
                return Ok(
                    json!({"status":"timeout", "error":{"kind":"timeout", "message":format!("agy exceeded {timeout_s}s and was killed")}}),
                );
            }
            Err(e) => {
                base::proc::kill_tree(&mut child.lock().unwrap_or_else(|e| e.into_inner()));
                return Err(io_failure(e));
            }
        }
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let output = out
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or_default();
    let errors = err
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or_default();
    let stderr = String::from_utf8_lossy(&errors);
    let lines: Vec<_> = stderr.lines().map(str::trim_end).collect();
    let output = String::from_utf8_lossy(&output);
    let result = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("event").and_then(Value::as_str) == Some("result"))
        .filter_map(|v| v.get("result").cloned())
        .last();
    let Some(result) = result else {
        let tail = lines[lines.len().saturating_sub(10)..].join("\n");
        let message = format!(
            "agy exited {} without a result event\n{tail}",
            status.code().unwrap_or(-1)
        )
        .trim()
        .to_string();
        return Ok(
            json!({"status":"bridge_error", "error":{"kind":"agy_no_result", "message":message}}),
        );
    };
    let mut report = json!({"status":"completed", "response":result.get("response"), "structured_output":result.get("structured_output"), "conversation_id":result.get("conversation_id"), "usage":result.get("usage"), "error":null});
    let failure = if result.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        Some((
            format!(
                "agy_{}",
                result
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_lowercase()
            ),
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ))
    } else if report["response"].as_str().unwrap_or("").trim().is_empty()
        && report["structured_output"].is_null()
    {
        let tail = lines[lines.len().saturating_sub(5)..]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        Some((
            "empty_response".into(),
            format!("agy reported SUCCESS but returned no text\n{tail}")
                .trim()
                .into(),
        ))
    } else if let Some(schema) = json_schema {
        let validation = if report["structured_output"].is_null() {
            Err("structured output is missing".to_string())
        } else {
            base::schema::validate(schema, &report["structured_output"])
        };
        validation.err().map(|error| {
            (
                "invalid_structured_output".into(),
                format!("agy returned invalid structured output: {error}"),
            )
        })
    } else {
        None
    };
    if let Some((kind, message)) = failure {
        report["status"] = json!("failed");
        report["error"] = json!({"kind":kind,"message":message});
    }
    Ok(report)
}
