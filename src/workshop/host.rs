use super::{
    approval, manifest,
    permissions::{self, DataDirs, Grants},
    Manifest,
};
use crate::{
    base::{self, proc, Failure, Res},
    seams::PersonApproval,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub struct HostConfig {
    pub home: PathBuf,
    pub python: Option<String>,
}
pub struct ModuleCtx {
    pub name: String,
    pub manifest: Manifest,
    pub grants: Grants,
    pub dirs: DataDirs,
    pub cancel: Arc<AtomicBool>,
}
pub trait RequestHandler: Send + Sync {
    fn handle(&self, ctx: &ModuleCtx, request: Value) -> Value;
}
pub struct AckHandler;
impl RequestHandler for AckHandler {
    fn handle(&self, _: &ModuleCtx, request: Value) -> Value {
        json!({"v":1,"id":request.get("id").cloned().unwrap_or(Value::Null),"ok":true,"result":null})
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModuleState {
    pub name: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub killed: bool,
    pub exit_code: Option<i32>,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Stopped,
    Running,
    Exited,
    Killed,
}

type Input = Arc<Mutex<Option<ChildStdin>>>;
struct Running {
    child: Arc<Mutex<Child>>,
    input: Input,
    ctx: Arc<ModuleCtx>,
    pid: u32,
}
pub struct Host {
    config: HostConfig,
    handler: Arc<dyn RequestHandler>,
    approval: Arc<dyn PersonApproval>,
    running: Mutex<HashMap<String, Running>>,
}
fn io_error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn initial(name: &str) -> ModuleState {
    ModuleState {
        name: name.into(),
        status: Status::Stopped,
        pid: None,
        started_at: None,
        killed: false,
        exit_code: None,
    }
}
fn read_state(dirs: &DataDirs, name: &str) -> Res<ModuleState> {
    match fs::read(dirs.root.join("state.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io_error),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(initial(name)),
        Err(e) => Err(io_error(e)),
    }
}
fn write_state(dirs: &DataDirs, state: &ModuleState) -> Res<()> {
    fs::create_dir_all(&dirs.root).map_err(io_error)?;
    let bytes = serde_json::to_vec_pretty(state).map_err(io_error)?;
    base::atomic_write(&dirs.root.join("state.json"), &bytes).map_err(io_error)
}
fn refresh(dirs: &DataDirs, name: &str, child: Option<&Arc<Mutex<Child>>>) -> Res<ModuleState> {
    let mut state = read_state(dirs, name)?;
    if state.killed {
        state.status = Status::Killed;
        return Ok(state);
    }
    if state.status == Status::Running {
        let exit = if let Some(child) = child {
            child
                .lock()
                .unwrap()
                .try_wait()
                .map_err(io_error)?
                .map(|s| s.code())
        } else if state.pid.is_some_and(proc::pid_alive) {
            None
        } else {
            Some(None)
        };
        if let Some(code) = exit {
            // A kill from another host takes precedence over a normal exit.
            let current = read_state(dirs, name)?;
            if current.killed || current.status != Status::Running || current.pid != state.pid {
                return Ok(current);
            }
            state.status = Status::Exited;
            state.exit_code = code;
            write_state(dirs, &state)?;
        }
    }
    Ok(state)
}
// Some restricted Windows environments deny taskkill even when the process handles
// themselves are accessible. Enumerate descendants before terminating them in that case.
#[cfg(windows)]
fn windows_kill_tree(pid: u32) -> Res<()> {
    use std::{ffi::c_void, mem, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
    };
    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        id: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
    }
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(2, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(Failure::new(
                "kill_failed",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let mut entry: ProcessEntry = mem::zeroed();
        entry.size = mem::size_of::<ProcessEntry>() as u32;
        let mut pairs = Vec::new();
        let mut has = Process32FirstW(snapshot, &mut entry);
        while has != 0 {
            pairs.push((entry.id, entry.parent));
            has = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut tree = vec![pid];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            for &(id, ppid) in &pairs {
                if ppid == parent && !tree.contains(&id) {
                    tree.push(id);
                }
            }
            i += 1;
        }
        for id in tree.into_iter().rev() {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, id);
            if handle == ptr::null_mut() {
                if proc::pid_alive(id) {
                    return Err(Failure::new(
                        "kill_failed",
                        std::io::Error::last_os_error().to_string(),
                    ));
                }
                continue;
            }
            let ok = TerminateProcess(handle, 1);
            let error = std::io::Error::last_os_error();
            CloseHandle(handle);
            if ok == 0 && proc::pid_alive(id) {
                return Err(Failure::new("kill_failed", error.to_string()));
            }
        }
    }
    Ok(())
}
fn kill_pid_tree(pid: u32) -> Res<()> {
    match proc::kill_pid_tree(pid) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(_) => windows_kill_tree(pid),
        #[cfg(not(windows))]
        Err(e) => Err(e),
    }
}
impl Host {
    pub fn new(
        mut config: HostConfig,
        handler: Arc<dyn RequestHandler>,
        approval: Arc<dyn PersonApproval>,
    ) -> Host {
        if let Ok(home) = std::path::absolute(&config.home) {
            config.home = home;
        }
        Host {
            config,
            handler,
            approval,
            running: Mutex::new(HashMap::new()),
        }
    }
    fn module(&self, name: &str) -> Res<(PathBuf, Manifest, DataDirs)> {
        base::require_slug("module name", name)?;
        let dir = manifest::modules_root(&self.config.home).join(name);
        let m = manifest::load(&dir)?;
        Ok((dir, m, DataDirs::new(&self.config.home, name)))
    }
    pub fn start(&self, name: &str) -> Res<ModuleState> {
        let mut running = self.running.lock().unwrap();
        let (dir, manifest, dirs) = self.module(name)?;
        let own = running.get(name);
        let state = refresh(
            &dirs,
            name,
            own.filter(|r| Some(r.pid) == read_state(&dirs, name).ok().and_then(|s| s.pid))
                .map(|r| &r.child),
        )?;
        if state.killed {
            return Err(Failure::new("killed", format!("module {name} is killed")));
        }
        if state.status == Status::Running {
            return Err(Failure::new("busy", format!("module {name} is running")));
        }
        let grants = Grants::from_manifest(&manifest)?;
        approval::ensure_approved(&dirs, &manifest, &grants, self.approval.as_ref())?;
        dirs.ensure()?;
        let mut argv = manifest.entry.clone();
        let is_python = argv[0] == "python";
        if is_python {
            argv[0] = match &self.config.python {
                Some(p) => p.clone(),
                None => ["python3", "python"]
                    .into_iter()
                    .find(|p| {
                        proc::command(&[(*p).into(), "--version".into()])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .is_ok_and(|s| s.success())
                    })
                    .ok_or_else(|| Failure::new("process", "python not found"))?
                    .into(),
            };
        }
        for arg in argv.iter_mut().skip(usize::from(is_python)) {
            let path = Path::new(arg.as_str());
            if path.is_relative() && dir.join(path).is_file() {
                *arg = fs::canonicalize(dir.join(path))
                    .map_err(io_error)?
                    .to_string_lossy()
                    .into_owned();
            }
        }
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dirs.logs.join("stderr.log"))
            .map_err(io_error)?;
        let mut command = proc::command(&argv);
        command
            .env_clear()
            .envs(permissions::child_env(name, &grants, &dirs))
            .current_dir(&dirs.files)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        let mut child =
            proc::spawn_group(&mut command).map_err(|e| Failure::new("process", e.to_string()))?;
        let pid = child.id();
        let input = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().expect("piped stdout");
        let child = Arc::new(Mutex::new(child));
        let ctx = Arc::new(ModuleCtx {
            name: name.into(),
            manifest,
            grants,
            dirs: dirs.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let state = ModuleState {
            name: name.into(),
            status: Status::Running,
            pid: Some(pid),
            started_at: Some(base::now_iso()),
            killed: false,
            exit_code: None,
        };
        if let Err(e) = write_state(&dirs, &state) {
            proc::kill_tree(&mut child.lock().unwrap());
            return Err(e);
        }
        let handler = Arc::clone(&self.handler);
        let read_ctx = Arc::clone(&ctx);
        let read_input = Arc::clone(&input);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if let Ok(request) = serde_json::from_slice::<Value>(&line) {
                    let handler = Arc::clone(&handler);
                    let ctx = Arc::clone(&read_ctx);
                    let input = Arc::clone(&read_input);
                    thread::spawn(move || {
                        let reply = handler.handle(&ctx, request);
                        if let Some(stdin) = input.lock().unwrap().as_mut() {
                            let _ = writeln!(stdin, "{reply}");
                            let _ = stdin.flush();
                        }
                    });
                } else if let Ok(mut log) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(read_ctx.dirs.logs.join("stdout.log"))
                {
                    let _ = log.write_all(&line);
                    if !line.ends_with(b"\n") {
                        let _ = log.write_all(b"\n");
                    }
                }
            }
        });
        let watch_child = Arc::clone(&child);
        let watch_ctx = Arc::clone(&ctx);
        thread::spawn(move || loop {
            if let Ok(state) = read_state(&watch_ctx.dirs, &watch_ctx.name) {
                if state.killed {
                    watch_ctx.cancel.store(true, Ordering::SeqCst);
                }
            }
            let exited = watch_child
                .lock()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .is_some();
            if exited {
                if read_state(&watch_ctx.dirs, &watch_ctx.name).is_ok_and(|s| s.pid == Some(pid)) {
                    let _ = refresh(&watch_ctx.dirs, &watch_ctx.name, Some(&watch_child));
                }
                if read_state(&watch_ctx.dirs, &watch_ctx.name).is_ok_and(|s| s.killed) {
                    watch_ctx.cancel.store(true, Ordering::SeqCst);
                }
                break;
            }
            thread::sleep(Duration::from_millis(200));
        });
        running.insert(
            name.into(),
            Running {
                child,
                input,
                ctx,
                pid,
            },
        );
        Ok(state)
    }
    pub fn status(&self, name: &str) -> Res<ModuleState> {
        let (_, _, dirs) = self.module(name)?;
        let running = self.running.lock().unwrap();
        let state = read_state(&dirs, name)?;
        let own = running.get(name).filter(|r| Some(r.pid) == state.pid);
        refresh(&dirs, name, own.map(|r| &r.child))
    }
    pub fn list(&self) -> Res<Vec<ModuleState>> {
        manifest::discover(&self.config.home)?
            .into_iter()
            .map(|(_, m)| self.status(&m.name))
            .collect()
    }
    pub fn stop(&self, name: &str) -> Res<ModuleState> {
        let (_, _, dirs) = self.module(name)?;
        let running = self.running.lock().unwrap();
        let mut state = read_state(&dirs, name)?;
        if state.killed {
            state.status = Status::Killed;
            return Ok(state);
        }
        let own = running.get(name).filter(|r| Some(r.pid) == state.pid);
        if let Some(r) = own {
            r.input.lock().unwrap().take();
        }
        if state.status == Status::Running {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let exited = if let Some(r) = own {
                    r.child
                        .lock()
                        .unwrap()
                        .try_wait()
                        .map_err(io_error)?
                        .is_some()
                } else {
                    !state.pid.is_some_and(proc::pid_alive)
                };
                if exited {
                    break;
                }
                if Instant::now() >= deadline {
                    if let Some(r) = own {
                        kill_pid_tree(r.pid)?;
                        let _ = r.child.lock().unwrap().wait();
                    } else if let Some(pid) = state.pid {
                        kill_pid_tree(pid)?;
                        let _ = proc::wait_pid_exit(pid, Duration::from_secs(5));
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        let current = read_state(&dirs, name)?;
        if current.killed {
            return Ok(current);
        }
        state.status = Status::Stopped;
        state.pid = None;
        state.exit_code = None;
        write_state(&dirs, &state)?;
        Ok(state)
    }
    pub fn kill(&self, name: &str) -> Res<ModuleState> {
        let (_, _, dirs) = self.module(name)?;
        let running = self.running.lock().unwrap();
        let mut state = read_state(&dirs, name)?;
        if let Some(r) = running
            .get(name)
            .filter(|r| state.pid.is_none() || Some(r.pid) == state.pid)
        {
            r.ctx.cancel.store(true, Ordering::SeqCst);
            r.input.lock().unwrap().take();
        }
        let pid_to_kill = if state.status == Status::Running {
            state.pid
        } else {
            None
        };
        state.killed = true;
        state.status = Status::Killed;
        write_state(&dirs, &state)?;
        if let Some(pid) = pid_to_kill {
            if proc::pid_alive(pid) {
                kill_pid_tree(pid)?;
            }
            if let Some(r) = running.get(name).filter(|r| r.pid == pid) {
                let _ = r.child.lock().unwrap().wait();
            } else {
                let _ = proc::wait_pid_exit(pid, Duration::from_secs(3));
            }
        }
        Ok(state)
    }
    pub fn revive(&self, name: &str) -> Res<ModuleState> {
        let (_, _, dirs) = self.module(name)?;
        let mut running = self.running.lock().unwrap();
        let mut state = read_state(&dirs, name)?;
        state.killed = false;
        state.status = Status::Stopped;
        state.pid = None;
        state.exit_code = None;
        write_state(&dirs, &state)?;
        running.remove(name);
        Ok(state)
    }
    pub fn ctx(&self, name: &str) -> Option<Arc<ModuleCtx>> {
        self.running
            .lock()
            .unwrap()
            .get(name)
            .map(|r| Arc::clone(&r.ctx))
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let names: Vec<_> = self
            .running
            .lock()
            .unwrap()
            .iter()
            .map(|(name, r)| (name.clone(), r.pid))
            .collect();
        for (name, pid) in names {
            let dirs = DataDirs::new(&self.config.home, &name);
            if read_state(&dirs, &name)
                .is_ok_and(|s| s.pid == Some(pid) && s.status == Status::Running)
            {
                let _ = self.stop(&name);
            }
        }
    }
}
