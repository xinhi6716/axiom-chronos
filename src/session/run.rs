use super::{command, user_message, Config, Renderer};
use crate::{
    base::{self, Failure, Res},
    codegraph, council, engine, goal, hook, provenance,
};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Stdio},
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
/// Runs `body` with Ctrl-C raising the stop flag it is given.
pub(super) fn until_interrupted<T>(body: impl FnOnce(&AtomicBool) -> T) -> T {
    let _lock = SIGNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _signals = Signals::install();
    body(&INTERRUPTED)
}
fn failure(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub const CONTINUE_TAG: &str = "<chronos-continue/>";
/// Long `codex_run` calls outlast the five-minute prompt cache; the one-hour
/// TTL keeps the dispatcher's prefix warm across them (Claude Code 2.1.242+).
pub const CACHE_TTL_ENV: [(&str, &str); 2] = [
    ("CLAUDE_CODE_PROMPT_CACHE_TTL", "1h"),
    ("CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL", "1h"),
];
pub const LIMIT_MARGIN_SECS: i64 = 120;
const LIMIT_JUST_PASSED_SECS: i64 = 15 * 60;

pub struct Clock {
    pub now: Box<dyn Fn() -> i64 + Send + Sync>,
    pub sleep: Box<dyn Fn(Duration) + Send + Sync>,
}
impl Clock {
    pub fn system() -> Self {
        Self {
            now: Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            }),
            sleep: Box::new(std::thread::sleep),
        }
    }
}

pub fn limit_reset(message: &str, now: i64, offset_secs: i32) -> Option<i64> {
    static RESET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RESET.get_or_init(|| {
        regex::Regex::new(r"(?i)\blimit\b.*\bresets\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?")
            .unwrap()
    });
    let c = re.captures(message)?;
    let mut hour: i64 = c[1].parse().ok()?;
    let minute: i64 = c.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?;
    match c.get(3).map(|m| m.as_str().to_ascii_lowercase()) {
        Some(half) => {
            if hour < 1 || hour > 12 {
                return None;
            }
            hour %= 12;
            if half == "pm" {
                hour += 12;
            }
        }
        None if hour > 23 => return None,
        None => {}
    }
    if minute > 59 {
        return None;
    }
    let local = now + i64::from(offset_secs);
    let mut target = local.div_euclid(86400) * 86400 + hour * 3600 + minute * 60;
    if target + LIMIT_JUST_PASSED_SECS < local {
        target += 86400;
    }
    Some(target - i64::from(offset_secs))
}
fn clock_time(unix_secs: i64, offset_secs: i32) -> String {
    let time = (unix_secs + i64::from(offset_secs)).rem_euclid(86400);
    format!("{:02}:{:02}", time / 3600, time / 60 % 60)
}

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

pub fn wish_text(words: &str, creative: bool, no_architect: bool) -> String {
    if creative {
        format!(
            "[creative]{}",
            if words.is_empty() {
                String::new()
            } else {
                format!(" {words}")
            }
        )
    } else if no_architect {
        format!("[no-architect] {words}")
    } else {
        words.to_owned()
    }
}

fn continue_prompt(config: &Config, session: &str) -> Res<String> {
    let goal = goal::reopen(&config.home, session)?;
    Ok(format!(
        "{CONTINUE_TAG} {}",
        hook::stop_reason_in(&config.home, session, &goal)
    ))
}

/// The first prompt and project directory of a resumable goal: the user's `message` when
/// given, otherwise the continuation prompt.
pub fn resume_target(
    config: &Config,
    session: &str,
    message: Option<&str>,
) -> Res<(String, PathBuf)> {
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
    let prompt = match message {
        Some(text) => {
            goal::reopen(&config.home, session)?;
            text.to_owned()
        }
        None => continue_prompt(config, session)?,
    };
    Ok((prompt, cwd))
}

pub fn resume(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    message: Option<&str>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    resume_keeping(config, session, max_turns, message, input, None, out)
}

pub(super) fn resume_keeping(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    message: Option<&str>,
    input: Box<dyn BufRead + Send>,
    hand_back: Option<HandBack>,
    out: &mut dyn Write,
) -> Res<i32> {
    let (prompt, cwd) = resume_target(config, session, message)?;
    run_observed(
        config,
        "wish",
        &prompt,
        &cwd,
        max_turns,
        input,
        hand_back,
        out,
        Some(session),
        &mut |_| {},
        &Launch::default(),
    )
}

