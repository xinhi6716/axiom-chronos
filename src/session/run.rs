use super::{
    host::{self, Host},
    Config, Renderer,
};
use crate::{
    base::{self, Failure, Res},
    goal, hook,
};
use std::{
    io::{BufRead, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::Duration,
};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_LOCK: Mutex<()> = Mutex::new(());

#[doc(hidden)]
pub(super) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

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

pub const CONTINUE_TAG: &str = "<chronos-continue/>";

pub fn log_stamp(unix_secs: i64, offset_secs: i32) -> String {
    let secs = unix_secs.saturating_add(i64::from(offset_secs));
    let days = secs.div_euclid(86400);
    let time = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}
#[cfg(unix)]
pub fn local_offset_secs(unix_secs: i64) -> i32 {
    let t = unix_secs as libc::time_t;
    let mut result = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        if libc::localtime_r(&t, result.as_mut_ptr()).is_null() {
            0
        } else {
            (*result.as_ptr()).tm_gmtoff as i32
        }
    }
}
#[cfg(windows)]
pub fn local_offset_secs(unix_secs: i64) -> i32 {
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{
            FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
        },
    };
    let Some(ticks) = unix_secs
        .checked_add(11644473600)
        .and_then(|v| v.checked_mul(10000000))
        .and_then(|v| u64::try_from(v).ok())
    else {
        return 0;
    };
    unsafe {
        let file = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        let mut local_file: FILETIME = std::mem::zeroed();
        if FileTimeToSystemTime(&file, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
            || SystemTimeToFileTime(&local, &mut local_file) == 0
        {
            return 0;
        }
        let local_ticks =
            (u64::from(local_file.dwHighDateTime) << 32) | u64::from(local_file.dwLowDateTime);
        ((i128::from(local_ticks) - i128::from(ticks)) / 10000000) as i32
    }
}

pub fn resume(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    let goal = goal::load(&config.home, session)?
        .ok_or_else(|| Failure::new("not_found", format!("no goal for session {session}")))?;
    if goal.status == "done" {
        return Err(Failure::new(
            "invalid_argument",
            format!("goal {session} is already done"),
        ));
    }
    let cwd = goal
        .cwd
        .as_deref()
        .map(Path::new)
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            Failure::new(
                "invalid_argument",
                format!("goal {session} has no project directory"),
            )
        })?
        .to_path_buf();
    // Preserve the originating workflow and bound when resuming research/creative work.
    let metadata = match std::fs::read(config.home.join("sessions").join(format!("{session}.json")))
    {
        Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes).map_err(failure)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(failure(e)),
    };
    let skill = metadata
        .as_ref()
        .and_then(|m| m["skill"].as_str())
        .unwrap_or("wish");
    let max_turns = max_turns.or_else(|| {
        metadata
            .as_ref()
            .and_then(|m| m["max_turns"].as_u64())
            .and_then(|n| u32::try_from(n).ok())
    });
    // Delay reopening until the executable/configuration checks have passed.
    super::command(config, &cwd, max_turns)?;
    let goal = goal::reopen(&config.home, session)?;
    let prompt = format!("{CONTINUE_TAG} {}", hook::stop_reason(session, &goal));
    run_inner(
        config,
        skill,
        &prompt,
        &cwd,
        max_turns,
        input,
        out,
        Some(session),
    )
}

