use super::{command, user_message, Config, Renderer};
use crate::{
    base::{self, Failure, Res},
    engine, goal, hook,
};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_LOCK: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub fn trigger_interrupt() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}
#[cfg(windows)]
unsafe extern "system" fn interrupt(kind: u32) -> i32 {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        INTERRUPTED.store(true, Ordering::SeqCst);
        1
    } else {
        0
    }
}
struct Signals {
    #[cfg(unix)]
    old_int: libc::sigaction,
    #[cfg(unix)]
    old_term: libc::sigaction,
}
impl Signals {
    fn install() -> Self {
        INTERRUPTED.store(false, Ordering::SeqCst);
        #[cfg(unix)]
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = interrupt as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut old_int = std::mem::zeroed();
            let mut old_term = std::mem::zeroed();
            libc::sigaction(libc::SIGINT, &action, &mut old_int);
            libc::sigaction(libc::SIGTERM, &action, &mut old_term);
            Self { old_int, old_term }
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(interrupt), 1);
            Self {}
        }
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::sigaction(libc::SIGINT, &self.old_int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.old_term, std::ptr::null_mut());
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(interrupt), 0);
        }
    }
}
fn failure(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn run(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    mut input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    base::require_slug("skill", skill)?;
    let plugin = engine::plugin_dir(&config.home).map_err(failure)?;
    let argv = command(config, &plugin, max_turns)?;
    let logs = config.home.join("logs");
    std::fs::create_dir_all(&logs).map_err(failure)?;
    let iso = base::now_iso();
    let stamp = format!(
        "{}{}{}-{}{}{}",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    );
    let (log_path, mut log) = (0u64..)
        .find_map(|n| {
            let suffix = if n == 0 {
                String::new()
            } else {
                format!("-{n}")
            };
            let path = logs.join(format!("{stamp}-{skill}{suffix}.jsonl"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => Some(Ok((path, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(failure(e))),
            }
        })
        .unwrap()?;
    let _lock = SIGNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _signals = Signals::install();
    let mut child = base::proc::command(&argv)
        .current_dir(project)
        .env("AXIOM_DATA_HOME", &config.home)
        .env("AXIOM_HEADLESS", "1")
        .env("AXIOM_BIN", &config.bin)
        .env_remove("AXIOM_PYTHON")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let mut stdin = child.stdin.take().unwrap();
    if let Err(e) = stdin
        .write_all(user_message(&format!("/chronos:{skill} {text}")).as_bytes())
        .and_then(|_| stdin.flush())
    {
        base::proc::kill_tree(&mut child);
        return Err(failure(e));
    }
    let writer = Arc::new(Mutex::new(Some(stdin)));
    let input_writer = Arc::clone(&writer);
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                continue;
            }
            let mut guard = input_writer.lock().unwrap_or_else(|e| e.into_inner());
            let Some(stdin) = guard.as_mut() else { break };
            if stdin
                .write_all(user_message(line).as_bytes())
                .and_then(|_| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(bytes).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut renderer = Renderer::new();
    let mut session = None::<String>;
    let mut final_result = false;
    let result: Res<i32> = loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            base::proc::kill_tree(&mut child);
            break Ok(130);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                if let Err(e) = log.write_all(&bytes) {
                    base::proc::kill_tree(&mut child);
                    break Err(failure(e));
                }
                let Ok(event) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                if event["type"] == "system" && event["subtype"] == "init" {
                    session = event["session_id"].as_str().map(str::to_owned);
                }
                renderer.event(&event, out);
                if event["type"] != "result" {
                    continue;
                }
                if event["is_error"].as_bool().unwrap_or(false) {
                    final_result = true;
                } else if let Some(id) = session.as_deref() {
                    match goal::load(&config.home, id) {
                        Ok(Some(g)) if g.status == "active" => {
                            let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(stdin) = guard.as_mut() {
                                if let Err(e) = stdin
                                    .write_all(user_message(&hook::stop_reason(id, &g)).as_bytes())
                                    .and_then(|_| stdin.flush())
                                {
                                    base::proc::kill_tree(&mut child);
                                    break Err(failure(e));
                                }
                            }
                            continue;
                        }
                        Ok(Some(g)) if g.status == "blocked" => {
                            let _ =
                                writeln!(out, "goal blocked: {}", g.note.as_deref().unwrap_or(""));
                        }
                        Ok(_) | Err(_) => {}
                    }
                    final_result = true;
                } else {
                    final_result = true;
                }
                if final_result {
                    writer.lock().unwrap_or_else(|e| e.into_inner()).take();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = match child.wait() {
                    Ok(s) => s,
                    Err(e) => break Err(failure(e)),
                };
                if final_result {
                    break Ok(if renderer.is_error() { 1 } else { 0 });
                }
                let code = status.code().filter(|c| *c != 0).unwrap_or(1);
                let _ = writeln!(
                    out,
                    "\nclaude exited {code} without a result; raw events are in {}",
                    log_path.display()
                );
                break Ok(code);
            }
        }
    };
    writer.lock().unwrap_or_else(|e| e.into_inner()).take();
    if final_result {
        renderer.summary(out);
    }
    let _ = writeln!(out, "\nlog: {}", log_path.display());
    let _ = out.flush();
    result
}