pub(super) fn run_observed(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    hand_back: Option<HandBack>,
    out: &mut dyn Write,
    resume: Option<&str>,
    observe: &mut dyn FnMut(&serde_json::Value),
    launch: &Launch,
) -> Res<i32> {
    let _lock = SIGNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _signals = Signals::install();
    let spawned = spawn_with(config, skill, text, project, max_turns, resume, launch)?;
    forward_input_keeping(Arc::clone(&spawned.writer), input, hand_back);
    drive(spawned, config, out, &INTERRUPTED, observe)
}

pub type Writer = Arc<Mutex<Option<ChildStdin>>>;

/// Where a run's forwarder returns a line it read but could not deliver because the run is over.
pub type HandBack = Box<dyn FnOnce(String) + Send>;

/// A write failed because the reading process is closing or gone: Unix EPIPE, Windows
/// ERROR_BROKEN_PIPE (109) or ERROR_NO_DATA (232, "The pipe is being closed").
fn pipe_closed(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::BrokenPipe
        || (cfg!(windows) && matches!(e.raw_os_error(), Some(109) | Some(232)))
}

pub fn send(stdin: &mut ChildStdin, text: &str) -> std::io::Result<()> {
    stdin
        .write_all(user_message(text).as_bytes())
        .and_then(|_| stdin.flush())
}

/// A running claude with its first message sent; `drive` takes it from here.
pub struct Spawned {
    child: Child,
    pub writer: Writer,
    lines: mpsc::Receiver<Vec<u8>>,
    log: File,
    pub log_path: PathBuf,
    pub provenance: provenance::Provenance,
    skill: String,
    project: PathBuf,
    max_turns: Option<u32>,
    advanced_research: bool,
}

/// What a launch opts in to. The default is a launch exactly as it has always been.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    /// Advanced research: the council of Astra, Fable 5.2 and Argon. Costs real money.
    pub advanced_research: bool,
}

pub fn spawn(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    resume: Option<&str>,
) -> Res<Spawned> {
    spawn_with(
        config,
        skill,
        text,
        project,
        max_turns,
        resume,
        &Launch::default(),
    )
}

pub fn spawn_with(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    resume: Option<&str>,
    launch: &Launch,
) -> Res<Spawned> {
    base::require_slug("skill", skill)?;
    let plugin = engine::plugin_dir(&config.home).map_err(failure)?;
    let mut argv = command(config, &plugin, max_turns, codegraph::enabled(project))?;
    let advanced_research = launch.advanced_research
        || resume.is_some_and(|id| council::switch_remembered(&config.home, id));
    if advanced_research {
        if let Some(tools) = argv
            .iter()
            .position(|a| a == "--allowedTools")
            .map(|i| i + 1)
        {
            argv[tools] = format!("{},{}", argv[tools], council::TOOL);
        }
        argv.extend([
            "--append-system-prompt".into(),
            council::SYSTEM_PROMPT.into(),
        ]);
    }
    if let Some(id) = resume {
        argv.extend(["--resume".into(), id.into()]);
    }
    let logs = config.home.join("logs");
    std::fs::create_dir_all(&logs).map_err(failure)?;
    let now = (config.clock.now)();
    let stamp = log_stamp(now, local_offset_secs(now));
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
    let provenance = provenance::of_session(
        config.claude.as_deref().unwrap_or(&[]),
        &config.bin,
        &plugin,
        project,
    );
    log.write_all(provenance.log_line().as_bytes())
        .map_err(failure)?;
    let mut child = base::proc::command(&argv);
    child
        .current_dir(project)
        .env("AXIOM_DATA_HOME", &config.home)
        .env("AXIOM_HEADLESS", "1")
        .env("AXIOM_BIN", &config.bin)
        .envs(CACHE_TTL_ENV)
        .env(provenance::ENV, provenance.to_value().to_string())
        .env("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS", "1")
        .env_remove("AXIOM_PYTHON")
        .env_remove(council::ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if advanced_research {
        child.env(council::ENV, "1");
    }
    let mut child = child
        .spawn()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let mut stdin = child.stdin.take().unwrap();
    if let Err(e) = send(
        &mut stdin,
        &if resume.is_some() {
            text.to_owned()
        } else if text.is_empty() {
            format!("/chronos:{skill}")
        } else {
            format!("/chronos:{skill} {text}")
        },
    ) {
        base::proc::kill_tree(&mut child);
        return Err(failure(e));
    }
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = mpsc::channel();
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
    Ok(Spawned {
        child,
        writer: Arc::new(Mutex::new(Some(stdin))),
        lines,
        log,
        log_path,
        provenance,
        skill: skill.to_owned(),
        project: project.to_path_buf(),
        max_turns,
        advanced_research,
    })
}

/// Forwards each non-blank line of `input` to claude as a user message until EOF. While the
/// writer is empty because claude is waiting out a usage limit, the line waits for the resumed
/// process; the thread ends once `drive` has let go of the writer.
pub fn forward_input(writer: Writer, input: Box<dyn BufRead + Send>) {
    forward_input_keeping(writer, input, None);
}

pub(super) fn forward_input_keeping(
    writer: Writer,
    mut input: Box<dyn BufRead + Send>,
    hand_back: Option<HandBack>,
) {
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                continue;
            }
            loop {
                let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
                match guard.as_mut() {
                    Some(stdin) => {
                        if send(stdin, line).is_err() {
                            return;
                        }
                        break;
                    }
                    None if Arc::strong_count(&writer) == 1 => {
                        if let Some(hand_back) = hand_back {
                            hand_back(line.to_owned());
                        }
                        return;
                    }
                    None => {
                        drop(guard);
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        }
    });
}

