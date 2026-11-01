use super::Config;
use crate::{
    base::{self, Failure, Res},
    bridge::{
        self,
        dispatch::{DispatchAction, DispatchOptions, DispatchRequest, RootContext, RunStatus},
    },
};
use std::{
    io::{BufRead, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicPtr, Ordering},
        Arc, Mutex,
    },
};

static SIGNAL_LOCK: Mutex<()> = Mutex::new(());
static SIGNAL_TARGET: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());
fn mark_interrupt() {
    let p = SIGNAL_TARGET.load(Ordering::SeqCst);
    if !p.is_null() {
        unsafe {
            (*p).store(true, Ordering::SeqCst);
        }
    }
}
#[doc(hidden)]
pub fn trigger_interrupt() {
    mark_interrupt();
}
#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    mark_interrupt();
}
#[cfg(windows)]
unsafe extern "system" fn interrupt(kind: u32) -> i32 {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        mark_interrupt();
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
    fn install(cancel: &Arc<AtomicBool>) -> Self {
        SIGNAL_TARGET.store(Arc::as_ptr(cancel) as *mut AtomicBool, Ordering::SeqCst);
        #[cfg(unix)]
        unsafe {
            let mut a: libc::sigaction = std::mem::zeroed();
            a.sa_sigaction = interrupt as usize;
            libc::sigemptyset(&mut a.sa_mask);
            let mut old_int = std::mem::zeroed();
            let mut old_term = std::mem::zeroed();
            libc::sigaction(libc::SIGINT, &a, &mut old_int);
            libc::sigaction(libc::SIGTERM, &a, &mut old_term);
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
        SIGNAL_TARGET.store(std::ptr::null_mut(), Ordering::SeqCst);
    }
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

pub fn dispatch_request(
    bridge: &bridge::Bridge,
    root: &RootContext,
    request: &DispatchRequest,
    options: &DispatchOptions,
    out: &mut dyn Write,
) -> Res<i32> {
    let _lock = SIGNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _signals = Signals::install(&options.cancel);
    writeln!(
        out,
        "{}",
        serde_json::json!({"request_id":request.request_id})
    )
    .and_then(|_| out.flush())
    .map_err(|e| Failure::new("io", e.to_string()))?;
    let result = bridge.dispatch(root, request, options, None)?;
    let code = if matches!(
        result.status,
        RunStatus::Blocked | RunStatus::OutcomeUnknown
    ) {
        1
    } else {
        0
    };
    serde_json::to_writer(
        &mut *out,
        &serde_json::json!({"request_id":request.request_id,"result":result}),
    )
    .map_err(|e| Failure::new("io", e.to_string()))?;
    writeln!(out).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(code)
}
pub fn dispatch_action(
    config: &Config,
    project: &Path,
    action: DispatchAction,
    request_id: String,
    out: &mut dyn Write,
) -> Res<i32> {
    let repo_root = project
        .canonicalize()
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    let root = RootContext {
        project: base::slug_for_dir(&repo_root),
        repo_root,
    };
    let mut bc = bridge::Config::from_env();
    if bc.data_home.is_none() {
        bc.data_home = Some(config.home.clone());
    }
    let bridge = bridge::Bridge::new(bc);
    let request = DispatchRequest { request_id, action };
    let options = DispatchOptions {
        max_wait_ms: 0,
        cancel_grace_ms: 10_000,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    dispatch_request(&bridge, &root, &request, &options, out)
}
pub fn resume(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    _input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    if max_turns.is_some() {
        return Err(Failure::new(
            "blocked_capability",
            "checkpoint dispatch does not support a turn limit",
        ));
    }
    let cwd = std::env::current_dir().map_err(|e| Failure::new("io", e.to_string()))?;
    dispatch_action(
        config,
        &cwd,
        DispatchAction::Resume {
            run_id: session.into(),
            recovery: None,
        },
        base::unique(),
        out,
    )
}
pub fn run(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    _input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    if max_turns.is_some() {
        return Err(Failure::new(
            "blocked_capability",
            "checkpoint dispatch does not support a turn limit",
        ));
    }
    dispatch_action(
        config,
        project,
        DispatchAction::Start {
            intent: if skill == "research" {
                format!("Research only; stop at the research review boundary. {text}")
            } else {
                text.into()
            },
        },
        base::unique(),
        out,
    )
}
