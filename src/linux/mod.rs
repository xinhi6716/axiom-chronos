use crate::base::{self, Failure, Res};
use serde_json::{json, Value};
use std::{
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};

fn tail(s: &str, count: usize) -> &str {
    let start = s.char_indices().rev().nth(count - 1).map_or(0, |(i, _)| i);
    &s[start..]
}

pub fn run(
    wsl: Option<Vec<String>>,
    on_windows: bool,
    project_dir: &str,
    args: &[String],
    timeout_s: u64,
) -> Res<Value> {
    let dir = base::require_directory(project_dir)?;
    if args.is_empty() {
        return Err(Failure::new("invalid_argument", "args must be a non-empty array of strings passed to python, e.g. [\"-m\", \"unittest\"]"));
    }
    let Some(mut argv) = wsl else {
        let reason = if on_windows {
            "WSL is not installed"
        } else {
            "this host is not Windows; run the command locally"
        };
        return Ok(json!({"status": "unavailable", "reason": reason, "error": null}));
    };
    argv.extend([
        "--cd".into(),
        dir.to_string_lossy().into_owned(),
        "-e".into(),
        "sh".into(),
        "-c".into(),
        "exec \"$HOME/.axiom/linux-venv/bin/python\" \"$@\"".into(),
        "axiom-linux".into(),
    ]);
    argv.extend_from_slice(args);
    let mut child = base::proc::command(&argv)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = read(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = read(Box::new(child.stderr.take().expect("piped stderr")));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(timeout_s) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                base::proc::kill_tree(&mut child);
                let _ = child.wait();
                return Err(Failure::new(
                    "timeout",
                    format!("linux run exceeded {timeout_s}s"),
                ));
            }
            Err(e) => {
                base::proc::kill_tree(&mut child);
                return Err(Failure::new("process", e.to_string()));
            }
        }
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    )
    .replace('\0', "");
    let code = status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            -status.signal().unwrap_or(1)
        }
        #[cfg(not(unix))]
        {
            -1
        }
    });
    if code == 127 {
        return Ok(
            json!({"status": "unavailable", "reason": "no ~/.axiom/linux-venv in WSL; see axiom doctor", "output_tail": tail(&output, 4000), "error": null}),
        );
    }
    Ok(
        json!({"status": if code == 0 { "completed" } else { "failed" }, "exit_code": code, "output_tail": tail(&output, 4000), "error": if code == 0 { Value::Null } else { json!({"kind": "linux_command_failed", "message": tail(&output, 800)}) }}),
    )
}
