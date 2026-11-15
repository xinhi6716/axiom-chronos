use super::{
    lock, login,
    protocol::{HostedWish, LogRecord, Snapshot, StartRequest, WireEvent},
    recover,
    scheduler::{self, SchedulerConfig},
    tray,
};
use crate::{
    base::{
        self,
        ipc::{self, Conn, Endpoint, Listener},
        Failure, Res,
    },
    host, session,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct ServerOptions {
    pub home: PathBuf,
    pub config: Arc<dyn Fn() -> session::Config + Send + Sync>,
    pub tray: Box<dyn tray::Tray>,
    pub desktop: Box<dyn tray::Desktop>,
    pub login: Box<dyn login::LoginStart>,
    pub login_launch: bool,
}
pub struct Server;
pub struct RunningService {
    pub endpoint: Endpoint,
    actions: mpsc::Sender<tray::TrayAction>,
    thread: Option<thread::JoinHandle<i32>>,
    state: Option<Arc<Mutex<State>>>,
    scheduler: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}
impl RunningService {
    pub fn tray_actions(&self) -> mpsc::Sender<tray::TrayAction> {
        self.actions.clone()
    }
    pub fn quit(&self, stop_running: bool) -> Res<()> {
        if let Some(state) = &self.state {
            state.lock().unwrap().quit(stop_running)
        } else {
            Ok(())
        }
    }
    pub fn wait(mut self) -> i32 {
        let code = self
            .thread
            .take()
            .map(|t| t.join().unwrap_or(1))
            .unwrap_or(0);
        if let Some(state) = &self.state {
            let _ = state.lock().unwrap().scheduler_stop.send(());
        }
        self.join_scheduler();
        code
    }
    fn join_scheduler(&self) {
        if let Some(handle) = self.scheduler.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}
struct Item {
    view: HostedWish,
    wish: Option<host::Wish>,
    request: Option<StartRequest>,
    user_stopped: bool,
    recovered: bool,
}
struct State {
    home: PathBuf,
    instance: String,
    config: Arc<dyn Fn() -> session::Config + Send + Sync>,
    tray: Box<dyn tray::Tray>,
    desktop: Box<dyn tray::Desktop>,
    login: Box<dyn login::LoginStart>,
    login_start: bool,
    items: Vec<Item>,
    events: VecDeque<WireEvent>,
    cursor: u64,
    quitting: bool,
    quit_deadline: Option<Instant>,
    scheduler_stop: mpsc::Sender<()>,
    last_count: usize,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn settings(home: &Path) -> Res<bool> {
    match std::fs::read(home.join("service/settings.json")) {
        Ok(b) => {
            let v: Value = serde_json::from_slice(&b).map_err(io)?;
            Ok(v["login_start"].as_bool().unwrap_or(false))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io(e)),
    }
}
impl Server {
    pub fn start(opts: ServerOptions) -> Res<RunningService> {
        let scheduler = SchedulerConfig::production(&opts.home);
        Self::start_with_scheduler(opts, scheduler)
    }
    pub fn start_with_scheduler(
        mut opts: ServerOptions,
        scheduler_config: SchedulerConfig,
    ) -> Res<RunningService> {
        std::fs::create_dir_all(opts.home.join("service")).map_err(io)?;
        let home = opts.home.canonicalize().map_err(io)?;
        let endpoint = ipc::endpoint_for(&home)?;
        let (tx, rx) = mpsc::channel();
        let enabled = settings(&home)?;
        if opts.login_launch && !enabled {
            return Ok(RunningService {
                endpoint,
                actions: tx,
                thread: None,
                state: None,
                scheduler: Arc::new(Mutex::new(None)),
            });
        }
        let guard = lock::acquire(&home)?;
        let listener = Listener::bind(&endpoint)?;
        let instance = format!("{}-{}", std::process::id(), now());
        let mut store = recover::load(&home)?;
        recover::recover(&home, &mut store)?;
        let mut items = Vec::new();
        for rec in store.sessions {
            let view = HostedWish {
                key: rec.session.clone().unwrap_or_else(|| rec.handle.clone()),
                handle: rec.handle,
                session: rec.session,
                text: rec.text,
                cwd: rec.cwd,
                started_unix: now() / 1000,
                state: rec.state,
                limit_until: None,
                ended_at: None,
                ending: String::new(),
                disk_line: None,
                log: vec![],
            };
            items.push(Item {
                view,
                wish: None,
                request: None,
                user_stopped: rec.user_stopped,
                recovered: rec.recovered,
            });
        }
        opts.tray.update(&tray::TrayStatus {
            running: 0,
            text: tray::status_text(0),
        });
        let (scheduler_stop, scheduler_rx) = mpsc::channel();
        let mut state = State {
            home: home.clone(),
            instance: instance.clone(),
            config: opts.config,
            tray: opts.tray,
            desktop: opts.desktop,
            login: opts.login,
            login_start: enabled,
            items,
            events: VecDeque::new(),
            cursor: 0,
            quitting: false,
            quit_deadline: None,
            scheduler_stop,
            last_count: 0,
        };
        if opts.login_launch {
            for i in 0..state.items.len() {
                if state.items[i].recovered
                    && !state.items[i].user_stopped
                    && state.items[i].view.state == "ended"
                {
                    if let Some(id) = state.items[i].view.session.clone() {
                        if crate::goal::load(&home, &id)?.is_some_and(|g| {
                            g.status == "blocked" && g.reason.as_deref() == Some("interrupted")
                        }) {
                            if let Ok(w) = host::Wish::resume((state.config)(), &id, None, None) {
                                state.items[i].wish = Some(w);
                                state.items[i].view.state = "starting".into();
                                state.items[i].recovered = false;
                            }
                        }
                    }
                }
            }
        }
        state.persist()?;
        state.update_tray();
        base::atomic_write(&home.join("service/service.json"),json!({"pid":std::process::id(),"started_ms":now(),"endpoint":endpoint.name,"instance":instance}).to_string().as_bytes()).map_err(io)?;
        let state = Arc::new(Mutex::new(state));
        let shared = state.clone();
        let scheduler_handle = Arc::new(Mutex::new(Some(scheduler::spawn(
            home.clone(),
            scheduler_config,
            scheduler_rx,
        ))));
        let scheduler_for_main = scheduler_handle.clone();
        let handle = thread::spawn(move || {
            let _guard = guard;
            let mut consecutive_errors = 0;
            loop {
                {
                    let mut s = shared.lock().unwrap();
                    s.pump();
                    while let Ok(action) = rx.try_recv() {
                        match action {
                            tray::TrayAction::Open => {
                                let _ = s.desktop.open();
                            }
                            tray::TrayAction::PauseAll => {
                                s.pause();
                            }
                            tray::TrayAction::Quit => {
                                let n = s.count();
                                if n == 0 {
                                    s.quitting = true;
                                    let _ = s.scheduler_stop.send(());
                                } else {
                                    s.emit("", None, "quit_requested", json!({"running":n}));
                                    let _ = s.desktop.confirm_quit(n);
                                }
                            }
                        }
                    }
                    // Keep accepting IPC until an already claimed scheduler launch completes.
                    if s.quitting
                        && scheduler_for_main
                            .lock()
                            .unwrap()
                            .as_ref()
                            .is_none_or(|h| h.is_finished())
                        && (s.count() == 0 || s.quit_deadline.is_some_and(|d| Instant::now() >= d))
                    {
                        // A wish can send Exited and finish after the pump above; record it
                        // before leaving, or its saved session never reaches "ended".
                        s.pump();
                        for item in &mut s.items {
                            if let Some(wish) = item.wish.take() {
                                thread::spawn(move || drop(wish));
                            }
                        }
                        s.tray.close();
                        break;
                    }
                }
                match listener.accept_timeout(Duration::from_millis(30)) {
                    Ok(Some(conn)) => {
                        consecutive_errors = 0;
                        let state = shared.clone();
                        thread::spawn(move || connection(conn, state));
                    }
                    Ok(None) => consecutive_errors = 0,
                    Err(e) => {
                        consecutive_errors += 1;
                        let _ =
                            std::fs::write(home.join("service/accept-error.txt"), e.to_string());
                        if consecutive_errors >= 200 {
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            }
            let _ = shared.lock().unwrap().scheduler_stop.send(());
            if let Some(handle) = scheduler_for_main.lock().unwrap().take() {
                let _ = handle.join();
            }
            0
        });
        Ok(RunningService {
            endpoint,
            actions: tx,
            thread: Some(handle),
            state: Some(state),
            scheduler: scheduler_handle,
        })
    }
}
impl State {
    fn count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.wish.as_ref().is_some_and(|w| w.is_running()))
            .count()
    }
    fn update_tray(&mut self) {
        let n = self.count();
        if n != self.last_count {
            self.last_count = n;
            self.tray.update(&tray::TrayStatus {
                running: n,
                text: tray::status_text(n),
            });
        }
    }
    fn persist(&self) -> Res<()> {
        let sessions = self
            .items
            .iter()
            .map(|i| recover::Session {
                handle: i.view.handle.clone(),
                session: i.view.session.clone(),
                cwd: i.view.cwd.clone(),
                text: i.view.text.clone(),
                state: i.view.state.clone(),
                user_stopped: i.user_stopped,
                recovered: i.recovered,
            })
            .collect();
        recover::save(
            &self.home,
            &recover::Store {
                instance: self.instance.clone(),
                pid: std::process::id(),
                sessions,
            },
        )
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            instance: self.instance.clone(),
            cursor: self.cursor,
            login_start: self.login_start,
            wishes: self.items.iter().map(|i| i.view.clone()).collect(),
        }
    }
    fn emit(&mut self, handle: &str, session: Option<String>, kind: &str, data: Value) {
        self.cursor += 1;
        self.events.push_back(WireEvent {
            seq: self.cursor,
            instance: self.instance.clone(),
            handle: handle.into(),
            session,
            kind: kind.into(),
            data,
        });
        if self.events.len() > 10_000 {
            self.events.pop_front();
        }
    }
    fn index(&self, key: &str) -> Res<usize> {
        self.items
            .iter()
            .position(|i| i.view.handle == key || i.view.session.as_deref() == Some(key))
            .ok_or_else(|| Failure::new("not_found", format!("no wish: {key}")))
    }
    fn launch(&mut self, req: StartRequest) -> Res<String> {
        let handle = format!("h-{}-{}", now(), self.items.len());
        let options = host::Options {
            creative: req.creative,
            max_turns: req.max_turns,
            advanced_research: req.advanced_research,
            codex_max_parallel: req.codex_max_parallel,
            need_bytes: req.need_bytes,
            ..Default::default()
        };
        let result = host::Wish::start((self.config)(), Path::new(&req.cwd), &req.wish, &options);
        let (wish, state, line) = match result {
            Ok(w) => (Some(w), "starting", None),
            Err(e) if e.kind == "disk_space" => (None, "waiting_for_space", Some(e.message)),
            Err(e) => return Err(e),
        };
        let view = HostedWish {
            key: handle.clone(),
            handle: handle.clone(),
            session: None,
            text: req.wish.clone(),
            cwd: req.cwd.clone(),
            started_unix: now() / 1000,
            state: state.into(),
            limit_until: None,
            ended_at: None,
            ending: String::new(),
            disk_line: line,
            log: vec![],
        };
        self.items.push(Item {
            view,
            wish,
            request: Some(req),
            user_stopped: false,
            recovered: false,
        });
        self.persist()?;
        self.update_tray();
        Ok(handle)
    }
    fn resume(&mut self, key: &str, note: Option<&str>) -> Res<String> {
        let i = self.index(key)?;
        if self.items[i].wish.as_ref().is_some_and(|w| w.is_running()) {
            return Err(Failure::new("running", "wish is already running"));
        }
        let result = if let Some(id) = self.items[i].view.session.clone() {
            host::Wish::resume((self.config)(), &id, note, None)
        } else if let Some(req) = self.items[i].request.as_ref() {
            let opts = host::Options {
                creative: req.creative,
                max_turns: req.max_turns,
                advanced_research: req.advanced_research,
                codex_max_parallel: req.codex_max_parallel,
                need_bytes: req.need_bytes,
                ..Default::default()
            };
            host::Wish::start((self.config)(), Path::new(&req.cwd), &req.wish, &opts)
        } else {
            return Err(Failure::new("not_found", "cannot resume without session"));
        };
        match result {
            Ok(w) => {
                self.items[i].wish = Some(w);
                self.items[i].view.state = "starting".into();
                self.items[i].view.disk_line = None;
                self.items[i].user_stopped = false;
                self.items[i].recovered = false;
            }
            Err(e) if e.kind == "disk_space" => {
                self.items[i].view.state = "waiting_for_space".into();
                self.items[i].view.disk_line = Some(e.message);
            }
            Err(e) => return Err(e),
        }
        self.persist()?;
        self.update_tray();
        Ok(self.items[i].view.handle.clone())
    }
    fn pause(&mut self) -> usize {
        let mut n = 0;
        for item in &mut self.items {
            if let Some(w) = &item.wish {
                if w.is_running() {
                    w.stop();
                    item.user_stopped = true;
                    item.recovered = false;
                    n += 1;
                }
            }
        }
        let _ = self.persist();
        n
    }
    fn quit(&mut self, stop: bool) -> Res<()> {
        if self.count() > 0 && !stop {
            return Err(Failure::new("running", "wishes are still running"));
        }
        if stop {
            self.pause();
        }
        self.quitting = true;
        let _ = self.scheduler_stop.send(());
        self.quit_deadline = Some(Instant::now() + Duration::from_secs(30));
        Ok(())
    }
    fn pump(&mut self) {
        let mut events = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            if let Some(w) = &item.wish {
                while let Ok(e) = w.events().try_recv() {
                    events.push((i, e));
                }
            }
        }
        let changed = !events.is_empty();
        for (i, e) in events {
            let item = &mut self.items[i];
            let (kind, data, log) = match e {
                host::Event::Started { session } => {
                    item.view.session = Some(session.clone());
                    item.view.key = session.clone();
                    item.view.state = "running".into();
                    item.view.limit_until = None;
                    ("started", json!({"session":session}), None)
                }
                host::Event::Text(t) => ("text", json!({"text":t}), Some(("assistant", t, "info"))),
                host::Event::ToolUse { name, label } => {
                    ("tool_use", json!({"name":name,"label":label}), None)
                }
                host::Event::ToolResult { name, summary } => {
                    ("tool_result", json!({"name":name,"summary":summary}), None)
                }
                host::Event::Result { text, is_error } => {
                    ("result", json!({"text":text,"is_error":is_error}), None)
                }
                host::Event::Waiting { until } => {
                    item.view.state = "limit_wait".into();
                    item.view.limit_until = Some(until);
                    ("waiting", json!({"until":until}), None)
                }
                host::Event::DiskPaused { line } => {
                    item.view.state = "waiting_for_space".into();
                    item.view.disk_line = Some(line.clone());
                    ("disk_paused", json!({"line":line}), None)
                }
                host::Event::Exited { code } => {
                    if item.view.state != "waiting_for_space" {
                        item.view.state = "ended".into();
                    }
                    item.view.ended_at = Some(base::now_iso());
                    item.view.ending = format!("exit {code}");
                    ("exited", json!({"code":code}), None)
                }
                host::Event::Failed(e) => {
                    item.view.state = "ended".into();
                    item.view.ended_at = Some(base::now_iso());
                    item.view.ending = e.message.clone();
                    ("failed", e.to_json(), None)
                }
                host::Event::RateLimit(q) => {
                    let window = |w: Option<crate::usage::quota::Window>| {
                        w.map(|w|json!({"used":w.used,"resets_at":w.resets_at,"observed_at":w.observed_at,"status":w.status}))
                    };
                    (
                        "rate_limit",
                        json!({"observed_at":q.observed_at,"present":q.present,"five_hour":window(q.five_hour),"weekly":window(q.weekly)}),
                        None,
                    )
                }
            };
            if let Some((source, text, level)) = log {
                item.view.log.push(LogRecord {
                    time_ms: now() as i64,
                    source: source.into(),
                    text,
                    level: level.into(),
                });
                if item.view.log.len() > 2000 {
                    item.view.log.remove(0);
                }
            }
            let handle = item.view.handle.clone();
            let session = item.view.session.clone();
            self.emit(&handle, session, kind, data);
        }
        if changed {
            let _ = self.persist();
        }
        self.update_tray();
    }
}
fn string<'a>(v: &'a Value, k: &str) -> Res<&'a str> {
    v[k].as_str()
        .ok_or_else(|| Failure::new("invalid_argument", format!("missing {k}")))
}
fn command(s: &mut State, v: &Value) -> Res<Value> {
    match string(v, "op")? {
        "hello" => {
            if v["version"] != 1 {
                return Err(Failure::new("version", "unsupported protocol version"));
            }
            Ok(json!({"version":1}))
        }
        "status" | "snapshot" => Ok(json!({"snapshot":s.snapshot()})),
        "start" => {
            let req: StartRequest = serde_json::from_value(v.clone())
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            Ok(json!({"handle":s.launch(req)?}))
        }
        "resume" => Ok(json!({"handle":s.resume(string(v,"key")?,v["note"].as_str())?})),
        "interject" => {
            let i = s.index(string(v, "key")?)?;
            s.items[i]
                .wish
                .as_ref()
                .ok_or_else(|| Failure::new("finished", "wish is not running"))?
                .interject(string(v, "text")?)?;
            Ok(json!({}))
        }
        "stop" => {
            let i = s.index(string(v, "key")?)?;
            if let Some(w) = &s.items[i].wish {
                w.stop();
            }
            s.items[i].user_stopped = true;
            s.items[i].recovered = false;
            s.persist()?;
            Ok(json!({}))
        }
        "pause_all" => Ok(json!({"stopped":s.pause()})),
        "quit" => {
            let stop = v["stop_running"].as_bool().unwrap_or(false);
            if s.count() > 0 && !stop {
                return Err(Failure::new("running", "wishes are still running"));
            }
            if stop {
                s.pause();
            }
            Ok(json!({}))
        }
        "set_login_start" => {
            let enabled = v["enabled"]
                .as_bool()
                .ok_or_else(|| Failure::new("invalid_argument", "missing enabled"))?;
            if enabled {
                let exe = std::env::current_exe()
                    .map_err(io)?
                    .to_string_lossy()
                    .into_owned();
                s.login.enable(&[exe, "serve".into(), "--login".into()])?;
            } else {
                s.login.disable()?;
            }
            base::atomic_write(
                &s.home.join("service/settings.json"),
                json!({"login_start":enabled}).to_string().as_bytes(),
            )
            .map_err(io)?;
            s.login_start = enabled;
            Ok(json!({}))
        }
        _ => Err(Failure::new("invalid_argument", "unknown operation")),
    }
}
fn connection(mut conn: Conn, state: Arc<Mutex<State>>) {
    let reader_conn = match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };
    let v = match ipc::read_json(&mut BufReader::new(reader_conn), 4 * 1024 * 1024) {
        Ok(Some(v)) => v,
        _ => return,
    };
    if v["op"] == "subscribe" {
        let mut after = v["after"].as_u64().unwrap_or(0);
        {
            let s = state.lock().unwrap();
            if s.events
                .front()
                .is_some_and(|e| after.saturating_add(1) < e.seq)
            {
                let _ = ipc::write_json(&mut conn, &json!({"gap":true}));
                return;
            }
        }
        if ipc::write_json(&mut conn, &json!({"ok":true})).is_err() {
            return;
        }
        loop {
            let (events, done, gap) = {
                let s = state.lock().unwrap();
                (
                    s.events
                        .iter()
                        .filter(|e| e.seq > after)
                        .cloned()
                        .collect::<Vec<_>>(),
                    s.quitting && s.count() == 0,
                    s.events
                        .front()
                        .is_some_and(|e| after.saturating_add(1) < e.seq),
                )
            };
            if gap {
                let _ = ipc::write_json(&mut conn, &json!({"gap":true}));
                return;
            }
            for e in events {
                after = e.seq;
                if ipc::write_json(&mut conn, &serde_json::to_value(e).unwrap()).is_err() {
                    return;
                }
            }
            if done {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    let result = {
        let mut s = state.lock().unwrap();
        s.pump();
        command(&mut s, &v)
    };
    let response = match result {
        Ok(mut value) => {
            value["ok"] = json!(true);
            value
        }
        Err(e) => json!({"ok":false,"error":e.to_json()}),
    };
    let quit = v["op"] == "quit" && response["ok"] == true;
    let _ = ipc::write_json(&mut conn, &response);
    if quit {
        let mut s = state.lock().unwrap();
        s.quitting = true;
        let _ = s.scheduler_stop.send(());
        s.quit_deadline = Some(Instant::now() + Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_preflight_keeps_handle_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join("service")).unwrap();
        let config_home = home.clone();
        let config = Arc::new(move || session::Config {
            claude: Ok(vec!["unused".into()]),
            home: config_home.clone(),
            bin: PathBuf::new(),
            clock: session::Clock::system(),
        });
        let (actions, _) = mpsc::channel();
        let mut s = State {
            home,
            instance: "test".into(),
            config,
            tray: tray::platform_tray(actions),
            desktop: tray::platform_desktop(),
            login: login::platform(),
            login_start: false,
            items: vec![],
            events: VecDeque::new(),
            cursor: 0,
            quitting: false,
            quit_deadline: None,
            scheduler_stop: mpsc::channel().0,
            last_count: 0,
        };
        let req = StartRequest {
            wish: "wish".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            need_bytes: Some(u64::MAX),
            ..Default::default()
        };
        let handle = s.launch(req).unwrap();
        assert_eq!(s.items[0].view.state, "waiting_for_space");
        assert!(s.items[0].view.disk_line.is_some());
        assert_eq!(s.resume(&handle, None).unwrap(), handle);
        assert_eq!(s.items[0].view.state, "waiting_for_space");
    }
}
