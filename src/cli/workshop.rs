use crate::{
    base::{self, Failure, Res},
    codex,
    seams::TtyApproval,
    workshop::{
        api::Api,
        host::{Host, HostConfig, Status},
        services::{
            BackendHeldSlots, ClaudeBackend, CodexBackend, ModelBackend, ModelCall, ModelReply,
            NoNetwork, NoRouter, Services,
        },
    },
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

struct LazyBackend {
    codex: bool,
    backend: Mutex<Option<Arc<dyn ModelBackend>>>,
}
impl ModelBackend for LazyBackend {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        let backend = {
            let mut slot = self.backend.lock().unwrap();
            if slot.is_none() {
                let (env, executable) = if self.codex {
                    ("AXIOM_CODEX_CMD", "codex")
                } else {
                    ("AXIOM_CLAUDE_CMD", "claude")
                };
                let command = base::resolve_command(env, executable)
                    .map_err(|e| Failure::new("unavailable", e.message))?;
                *slot = Some(if self.codex {
                    Arc::new(CodexBackend {
                        server: Arc::new(codex::AppServer::new(command)),
                    })
                } else {
                    Arc::new(ClaudeBackend { command })
                });
            }
            slot.as_ref().unwrap().clone()
        };
        backend.call(call, cancel)
    }
}
fn host() -> Host {
    let backend = |codex| {
        Arc::new(LazyBackend {
            codex,
            backend: Mutex::new(None),
        }) as Arc<dyn ModelBackend>
    };
    let services = Services {
        engine_home: codex::slots::home(),
        // CodexBackend already acquires the global Codex slot.
        slots: Arc::new(BackendHeldSlots),
        claude: backend(false),
        codex: backend(true),
        router: Arc::new(NoRouter),
        network: Arc::new(NoNetwork),
    };
    Host::new(
        HostConfig {
            home: base::axiom_home(),
            python: None,
        },
        Arc::new(Api::new(services)),
        Arc::new(TtyApproval),
    )
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_LOCK: Mutex<()> = Mutex::new(());
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
fn run(host: &Host, name: &str) -> Res<i32> {
    let _lock = SIGNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _signals = Signals::install();
    println!("{}", serde_json::to_string(&host.start(name)?).unwrap());
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            if let Err(e) = host.stop(name) {
                eprintln!("axiom: {}", e.message);
            }
            return Ok(130);
        }
        let state = host.status(name)?;
        if state.status != Status::Running {
            return Ok(
                if state.status == Status::Stopped
                    || (state.status == Status::Exited && state.exit_code == Some(0))
                {
                    0
                } else {
                    1
                },
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}
pub(super) fn command(args: &[String]) -> i32 {
    let valid = match args {
        [action] => action == "list",
        [action, name] => {
            matches!(action.as_str(), "status" | "kill" | "revive" | "run")
                && !name.starts_with('-')
        }
        _ => false,
    };
    if !valid {
        return super::usage("workshop: invalid command or arguments");
    }
    let host = host();
    let result = match args[0].as_str() {
        "list" => host.list().map(|state| {
            println!("{}", serde_json::to_string(&state).unwrap());
            0
        }),
        "run" => run(&host, &args[1]),
        action => {
            let state = match action {
                "status" => host.status(&args[1]),
                "kill" => host.kill(&args[1]),
                _ => host.revive(&args[1]),
            };
            state.map(|state| {
                println!("{}", serde_json::to_string(&state).unwrap());
                0
            })
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