pub fn run(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    run_inner(config, skill, text, project, max_turns, input, out, None)
}
fn run_inner(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    mut input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
    resume: Option<&str>,
) -> Res<i32> {
    base::require_slug("skill", skill)?;
    base::require_directory(&project.to_string_lossy())?;
    // Validate the executable before opening state or launching any process.
    super::command(config, project, max_turns)?;
    let logs = config.home.join("logs");
    std::fs::create_dir_all(&logs).map_err(failure)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let stamp = log_stamp(now, local_offset_secs(now));
    let (log_path, log) = (0u64..)
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
    let host = Host::new(config, project, log, max_turns)?;
    let skill_contract = match host.workflow(skill, text) {
        Ok(contract) => contract,
        Err(e) => {
            host.shutdown();
            return Err(e);
        }
    };
    let session = match host.start("core", resume, skill == "research") {
        Ok(id) => id,
        Err(e) => {
            host.shutdown();
            if let Some(id) = resume {
                goal::interrupt(&config.home, id, &format!("{}: {}", e.kind, e.message))?;
                let _ = writeln!(
                    out,
                    "resume: axiom wish --resume {id}\nlog: {}",
                    log_path.display()
                );
            }
            return Err(e);
        }
    };
    *host.root.lock().unwrap() = session.clone();
    let mut opened = false;
    let setup = (|| {
        if resume.is_none() {
            if goal::load(&config.home, &session)?.is_some() {
                return Err(Failure::new(
                    "already_exists",
                    "thread already has a root goal",
                ));
            }
            if skill == "research" {
                goal::open_research(&config.home, &session, text, project.to_str())?;
            } else {
                goal::open(&config.home, &session, text, project.to_str())?;
            }
            opened = true;
        }
        let state = config.home.join("sessions").join(format!("{session}.json"));
        std::fs::create_dir_all(state.parent().unwrap()).map_err(failure)?;
        base::atomic_write(
            &state,
            &serde_json::to_vec(
                &serde_json::json!({"thread_id":session,"skill":skill,"max_turns":max_turns}),
            )
            .map_err(failure)?,
        )
        .map_err(failure)?;
        host.register(&session, "core", vec![])
    })();
    if let Err(e) = setup {
        host.shutdown();
        if opened || resume.is_some() {
            let _ = goal::interrupt(
                &config.home,
                &session,
                &format!("{}: {}", e.kind, e.message),
            );
            let _ = writeln!(
                out,
                "resume: axiom wish --resume {session}\nlog: {}",
                log_path.display()
            );
        }
        return Err(e);
    }
    let events = host.server.subscribe(&session);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => (),
            }
            let text = line.trim_end_matches(['\r', '\n']);
            if !text.trim().is_empty() && tx.send(text.to_owned()).is_err() {
                break;
            }
        }
        // EOF is only input closure; the goal still controls session completion.
    });
    let mut renderer = Renderer::new();
    let mut prompt = if resume.is_some() {
        text.to_owned()
    } else {
        format!("/chronos:{skill} {text}")
    };
    let mut notice = Some(format!("Root session: {session}. Project: {}. Goal is already open. Follow this workflow:\n{skill_contract}",base::slug_for_dir(project)));
    let mut cause = None;
    let mut deferred = Vec::<String>::new();
    let result: Res<i32> = (|| loop {
        if interrupted() {
            cause = Some("interrupted by the user".to_owned());
            return Ok(130);
        }
        if host.budget_exhausted.load(Ordering::Acquire) || host.step(&session).is_err() {
            cause = Some("maximum turns reached".to_owned());
            return Ok(1);
        }
        let mut turn_input = vec![serde_json::json!({"type":"text","text":prompt})];
        if let Some(notice) = notice.take() {
            turn_input.push(serde_json::json!({"type":"text","text":notice}));
        }
        let started = host.server.request(
            "turn/start",
            host.turn_params(&session, serde_json::json!(turn_input), "max"),
        )?;
        let turn = started["turn"]["id"]
            .as_str()
            .ok_or_else(|| Failure::new("codex_rpc", "turn/start: missing turn id"))?
            .to_owned();
        host.server.register_turn(&session, &turn);
        let mut report =
            serde_json::json!({"status":null,"final_message":null,"usage":null,"error":null});
        loop {
            for notice in host.drain_notices() {
                renderer.event(&notice, out);
            }
            if host.cancelled.load(Ordering::Acquire)
                && !goal::load(&config.home, &session)?
                    .is_some_and(|g| g.reason.as_deref() == Some("user_stop"))
            {
                return Err(Failure::new(
                    "interrupted",
                    "session callback failed or was cancelled",
                ));
            }
            if host.budget_exhausted.load(Ordering::Acquire) {
                cause = Some("maximum turns reached (model/tool step budget)".to_owned());
                return Ok(1);
            }
            if interrupted() {
                cause = Some("interrupted by the user".to_owned());
                return Ok(130);
            }
            if goal::load(&config.home, &session)?
                .is_some_and(|g| g.reason.as_deref() == Some("user_stop"))
            {
                host.bridge.cancel();
                let g = goal::load(&config.home, &session)?.unwrap();
                let _ = writeln!(out, "goal blocked: {}", g.note.unwrap_or_default());
                return Ok(1);
            }
            while let Ok(line) = rx.try_recv() {
                {
                    let _goal = host.goal_lock.lock().unwrap();
                    goal::interject(&config.home, &session, &line)?;
                }
                if let Err(e) = host.server.request("turn/steer",serde_json::json!({"threadId":session,"expectedTurnId":turn,"input":[{"type":"text","text":line}]})) {
                    if e.kind != "codex_rpc" { return Err(e); }
                    // Completion may race input. Submit the message at the next turn
                    // boundary rather than losing it or reporting an artificial crash.
                    host.log(&serde_json::json!({"hostEvent":"steerDeferred","threadId":session,"expectedTurnId":turn,"error":e.to_json()}))?;
                    deferred.push(line);
                }
            }
            let event = match events.recv_timeout(Duration::from_millis(50)) {
                Ok(e) => e,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => {
                    return Err(Failure::new(
                        "codex_exited",
                        "Codex app-server exited without a completed turn",
                    ))
                }
            };
            host.log(&event)?;
            renderer.event(&event, out);
            host.observe(&event)?;
            host::record(&mut report, &event);
            if event["method"] != "turn/completed" || event["params"]["turn"]["id"] != turn {
                continue;
            }
            host.server.unregister_turn(&session, &turn);
            break;
        }
        if report["status"] != "completed" {
            cause = Some(format!(
                "Codex turn {}: {}",
                report["status"], report["error"]
            ));
            return Ok(1);
        }
        let g = goal::load(&config.home, &session)?
            .ok_or_else(|| Failure::new("not_found", "root goal disappeared"))?;
        match g.status.as_str() {
            "active" => {
                prompt = format!("{CONTINUE_TAG} {}", hook::stop_reason(&session, &g));
                if !deferred.is_empty() {
                    prompt.push_str("\nUser interjections received as the prior turn ended:\n");
                    prompt.push_str(&std::mem::take(&mut deferred).join("\n"));
                }
            }
            "done" => return Ok(0),
            "blocked" => {
                let _ = writeln!(out, "goal blocked: {}", g.note.unwrap_or_default());
                return Ok(1);
            }
            _ => return Err(Failure::new("invalid_goal", "unknown goal status")),
        }
    })();
    if let Err(e) = &result {
        cause = Some(format!("{}: {}", e.kind, e.message));
    }
    host.shutdown();
    for notice in host.drain_notices() {
        renderer.event(&notice, out);
    }
    renderer.summary(out);
    if let Some(cause) = cause {
        match goal::interrupt(&config.home, &session, &cause) {
            Ok(Some(g)) => {
                let _ = writeln!(
                    out,
                    "goal blocked: {}\nresume: axiom wish --resume {session}",
                    g.note.unwrap_or_default()
                );
            }
            Ok(None) => (),
            Err(e) => {
                let _ = writeln!(out, "goal error: {}: {}", e.kind, e.message);
            }
        }
    }
    let _ = writeln!(out, "\nlog: {}", log_path.display());
    let _ = out.flush();
    match result {
        Err(e) => {
            let _ = writeln!(out, "session error: {}: {}", e.kind, e.message);
            Ok(1)
        }
        ok => ok,
    }
}