fn wait_until(config: &Config, at: i64, stop: &AtomicBool) -> bool {
    loop {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        let remaining = at - (config.clock.now)();
        if remaining <= 0 {
            return true;
        }
        (config.clock.sleep)(Duration::from_secs(remaining.min(1) as u64));
    }
}

/// Runs the session to its end: renders to `out`, hands every parsed event to `observe`,
/// keeps an active goal going, and kills the process tree once `stop` is set. When claude
/// reports a usage limit with a reset time, waits for it (interruptible through `stop`),
/// resumes the same session into the same writer, and tells `observe` with a
/// `{"type":"chronos","subtype":"limit_wait","session":..,"until":..}` event.
pub fn drive(
    spawned: Spawned,
    config: &Config,
    out: &mut dyn Write,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Value),
) -> Res<i32> {
    let writer = Arc::clone(&spawned.writer);
    let (skill, project, max_turns) = (
        spawned.skill.clone(),
        spawned.project.clone(),
        spawned.max_turns,
    );
    let mut spawned = spawned;
    loop {
        let outcome = drive_once(spawned, config, out, stop, observe)?;
        let Some((session, reset)) = outcome.limit else {
            return Ok(outcome.code);
        };
        let now = (config.clock.now)();
        let at = reset.max(now) + LIMIT_MARGIN_SECS;
        let wait = at - now;
        let _ = writeln!(
            out,
            "usage limit: continuing at {} (in {}h{:02}m); Ctrl-C to stop",
            clock_time(at, local_offset_secs(at)),
            wait / 3600,
            wait / 60 % 60
        );
        let _ = out.flush();
        observe(&json!({"type":"chronos","subtype":"limit_wait","session":session,"until":at}));
        if !wait_until(config, at, stop) {
            let _ = writeln!(out, "interrupted; resume: axiom wish --resume {session}");
            let _ = out.flush();
            return Ok(130);
        }
        let prompt = continue_prompt(config, &session)?;
        let mut next = spawn(config, &skill, &prompt, &project, max_turns, Some(&session))?;
        *writer.lock().unwrap_or_else(|e| e.into_inner()) =
            next.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
        next.writer = Arc::clone(&writer);
        spawned = next;
    }
}
struct Outcome {
    code: i32,
    limit: Option<(String, i64)>,
}
fn drive_once(
    spawned: Spawned,
    config: &Config,
    out: &mut dyn Write,
    stop: &AtomicBool,
    observe: &mut dyn FnMut(&Value),
) -> Res<Outcome> {
    let Spawned {
        mut child,
        writer,
        lines: rx,
        mut log,
        log_path,
        provenance,
        advanced_research,
        ..
    } = spawned;
    let mut renderer = Renderer::new();
    let mut session = None::<String>;
    let mut final_result = false;
    let mut goal_error = None;
    let mut cause = None;
    let mut reset = None;
    let mut last_refusal = None::<String>;
    let idle_limit = goal::progress::idle_limit();
    let result: Res<i32> = loop {
        if stop.load(Ordering::SeqCst) {
            base::proc::kill_tree(&mut child);
            cause = Some("interrupted by the user".to_string());
            break Ok(130);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                if let Err(e) = log.write_all(&bytes) {
                    base::proc::kill_tree(&mut child);
                    break Err(failure(e));
                }
                let Ok(event) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                if event["type"] == "system" && event["subtype"] == "init" {
                    session = event["session_id"].as_str().map(str::to_owned);
                    if let (true, Some(id)) = (advanced_research, session.as_deref()) {
                        council::remember_switch(&config.home, id);
                    }
                }
                renderer.event(&event, out);
                observe(&event);
                if event["parent_tool_use_id"].is_null() {
                    if let Some(blocks) = event["message"]["content"].as_array() {
                        for block in blocks {
                            if block["type"] == "tool_result" && block["is_error"] == true {
                                let text =
                                    block["content"].as_str().map(str::to_owned).or_else(|| {
                                        block["content"].as_array().map(|blocks| {
                                            blocks
                                                .iter()
                                                .filter_map(|b| b["text"].as_str())
                                                .collect::<Vec<_>>()
                                                .join("\n")
                                        })
                                    });
                                if let Some(text) = text.filter(|s| !s.trim().is_empty()) {
                                    last_refusal = Some(text.chars().take(300).collect());
                                }
                            }
                        }
                    }
                }
                if event["type"] != "result" {
                    continue;
                }
                if event["is_error"].as_bool().unwrap_or(false) {
                    let message = event["result"].as_str().unwrap_or("");
                    let now = (config.clock.now)();
                    reset = limit_reset(message, now, local_offset_secs(now));
                    cause = Some(format!(
                        "claude reported an error: {}",
                        message.chars().take(200).collect::<String>()
                    ));
                    final_result = true;
                } else if let Some(id) = session.as_deref() {
                    match goal::load(&config.home, id) {
                        Ok(Some(g)) if g.status == "active" => {
                            let reason = hook::stop_reason_in(&config.home, id, &g);
                            let idle = match goal::progress::idle_continues(&config.home, id) {
                                Ok(idle) => idle,
                                Err(e) => {
                                    base::proc::kill_tree(&mut child);
                                    cause = Some(format!("could not track goal progress: {e}"));
                                    break Err(e);
                                }
                            };
                            if idle >= idle_limit {
                                let quote: String = last_refusal
                                    .as_deref()
                                    .unwrap_or(&reason)
                                    .chars()
                                    .take(300)
                                    .collect();
                                let note = format!("core made no progress for {idle} continues; last refusal or stop reason: \"{quote}\"");
                                match goal::interrupt(&config.home, id, &note) {
                                    Ok(Some(blocked)) => {
                                        let _ = writeln!(
                                            out,
                                            "goal blocked: {}",
                                            blocked.note.as_deref().unwrap_or("")
                                        );
                                    }
                                    Ok(None) => (),
                                    Err(e) => goal_error = Some(e),
                                }
                            } else {
                                let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(stdin) = guard.as_mut() {
                                    if let Err(e) = send(stdin, &format!("{CONTINUE_TAG} {reason}"))
                                    {
                                        if !pipe_closed(&e) {
                                            base::proc::kill_tree(&mut child);
                                            break Err(failure(e));
                                        }
                                        // claude is already exiting; its end is handled below
                                        // when its output closes, as if the message had been read.
                                        guard.take();
                                    }
                                }
                                continue;
                            }
                        }
                        Ok(Some(g)) if g.status == "blocked" || g.status == "waiting" => {
                            let _ = writeln!(
                                out,
                                "goal {}: {}",
                                g.status,
                                g.note.as_deref().unwrap_or("")
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            goal_error = Some(e);
                        }
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
                cause = Some(format!("claude exited {code} without a result"));
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
    let mut limit = None;
    if let (Some(id), Some(cause)) = (session.as_deref(), cause.as_deref()) {
        match goal::interrupt(&config.home, id, cause) {
            Ok(Some(g)) => {
                let _ = writeln!(out, "goal blocked: {}", g.note.unwrap_or_default());
                match reset {
                    Some(at) => limit = Some((id.to_owned(), at)),
                    None => {
                        let _ = writeln!(out, "resume: axiom wish --resume {id}");
                    }
                }
            }
            Ok(None) => (),
            Err(e) => goal_error = Some(e),
        }
    }
    let _ = writeln!(out, "\nlog: {}", log_path.display());
    let _ = writeln!(out, "provenance: {}", provenance.line());
    let mut result = result;
    if let Some(e) = goal_error {
        let _ = writeln!(out, "goal error: {}: {}", e.kind, e.message);
        if matches!(result, Ok(0)) {
            result = Ok(1);
        }
    }
    let _ = out.flush();
    result.map(|code| Outcome { code, limit })
}

#[cfg(test)]
mod tests {
    use super::pipe_closed;
    use std::io::{Error, ErrorKind};

    #[test]
    fn a_closing_reader_is_not_a_failure_but_other_errors_are() {
        assert!(pipe_closed(&Error::from(ErrorKind::BrokenPipe)));
        assert!(!pipe_closed(&Error::from(ErrorKind::PermissionDenied)));
        assert!(!pipe_closed(&Error::other("disk full")));
        #[cfg(windows)]
        {
            assert!(pipe_closed(&Error::from_raw_os_error(232)));
            assert!(pipe_closed(&Error::from_raw_os_error(109)));
            assert!(!pipe_closed(&Error::from_raw_os_error(5)));
        }
    }
}
