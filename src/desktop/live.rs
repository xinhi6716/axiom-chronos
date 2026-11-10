//! Connects the screens to the real engine. One worker thread owns the `Poller` and every hosted
//! `host::Wish`; it takes commands over a channel and sends finished `Run`s back. The UI thread
//! never reads the disk, starts a process or drops a `Wish` (dropping one waits for its process).
use super::model::*;
use crate::{
    app::{self, Poller, Snapshot},
    base::{self, Failure, Res},
    cli::doctor,
    goal,
    host::{self, Event},
    memory::Memory,
    session::{self, Config},
    usage::{self, Ledger, Locations, Warning},
};
use gpui_kit::{App, Entity, SharedString};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context as TaskContext, Poll, Waker},
    time::{Duration, Instant, SystemTime},
};

const ACTIVE_POLL: Duration = Duration::from_secs(1);
const IDLE_POLL: Duration = Duration::from_secs(5);
/// An open goal with no activity for this long, and nothing in flight, has lost its process.
const QUIET_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The same, while a tool call is open or the last turn ended in an error such as a usage limit:
/// those go quiet for hours while the process is alive. A dead run offers Resume late; a live one
/// offered Resume would be run twice.
const LONG_QUIET_LIMIT: Duration = Duration::from_secs(6 * 60 * 60);
/// Reading a transcript can take seconds, so a live run's ledger is refreshed this rarely.
const LEDGER_EVERY: Duration = Duration::from_secs(60);
const MEMORY_EVERY: Duration = Duration::from_secs(60);
const MEMORY_SHOWN: i64 = 500;
const LOG_LINES_KEPT: usize = 2000;
const LOG_LINE_CHARS: usize = 400;
const TITLE_BYTES: u64 = 16 << 10;

const STAGE_NAMES: [(&str, &str); 7] = [
    ("astra", "Astra"),
    ("critic", "Critic"),
    ("architect", "Architect"),
    ("build", "Build"),
    ("full_test", "Full test"),
    ("memory", "Memory"),
    ("report", "Report"),
];

enum Command {
    Start(WishRequest),
    Interject { session: String, text: String },
    Stop(String),
    Resume { session: String, note: Option<String> },
    DeleteMemory(i64),
    CorrectMemory(i64, String),
    Refresh,
    Recheck,
    Follow(String),
    Ledger { session: String, activity: Option<String>, result: Res<Ledger> },
    Memory(Res<Vec<MemoryEntry>>),
    Checks(Vec<Check>),
    Quit,
}

enum Update {
    Runs { runs: Vec<Run>, renamed: Vec<(String, String)>, open: Option<String> },
    Memory(Vec<MemoryEntry>),
    Checks(Vec<Check>),
    Failed { action: Action, session: Option<String>, message: String },
}

#[derive(Default)]
struct Mailbox {
    updates: Vec<Update>,
    closed: bool,
    waker: Option<Waker>,
}

/// Worker to UI. The UI task sleeps until something arrives, so an idle app uses no CPU.
#[derive(Clone, Default)]
struct Inbox(Arc<Mutex<Mailbox>>);

impl Inbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, Mailbox> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, update: Update) {
        let mut mailbox = self.lock();
        mailbox.updates.push(update);
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
    }

    fn close(&self) {
        let mut mailbox = self.lock();
        mailbox.closed = true;
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
    }

    fn next(&self) -> Next {
        Next(self.clone())
    }
}

struct Next(Inbox);

impl Future for Next {
    /// `None` once the worker has gone and everything it sent has been taken.
    type Output = Option<Vec<Update>>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut mailbox = self.0.lock();
        if !mailbox.updates.is_empty() {
            return Poll::Ready(Some(std::mem::take(&mut mailbox.updates)));
        }
        if mailbox.closed {
            return Poll::Ready(None);
        }
        mailbox.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Closes the inbox however the worker ends, so a panic reaches the screen.
struct CloseOnExit(Inbox);

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct LiveBackend {
    commands: mpsc::Sender<Command>,
}

impl LiveBackend {
    fn send(&self, command: Command) -> Res<()> {
        self.commands
            .send(command)
            .map_err(|_| Failure::new("engine_stopped", "the engine worker is no longer running"))
    }
}

impl Backend for LiveBackend {
    fn start_wish(&self, request: WishRequest) -> Res<()> {
        self.send(Command::Start(request))
    }

    fn interject(&self, session: &str, text: &str) -> Res<()> {
        self.send(Command::Interject { session: session.into(), text: text.into() })
    }

    fn stop(&self, session: &str) -> Res<()> {
        self.send(Command::Stop(session.into()))
    }

    fn resume(&self, session: &str, note: Option<&str>) -> Res<()> {
        self.send(Command::Resume { session: session.into(), note: note.map(str::to_owned) })
    }

    fn delete_memory(&self, id: i64) -> Res<()> {
        self.send(Command::DeleteMemory(id))
    }

    fn correct_memory(&self, id: i64, text: &str) -> Res<()> {
        self.send(Command::CorrectMemory(id, text.into()))
    }

    fn refresh(&self) -> Res<()> {
        self.send(Command::Refresh)
    }

    fn recheck(&self) -> Res<()> {
        self.send(Command::Recheck)
    }

    fn follow(&self, session: &str) -> Res<()> {
        self.send(Command::Follow(session.into()))
    }
}

impl Drop for LiveBackend {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
    }
}

/// Starts the worker and keeps `state` up to date from it.
pub fn connect(state: Entity<AppState>, cx: &mut App) {
    let (commands, receiver) = mpsc::channel();
    let inbox = Inbox::default();
    let locations = Locations::from_env();
    let engine = Engine::new(locations.home, locations.projects, Box::new(Config::from_env), commands.clone(), inbox.clone());
    std::thread::Builder::new()
        .name("chronos-engine".into())
        .spawn(move || engine.run(receiver))
        .expect("could not start the engine worker");
    state.update(cx, |s, _| s.backend = Some(Box::new(LiveBackend { commands })));
    cx.spawn(async move |cx| {
        let mut followed: Option<SharedString> = None;
        while let Some(updates) = inbox.next().await {
            let _ = cx.update(|cx| {
                state.update(cx, |s, cx| {
                    apply(s, updates);
                    let viewing = s.run().map(|run| run.session.clone());
                    if viewing != followed {
                        followed = viewing;
                        s.follow_current();
                    }
                    cx.notify();
                })
            });
        }
        let _ = cx.update(|cx| {
            state.update(cx, |s, cx| {
                s.report(Action::Engine, None, "the engine worker stopped");
                cx.notify();
            })
        });
    })
    .detach();
}

fn apply(state: &mut AppState, updates: Vec<Update>) {
    for update in updates {
        match update {
            Update::Runs { runs, renamed, open } => state.replace_runs(runs, &renamed, open.as_deref()),
            Update::Memory(memory) => state.memory = memory,
            Update::Checks(checks) => state.checks = checks,
            Update::Failed { action, session, message } => state.report(action, session.map(Into::into), message),
        }
    }
}

/// A wish started from this app. Kept after it ends: its log exists nowhere else.
struct Hosted {
    /// Only ever dropped on the worker, and only once its thread has finished.
    wish: Option<host::Wish>,
    /// The session id is not known yet; the key is a stand-in until `Event::Started`.
    provisional: bool,
    text: String,
    cwd: PathBuf,
    started: u64,
    log: Vec<LogLine>,
    limit_until: Option<i64>,
    ended_at: Option<String>,
    ending: String,
    /// The end needs no error: the user asked for it, or an error was already reported.
    quiet_end: bool,
}

impl Hosted {
    fn new(provisional: bool, text: String, cwd: PathBuf) -> Self {
        Self {
            wish: None,
            provisional,
            text,
            cwd,
            started: app::unix(SystemTime::now()).unwrap_or(0),
            log: vec![],
            limit_until: None,
            ended_at: None,
            ending: String::new(),
            quiet_end: false,
        }
    }

    fn say(&mut self, source: &str, text: impl AsRef<str>, level: LogLevel) {
        self.log.push(LogLine {
            time: super::control::now_clock(),
            source: source.to_owned().into(),
            text: clipped(text.as_ref().trim(), LOG_LINE_CHARS).into(),
            level,
        });
        if self.log.len() > LOG_LINES_KEPT {
            self.log.remove(0);
        }
    }

    fn view(&self) -> HostedView<'_> {
        HostedView {
            running: self.wish.is_some() && self.ended_at.is_none(),
            limit_until: self.limit_until,
            ended_at: self.ended_at.as_deref(),
            log: &self.log,
        }
    }
}

#[derive(Clone, Copy)]
struct HostedView<'a> {
    running: bool,
    limit_until: Option<i64>,
    ended_at: Option<&'a str>,
    log: &'a [LogLine],
}

struct LedgerEntry {
    ledger: Option<Ledger>,
    activity: Option<String>,
    at: Instant,
}

/// Everything that is owned by the worker thread.
struct Engine {
    home: PathBuf,
    projects: PathBuf,
    config: Box<dyn Fn() -> Config + Send>,
    poller: Poller,
    hosted: HashMap<String, Hosted>,
    /// Provisional ids that have become real, for commands sent just before the switch.
    aliases: HashMap<String, String>,
    snapshot: Option<Snapshot>,
    ledgers: HashMap<String, LedgerEntry>,
    ledger_busy: bool,
    memory_busy: bool,
    memory_read: Option<(Instant, Option<SystemTime>)>,
    memory_dirty: bool,
    memory: Vec<MemoryEntry>,
    followed: Option<String>,
    titles: HashMap<PathBuf, SharedString>,
    reported: HashSet<String>,
    sent: Option<Vec<Run>>,
    renamed: Vec<(String, String)>,
    open: Option<String>,
    provisional_count: u64,
    /// Helper threads answer on the same channel, which also wakes the loop.
    commands: mpsc::Sender<Command>,
    inbox: Inbox,
}

impl Engine {
    fn new(
        home: PathBuf,
        projects: PathBuf,
        config: Box<dyn Fn() -> Config + Send>,
        commands: mpsc::Sender<Command>,
        inbox: Inbox,
    ) -> Self {
        Self {
            poller: Poller::new(&home),
            home,
            projects,
            config,
            hosted: HashMap::new(),
            aliases: HashMap::new(),
            snapshot: None,
            ledgers: HashMap::new(),
            ledger_busy: false,
            memory_busy: false,
            memory_read: None,
            memory_dirty: true,
            memory: vec![],
            followed: None,
            titles: HashMap::new(),
            reported: HashSet::new(),
            sent: None,
            renamed: vec![],
            open: None,
            provisional_count: 0,
            commands,
            inbox,
        }
    }

    fn run(mut self, commands: mpsc::Receiver<Command>) {
        let _close = CloseOnExit(self.inbox.clone());
        self.recheck();
        let mut next_poll = Instant::now();
        loop {
            match commands.recv_timeout(next_poll.saturating_duration_since(Instant::now())) {
                Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(command) => {
                    let mut poll_now = self.handle(command);
                    while let Ok(command) = commands.try_recv() {
                        if matches!(command, Command::Quit) {
                            return;
                        }
                        poll_now |= self.handle(command);
                    }
                    self.drain_events();
                    self.publish();
                    if poll_now {
                        next_poll = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.tick();
                    next_poll = Instant::now() + self.interval();
                }
            }
        }
    }

    fn interval(&self) -> Duration {
        let hosted = self.hosted.values().any(|h| h.view().running);
        let live = |run: &Run| matches!(run.status, RunStatus::Active | RunStatus::LimitWait);
        if hosted || self.sent.as_ref().is_some_and(|runs| runs.iter().any(live)) {
            ACTIVE_POLL
        } else {
            IDLE_POLL
        }
    }

    fn tick(&mut self) {
        self.drain_events();
        self.snapshot = Some(self.poller.poll());
        self.reap();
        self.report_warnings();
        self.schedule_ledger();
        self.schedule_memory();
        self.publish();
    }

    /// Runs one command. Returns whether the disk should be read again right away.
    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Start(request) => {
                self.start(request);
                false
            }
            Command::Interject { session, text } => {
                self.interject(&session, &text);
                false
            }
            Command::Stop(session) => {
                self.stop(&session);
                false
            }
            Command::Resume { session, note } => {
                self.resume(&session, note.as_deref());
                true
            }
            Command::DeleteMemory(id) => {
                self.delete_memory(id);
                false
            }
            Command::CorrectMemory(id, text) => {
                self.correct_memory(id, &text);
                false
            }
            Command::Refresh => true,
            Command::Recheck => {
                self.recheck();
                false
            }
            Command::Follow(session) => {
                self.poller.follow(&session);
                self.followed = Some(session);
                true
            }
            Command::Ledger { session, activity, result } => {
                self.ledger_busy = false;
                let ledger = match result {
                    Ok(ledger) => Some(ledger),
                    Err(e) if e.kind == "not_found" => None,
                    Err(e) => {
                        self.warn_once(format!("usage for {session}: {}", e.message));
                        None
                    }
                };
                self.ledgers.insert(session, LedgerEntry { ledger, activity, at: Instant::now() });
                false
            }
            Command::Memory(result) => {
                self.memory_busy = false;
                match result {
                    Ok(memory) => {
                        if memory != self.memory {
                            self.memory = memory.clone();
                            self.inbox.push(Update::Memory(memory));
                        }
                    }
                    Err(e) => self.warn_once(format!("memory: {}", e.message)),
                }
                false
            }
            Command::Checks(checks) => {
                self.inbox.push(Update::Checks(checks));
                false
            }
            Command::Quit => false,
        }
    }

    fn fail(&self, action: Action, session: Option<&str>, message: impl Into<String>) {
        self.inbox.push(Update::Failed { action, session: session.map(str::to_owned), message: message.into() });
    }

    fn warn_once(&mut self, warning: String) {
        if self.reported.insert(warning.clone()) {
            self.fail(Action::ReadData, None, warning);
        }
    }

    fn resolve(&self, session: &str) -> String {
        self.aliases.get(session).cloned().unwrap_or_else(|| session.to_owned())
    }

    fn start(&mut self, request: WishRequest) {
        self.provisional_count += 1;
        let key = format!("pending-{}", self.provisional_count);
        self.hosted.insert(key.clone(), Hosted::new(true, request.wish.clone(), request.cwd.clone()));
        self.open = Some(key.clone());
        // Starting a process takes a moment; the run should appear before it does.
        self.publish();
        let options = host::Options { creative: request.creative, max_turns: request.max_turns, advanced_research: request.advanced_research };
        match host::Wish::start((self.config)(), &request.cwd, &request.wish, &options) {
            Ok(wish) => {
                if let Some(entry) = self.hosted.get_mut(&key) {
                    entry.wish = Some(wish);
                }
            }
            Err(failure) => {
                self.hosted.remove(&key);
                // The dialog has closed; the details are the only copy of what was typed.
                self.fail(Action::StartWish, None, format!("{}\n\nYour wish: {}", failure.message, request.wish));
            }
        }
    }

    fn interject(&mut self, session: &str, text: &str) {
        let session = self.resolve(session);
        let result = match self.hosted.get(&session).and_then(|h| h.wish.as_ref()) {
            Some(wish) => wish.interject(text),
            None => Err(Failure::new("not_hosted", "this wish has finished, or was not started from this app")),
        };
        match result {
            Ok(()) => {
                if let Some(entry) = self.hosted.get_mut(&session) {
                    entry.say(YOU, text, LogLevel::Info);
                }
            }
            Err(failure) => {
                self.fail(Action::Interject, Some(&session), format!("{}\n\nYour message: {text}", failure.message));
                self.resend_runs();
            }
        }
    }

    fn stop(&mut self, session: &str) {
        let session = self.resolve(session);
        match self.hosted.get_mut(&session) {
            Some(entry) if entry.view().running => {
                entry.quiet_end = true;
                if let Some(wish) = &entry.wish {
                    wish.stop();
                }
                entry.say("chronos", "Stopping; it can be resumed later", LogLevel::Info);
            }
            _ => self.fail(
                Action::Stop,
                Some(&session),
                "this wish is not running from this app; stop it where it was started",
            ),
        }
    }

    fn resume(&mut self, session: &str, note: Option<&str>) {
        let session = self.resolve(session);
        if self.hosted.get(&session).is_some_and(|h| h.view().running) {
            self.fail(Action::Resume, Some(&session), "this wish is already running");
            return;
        }
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        let config = (self.config)();
        match host::Wish::resume(config, &session, note, None) {
            Ok(wish) => {
                let entry = self.held(&session);
                entry.wish = Some(wish);
                entry.ended_at = None;
                entry.limit_until = None;
                entry.quiet_end = false;
                if let Some(note) = note {
                    entry.say(YOU, note, LogLevel::Info);
                }
                entry.say("chronos", "Resumed", LogLevel::Info);
            }
            Err(failure) => {
                // The goal may already be reopened with no process behind it.
                let reopened = goal::load(&self.home, &session)
                    .ok()
                    .flatten()
                    .is_some_and(|g| g.status == "active");
                if reopened {
                    let entry = self.held(&session);
                    entry.ended_at = Some(base::now_iso());
                    entry.quiet_end = true;
                }
                let typed = note.map_or(String::new(), |note| format!("\n\nYour reply: {note}"));
                self.fail(Action::Resume, Some(&session), format!("{}{typed}", failure.message));
                self.resend_runs();
            }
        }
    }

    /// The control bar logs what was sent before the engine answers; a refused message must
    /// not stay in the log, so the next publish replaces the screen's copy.
    fn resend_runs(&mut self) {
        self.sent = None;
    }

    fn held(&mut self, session: &str) -> &mut Hosted {
        self.hosted.entry(session.to_owned()).or_insert_with(|| Hosted::new(false, String::new(), PathBuf::new()))
    }

    fn delete_memory(&mut self, id: i64) {
        match Memory::open(&self.home).and_then(|m| m.delete(id)) {
            Ok(_) => {
                self.memory.retain(|m| m.id != id);
                self.inbox.push(Update::Memory(self.memory.clone()));
                self.memory_dirty = true;
            }
            Err(failure) => self.fail(Action::DeleteMemory, None, failure.message),
        }
    }

    fn correct_memory(&mut self, id: i64, text: &str) {
        match Memory::open(&self.home).and_then(|m| m.correct(id, text)) {
            Ok(_) => {
                if let Some(entry) = self.memory.iter_mut().find(|m| m.id == id) {
                    entry.body = text.trim().to_owned().into();
                }
                self.inbox.push(Update::Memory(self.memory.clone()));
                self.memory_dirty = true;
            }
            Err(failure) => self.fail(Action::CorrectMemory, None, failure.message),
        }
    }

    fn recheck(&self) {
        let commands = self.commands.clone();
        std::thread::spawn(move || {
            let checks = doctor::checks().into_iter().map(map_check).collect();
            let _ = commands.send(Command::Checks(checks));
        });
    }

    fn drain_events(&mut self) {
        let keys: Vec<String> = self.hosted.keys().cloned().collect();
        for key in keys {
            let events: Vec<Event> = match self.hosted.get(&key).and_then(|h| h.wish.as_ref()) {
                Some(wish) => wish.events().try_iter().collect(),
                None => continue,
            };
            let mut key = key;
            for event in events {
                self.on_event(&mut key, event);
            }
        }
    }

    fn on_event(&mut self, key: &mut String, event: Event) {
        if let Event::Started { session } = &event {
            if self.hosted.get(key.as_str()).is_some_and(|h| h.provisional) {
                if let Some(mut entry) = self.hosted.remove(key.as_str()) {
                    entry.provisional = false;
                    self.hosted.insert(session.clone(), entry);
                }
                self.aliases.insert(key.clone(), session.clone());
                self.renamed.push((key.clone(), session.clone()));
                if self.open.as_deref() == Some(key.as_str()) {
                    self.open = Some(session.clone());
                }
                self.poller.follow(session);
                *key = session.clone();
            }
        }
        let Some(entry) = self.hosted.get_mut(key.as_str()) else { return };
        match event {
            Event::Started { .. } => {
                entry.limit_until = None;
                entry.say("chronos", "Session started", LogLevel::Info);
            }
            Event::Text(text) => entry.say("claude", text, LogLevel::Info),
            Event::ToolUse { name, label } => {
                let text = label.unwrap_or_else(|| name.clone());
                entry.say(&name, text, LogLevel::Info);
            }
            Event::ToolResult { name, summary } => entry.say(&name, summary, LogLevel::Info),
            Event::Result { text, is_error: true } => {
                entry.say("claude", format!("Model session reported an error: {text}"), LogLevel::Error)
            }
            // Only the units' evidence can say whether the work was done right.
            Event::Result { is_error: false, .. } => {
                entry.say("claude", "Model session finished; this does not verify the work", LogLevel::Info)
            }
            Event::Waiting { until } => {
                entry.limit_until = Some(until);
                entry.say("chronos", format!("Usage limit reached; continuing at {}", clock_of(until)), LogLevel::Warn);
            }
            Event::Exited { code } => {
                entry.ended_at = Some(base::now_iso());
                entry.limit_until = None;
                entry.ending = format!("the session exited with code {code} before it recorded a goal");
                let (text, level) = match code {
                    0 => ("Session exited".to_owned(), LogLevel::Info),
                    130 => ("Stopped".to_owned(), LogLevel::Info),
                    code => (format!("Session exited with code {code}"), LogLevel::Warn),
                };
                entry.say("chronos", text, level);
            }
            Event::Failed(failure) => {
                entry.ended_at = Some(base::now_iso());
                entry.limit_until = None;
                entry.quiet_end = true;
                entry.say("chronos", &failure.message, LogLevel::Error);
                let session = key.clone();
                self.fail(Action::Session, Some(&session), failure.message);
            }
        }
    }

    /// Lets go of finished wishes, and forgets ones that ended before recording any goal.
    fn reap(&mut self) {
        let Some(snapshot) = &self.snapshot else { return };
        let known: HashSet<&str> = snapshot.wishes.iter().map(|w| w.session.as_str()).collect();
        let mut gone = vec![];
        for (key, entry) in &mut self.hosted {
            if entry.ended_at.is_some() {
                if entry.wish.as_ref().is_some_and(|wish| !wish.is_running()) {
                    entry.wish = None;
                }
                if !known.contains(key.as_str()) && entry.wish.is_none() {
                    gone.push(key.clone());
                }
            }
        }
        for key in gone {
            let Some(entry) = self.hosted.remove(&key) else { continue };
            if !entry.quiet_end {
                self.fail(Action::StartWish, None, entry.ending);
            }
        }
    }

    /// Warnings a run owns are shown on that run; the rest are reported once.
    fn report_warnings(&mut self) {
        let Some(snapshot) = &self.snapshot else { return };
        let stores: Vec<String> = snapshot.wishes.iter().filter_map(store_prefix).collect();
        let loose: Vec<String> = snapshot
            .warnings
            .iter()
            .filter(|warning| !stores.iter().any(|store| warning.starts_with(store.as_str())))
            .cloned()
            .collect();
        for warning in loose {
            self.warn_once(warning);
        }
    }

    /// The ledger reads whole transcripts, so it covers only live runs and the one on screen.
    fn schedule_ledger(&mut self) {
        if self.ledger_busy {
            return;
        }
        let Some(snapshot) = &self.snapshot else { return };
        let due = snapshot
            .wishes
            .iter()
            .filter(|w| w.goal.status == "active" || self.followed.as_deref() == Some(w.session.as_str()))
            .find(|w| match self.ledgers.get(&w.session) {
                None => true,
                Some(entry) => entry.activity != w.last_activity && entry.at.elapsed() >= LEDGER_EVERY,
            });
        let Some(wish) = due else { return };
        let (session, activity) = (wish.session.clone(), wish.last_activity.clone());
        let locations = Locations { home: self.home.clone(), projects: self.projects.clone() };
        let commands = self.commands.clone();
        self.ledger_busy = true;
        std::thread::spawn(move || {
            let result = usage::ledger(&locations, Some(&session));
            let _ = commands.send(Command::Ledger { session, activity, result });
        });
    }

    /// Listing hashes every file an entry names, so it runs off this thread and only on change.
    fn schedule_memory(&mut self) {
        if self.memory_busy {
            return;
        }
        let stamp = std::fs::metadata(self.home.join("memory").join("memory.db"))
            .and_then(|m| m.modified())
            .ok();
        let due = self.memory_dirty
            || match self.memory_read {
                None => true,
                Some((at, seen)) => seen != stamp && at.elapsed() >= MEMORY_EVERY,
            };
        if !due {
            return;
        }
        self.memory_dirty = false;
        self.memory_read = Some((Instant::now(), stamp));
        self.memory_busy = true;
        let (home, commands) = (self.home.clone(), self.commands.clone());
        std::thread::spawn(move || {
            let _ = commands.send(Command::Memory(read_memory(&home)));
        });
    }

    fn compose(&mut self) -> Vec<Run> {
        let now = SystemTime::now();
        let mut runs = Vec::new();
        let mut known = HashSet::new();
        if let Some(snapshot) = &self.snapshot {
            for wish in &snapshot.wishes {
                known.insert(wish.session.clone());
                let sources = Sources {
                    needs: &snapshot.needs_you,
                    warnings: &snapshot.warnings,
                    hosted: self.hosted.get(&wish.session).map(Hosted::view),
                    ledger: self.ledgers.get(&wish.session).and_then(|e| e.ledger.as_ref()),
                    now,
                };
                runs.push(map_wish(wish, &sources, &mut self.titles));
            }
        }
        // Started here but with no goal on disk yet.
        let mut early: Vec<(&String, &Hosted)> =
            self.hosted.iter().filter(|(key, _)| !known.contains(key.as_str())).collect();
        early.sort_by(|a, b| b.1.started.cmp(&a.1.started).then_with(|| a.0.cmp(b.0)));
        let early: Vec<Run> = early.into_iter().map(|(key, entry)| early_run(key, entry)).collect();
        runs.splice(0..0, early);
        runs
    }

    /// Sends the runs if anything about them changed.
    fn publish(&mut self) {
        let runs = self.compose();
        let changed = self.sent.as_ref() != Some(&runs);
        if !changed && self.renamed.is_empty() && self.open.is_none() {
            return;
        }
        self.sent = Some(runs.clone());
        self.inbox.push(Update::Runs { runs, renamed: std::mem::take(&mut self.renamed), open: self.open.take() });
    }
}

fn map_check(check: doctor::Check) -> Check {
    Check {
        name: check.label.into(),
        status: if check.ok {
            CheckStatus::Ok
        } else if check.required {
            CheckStatus::Fail
        } else {
            CheckStatus::Warn
        },
        detail: check.detail.into(),
    }
}

fn read_memory(home: &Path) -> Res<Vec<MemoryEntry>> {
    // Opening creates the database, and looking must not.
    if !home.join("memory").join("memory.db").is_file() {
        return Ok(vec![]);
    }
    let listed = Memory::open(home)?.list(None, MEMORY_SHOWN)?;
    let text = |entry: &Value, key: &str| -> SharedString { entry[key].as_str().unwrap_or("").to_owned().into() };
    Ok(listed["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(MemoryEntry {
                id: entry["id"].as_i64()?,
                project: text(entry, "project"),
                title: text(entry, "title"),
                body: text(entry, "body"),
                // `null` means the project is gone, so nothing is known to have changed.
                stale: entry["stale"].as_bool() == Some(true),
            })
        })
        .collect())
}

/// What `map_wish` needs besides the wish itself.
struct Sources<'a> {
    needs: &'a [app::Attention],
    warnings: &'a [String],
    hosted: Option<HostedView<'a>>,
    ledger: Option<&'a Ledger>,
    now: SystemTime,
}

fn clipped(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

fn stamp_of(unix: i64) -> String {
    session::log_stamp(unix, session::local_offset_secs(unix))
}

/// `HH:MM` local time.
fn clock_of(unix: i64) -> String {
    let stamp = stamp_of(unix);
    match (stamp.get(9..11), stamp.get(11..13)) {
        (Some(hours), Some(minutes)) => format!("{hours}:{minutes}"),
        _ => "—".into(),
    }
}

/// "Nov 3, 05:15" from a `YYYYMMDD-HHMMSS` stamp, or nothing when it cannot be read.
fn started_text(stamp: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = stamp.get(4..6).and_then(|m| m.parse::<usize>().ok()).and_then(|m| MONTHS.get(m.checked_sub(1)?));
    let day = stamp.get(6..8).and_then(|d| d.parse::<u32>().ok());
    match (month, day, stamp.get(9..11), stamp.get(11..13)) {
        (Some(month), Some(day), Some(hours), Some(minutes)) => format!("{month} {day}, {hours}:{minutes}"),
        _ => String::new(),
    }
}

fn elapsed_text(seconds: u64) -> SharedString {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "<1 m".into(),
        (0, m) => format!("{m} m").into(),
        (h, m) => format!("{h} h {m} m").into(),
    }
}

fn store_prefix(wish: &app::Wish) -> Option<String> {
    let name = wish.store.as_ref()?.file_name()?.to_string_lossy().into_owned();
    Some(format!("{name}/"))
}

fn stage_of(stage: &app::Stage) -> Stage {
    let name = STAGE_NAMES.iter().find(|(key, _)| *key == stage.name).map_or(stage.name, |(_, name)| name);
    let mut phase = match stage.status {
        app::Status::NotStarted => Phase::Waiting,
        app::Status::Running => Phase::Running,
        app::Status::Done => Phase::Done,
        app::Status::Failed => Phase::Failed,
        app::Status::Skipped => Phase::Skipped,
    };
    // A pass from before the units last changed says nothing about the code as it is now.
    if stage.name == "full_test" && phase == Phase::Done && stage.detail.as_deref().is_some_and(|d| d.contains("stale:")) {
        phase = Phase::Waiting;
    }
    Stage { name: name.into(), phase, note: stage.detail.clone().map(Into::into) }
}

/// Stored state and the evidence behind it must agree before a unit is called verified.
fn unit_state(unit: &app::UnitView) -> UnitState {
    if unit.claim.is_some() {
        return UnitState::Running;
    }
    match unit.state.as_str() {
        "verified" if unit.derived == "verified" => UnitState::Verified,
        "verified" | "built" => UnitState::Built,
        "failed" | "accept-failed" | "review-failed" => UnitState::Failed,
        _ => UnitState::Pending,
    }
}

/// The GOAL field of a brief: the rest of its line, or the next line when that is empty.
fn goal_line(text: &str) -> Option<String> {
    const OTHER_FIELDS: [&str; 3] = ["SCOPE", "VERIFY", "CONTEXT"];
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("GOAL") else { continue };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim();
        if !rest.is_empty() {
            return Some(rest.to_owned());
        }
        let next = lines.map(str::trim).find(|l| !l.is_empty())?;
        return (!OTHER_FIELDS.iter().any(|field| next.starts_with(field))).then(|| next.to_owned());
    }
    None
}

fn read_start(path: &Path, bytes: u64) -> Option<String> {
    use std::io::Read;
    let mut text = Vec::new();
    std::fs::File::open(path).ok()?.take(bytes).read_to_end(&mut text).ok()?;
    Some(String::from_utf8_lossy(&text).into_owned())
}

/// The title from the brief's GOAL. Briefs do not change once written, so one read is enough.
fn unit_title(unit: &app::UnitView, titles: &mut HashMap<PathBuf, SharedString>) -> SharedString {
    let Some(path) = &unit.brief else { return unit.id.clone().into() };
    if let Some(title) = titles.get(path) {
        return title.clone();
    }
    match read_start(path, TITLE_BYTES).as_deref().and_then(goal_line) {
        Some(goal) => {
            let title: SharedString = clipped(&goal, 200).into();
            titles.insert(path.clone(), title.clone());
            title
        }
        None => unit.id.clone().into(),
    }
}

fn contract_text(module: &app::ModuleView) -> Option<TextSource> {
    let mut parts = vec![];
    if !module.contract.trim().is_empty() {
        parts.push(module.contract.trim().to_owned());
    }
    if !module.paths.is_empty() {
        parts.push(format!("PATHS\n{}", module.paths.join("\n")));
    }
    if !module.accept_command.trim().is_empty() {
        parts.push(format!("ACCEPT\n{}", module.accept_command.trim()));
    }
    (!parts.is_empty()).then(|| TextSource::Inline(parts.join("\n\n").into()))
}

fn review_text(wish: &app::Wish, unit: &app::UnitView) -> Option<TextSource> {
    let verdict = wish.store.as_ref().map(|store| store.join("reports").join(format!("{}.verdict.json", unit.id)));
    if let Some(path) = verdict.filter(|path| path.is_file()) {
        return Some(TextSource::Path(path));
    }
    let review = unit.review.as_ref()?;
    Some(TextSource::Inline(format!("{}\nexit {}\n{}", review.command, review.exit_code, review.ts).into()))
}

/// Modules and units, joined by index. A unit outside the module map gets a module named after
/// its track, because every screen expects a unit to belong to one.
fn structure(
    wish: &app::Wish,
    titles: &mut HashMap<PathBuf, SharedString>,
    warnings: &mut Vec<SharedString>,
) -> (Vec<Module>, Vec<Unit>) {
    let position = |id: &str| wish.modules.iter().position(|m| m.id == id);
    let mut modules: Vec<Module> = wish
        .modules
        .iter()
        .map(|module| Module {
            id: module.id.clone().into(),
            purpose: module.purpose.clone().into(),
            depends: module
                .depends
                .iter()
                .filter_map(|id| {
                    let found = position(id);
                    if found.is_none() {
                        warnings.push(format!("Module {} depends on {id}, which is not in the map", module.id).into());
                    }
                    found
                })
                .collect(),
            units: vec![],
            contract: contract_text(module),
        })
        .collect();
    let mut units = Vec::with_capacity(wish.units.len());
    for unit in &wish.units {
        let found = modules.iter().position(|m| m.id.as_ref() == unit.track.as_str());
        let module = found.unwrap_or_else(|| {
            modules.push(Module {
                id: unit.track.clone().into(),
                purpose: unit.track.clone().into(),
                depends: vec![],
                units: vec![],
                contract: None,
            });
            modules.len() - 1
        });
        modules[module].units.push(units.len());
        units.push(Unit {
            id: unit.id.clone().into(),
            title: unit_title(unit, titles),
            module,
            state: unit_state(unit),
            raw_state: unit.state.clone().into(),
            attempts: unit.attempts as u32,
            files: UnitFiles {
                brief: unit.brief.clone().map(TextSource::Path),
                report: unit.report.clone().map(TextSource::Path),
                verify: unit.verify_output.clone().map(TextSource::Path),
                review: review_text(wish, unit),
            },
        });
    }
    (modules, units)
}

fn need_of(attention: &app::Attention) -> Need {
    let kind = match attention.kind.as_str() {
        "interjection" => NeedKind::Interjection,
        "waiting" => NeedKind::Waiting,
        _ => NeedKind::Blocked,
    };
    let text = attention.note.trim();
    let (title, note) = match text.split_once('\n') {
        Some((first, rest)) => (first.trim(), rest.trim()),
        None if text.is_empty() => ("No note was left.", ""),
        None => (text, ""),
    };
    Need { kind, title: clipped(title, 120).into(), note: note.to_owned().into() }
}

/// The status of a wish, and whether this app holds it. Only hosted wishes have a process to look
/// at; for the rest, an open goal is judged by how long it has been quiet.
fn status_of(wish: &app::Wish, hosted: Option<HostedView<'_>>, now: SystemTime) -> (RunStatus, bool) {
    match wish.goal.status.as_str() {
        "done" => return (RunStatus::Done, hosted.is_some()),
        "waiting" => return (RunStatus::Waiting, hosted.is_some()),
        "active" => {}
        // "blocked", and anything a newer engine writes: it must not look finished or running.
        _ => return (RunStatus::Blocked, hosted.is_some()),
    }
    if let Some(view) = hosted {
        if view.running {
            let status = if view.limit_until.is_some() { RunStatus::LimitWait } else { RunStatus::Active };
            return (status, true);
        }
        let touched_since = match (&wish.last_activity, view.ended_at) {
            (Some(activity), Some(ended)) => activity.as_str() > ended,
            _ => false,
        };
        if !touched_since {
            return (RunStatus::Interrupted, true);
        }
    }
    (quiet_status(wish, now), false)
}

fn quiet_status(wish: &app::Wish, now: SystemTime) -> RunStatus {
    let last = wish.runs.last();
    let may_be_waiting = last.is_some_and(|run| {
        run.scan.as_ref().is_some_and(|scan| !scan.open.is_empty())
            || run.summary.as_ref().is_some_and(|summary| summary.is_error)
    });
    let limit = if may_be_waiting { LONG_QUIET_LIMIT } else { QUIET_LIMIT };
    let cutoff = app::iso(now.checked_sub(limit).unwrap_or(SystemTime::UNIX_EPOCH));
    match &wish.last_activity {
        Some(activity) if activity.as_str() < cutoff.as_str() => RunStatus::Interrupted,
        _ => RunStatus::Active,
    }
}

fn add_known(total: Option<f64>, value: Option<f64>) -> Option<f64> {
    Some(total? + value?)
}

fn sum_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    Some(left? + right?)
}

fn claude_models(wish: &app::Wish, ledger: Option<&Ledger>) -> Vec<ModelCost> {
    // Whether a resumed session's figures repeat the earlier ones is not documented, so figures
    // are read from a summary only when the session has exactly one log.
    let single = match wish.runs.as_slice() {
        [only] => only.summary.as_ref().filter(|s| !s.models.is_empty()),
        _ => None,
    };
    if let Some(summary) = single {
        return summary
            .models
            .iter()
            .map(|(name, model)| ModelCost {
                model: name.clone().into(),
                usd: model.cost_usd,
                input: model.input,
                cached: sum_u64(model.cache_read, model.cache_creation),
                output: model.output,
            })
            .collect();
    }
    if let Some(ledger) = ledger {
        let mut rows: Vec<ModelCost> = vec![];
        for row in &ledger.by_agent {
            let tally = &row.tally;
            let cached = tally.cache_read + tally.cache_write;
            match rows.iter_mut().find(|r| r.model.as_ref() == row.model.as_str()) {
                Some(found) => {
                    found.usd = add_known(found.usd, tally.usd);
                    found.input = sum_u64(found.input, Some(tally.input));
                    found.cached = sum_u64(found.cached, Some(cached));
                    found.output = sum_u64(found.output, Some(tally.output));
                }
                None => rows.push(ModelCost {
                    model: row.model.clone().into(),
                    usd: tally.usd,
                    input: Some(tally.input),
                    cached: Some(cached),
                    output: Some(tally.output),
                }),
            }
        }
        return rows;
    }
    let mut names: Vec<&String> = wish.runs.iter().filter_map(|r| r.summary.as_ref()).flat_map(|s| s.models.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, output: None })
        .collect()
}

/// Codex rows. Codex's input count includes its cached tokens; the screens add the columns
/// together, so the cached part is taken out of input here.
fn codex_models(wish: &app::Wish, ledger: Option<&Ledger>) -> Vec<ModelCost> {
    let row = |model: &str, input: u64, cached: u64, output: u64| ModelCost {
        model: model.to_owned().into(),
        usd: None,
        input: Some(input.saturating_sub(cached)),
        cached: Some(cached),
        output: Some(output),
    };
    let scans: Vec<&app::Scan> = wish.runs.iter().filter_map(|r| r.scan.as_ref()).collect();
    if !scans.is_empty() && scans.len() == wish.runs.len() && scans.iter().all(|s| s.complete) {
        let mut totals = std::collections::BTreeMap::<&str, app::Tokens>::new();
        for (name, tokens) in scans.iter().flat_map(|s| s.codex.iter()) {
            let total = totals.entry(name.as_str()).or_default();
            total.input += tokens.input;
            total.cached += tokens.cached;
            total.output += tokens.output;
        }
        return totals.iter().map(|(name, t)| row(name, t.input, t.cached, t.output)).collect();
    }
    if let Some(ledger) = ledger {
        return ledger.codex.iter().map(|c| row(&c.model, c.tokens.input, c.tokens.cached_input, c.tokens.output)).collect();
    }
    let mut names: Vec<&String> = scans.iter().flat_map(|s| s.codex.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, output: None })
        .collect()
}

fn roles_of(ledger: &Ledger) -> Vec<RoleCost> {
    let claude = ledger.by_agent.iter().map(|row| RoleCost {
        role: row.agent_type.clone().into(),
        usd: row.tally.usd,
        input: row.tally.input,
        cached: row.tally.cache_read + row.tally.cache_write,
        output: row.tally.output,
    });
    let codex = ledger.codex.iter().map(|row| RoleCost {
        role: format!("codex {}", row.model).into(),
        usd: None,
        input: row.tokens.input.saturating_sub(row.tokens.cached_input),
        cached: row.tokens.cached_input,
        output: row.tokens.output,
    });
    claude.chain(codex).collect()
}

/// An agent whose peak was never reported has no row; zero would be a made-up figure.
fn contexts_of(wish: &app::Wish, ledger: Option<&Ledger>) -> Vec<AgentContext> {
    let from_scans: Vec<AgentContext> = wish
        .runs
        .iter()
        .filter_map(|r| r.scan.as_ref())
        .flat_map(|s| s.agents.iter())
        .filter_map(|agent| {
            Some(AgentContext {
                agent: agent.kind.clone().unwrap_or_else(|| agent.id.clone()).into(),
                peak_tokens: agent.peak_context?,
            })
        })
        .collect();
    if !from_scans.is_empty() {
        return from_scans;
    }
    ledger
        .into_iter()
        .flat_map(|l| l.instances.iter())
        .filter(|i| i.tally.peak_context > 0)
        .map(|i| AgentContext { agent: i.agent_type.clone().into(), peak_tokens: i.tally.peak_context })
        .collect()
}

fn usage_of(wish: &app::Wish, ledger: Option<&Ledger>) -> Usage {
    let reported = match wish.runs.as_slice() {
        [only] => only.summary.as_ref().and_then(|s| s.total_cost_usd),
        _ => None,
    };
    let mut models = claude_models(wish, ledger);
    models.extend(codex_models(wish, ledger));
    Usage {
        total_usd: reported.or_else(|| ledger.and_then(|l| l.totals.usd)),
        models,
        roles: ledger.map(roles_of).unwrap_or_default(),
        contexts: contexts_of(wish, ledger),
    }
}

fn warning_text(warning: &Warning) -> String {
    match warning {
        Warning::ContextHigh { agent_type, model, peak_context, .. } => {
            format!("{agent_type} reached {peak_context} tokens of context on {model}.")
        }
        Warning::CacheRewritesAfterGap { agent_type, count, tokens, usd, .. } => {
            format!("{agent_type} rewrote its cache {count} times after long waits ({tokens} tokens, about ${usd:.2}).")
        }
        Warning::LargeFileRead { agent_type, path, reads, largest_bytes, .. } => {
            format!("{agent_type} read {path} {reads} times, up to {largest_bytes} bytes.")
        }
        Warning::CopiedHistoryNotSeparable { file, zeroed_calls } => {
            format!("{zeroed_calls} calls in {file} could not be told apart from copied history and were not counted.")
        }
    }
}

fn map_wish(wish: &app::Wish, sources: &Sources<'_>, titles: &mut HashMap<PathBuf, SharedString>) -> Run {
    let (status, hosted) = status_of(wish, sources.hosted, sources.now);
    let mut warnings: Vec<SharedString> = vec![];
    if let Some(prefix) = store_prefix(wish) {
        warnings.extend(sources.warnings.iter().filter(|w| w.starts_with(&prefix)).map(|w| w.clone().into()));
    }
    if let Some(ledger) = sources.ledger {
        warnings.extend(ledger.warnings.iter().map(|w| warning_text(w).into()));
    }
    let (modules, units) = structure(wish, titles, &mut warnings);
    let mut stages: Vec<Stage> = wish.stages.iter().map(stage_of).collect();
    if let Some(until) = sources.hosted.and_then(|h| h.limit_until).filter(|_| status == RunStatus::LimitWait) {
        if let Some(stage) = stages.iter_mut().find(|s| s.phase == Phase::Running) {
            stage.note = Some(format!("Continues at {}", clock_of(until)).into());
        }
    }
    let mut needs: Vec<Need> =
        sources.needs.iter().filter(|a| a.session == wish.session).map(need_of).collect();
    if status == RunStatus::Interrupted {
        needs.push(Need {
            kind: NeedKind::Blocked,
            title: "The session stopped without closing its goal".into(),
            note: "Its process is gone but the wish is still open. Resume to continue.".into(),
        });
    }
    let started_unix = wish.runs.iter().filter_map(|r| r.started_unix).min();
    let first_stamp = wish.runs.iter().filter_map(|r| r.started.as_deref()).min();
    let end = if matches!(status, RunStatus::Active | RunStatus::LimitWait) {
        app::unix(sources.now)
    } else {
        wish.runs.iter().filter_map(|r| r.modified_unix).max()
    };
    let elapsed = started_unix
        .zip(end)
        .map(|(start, end)| elapsed_text(end.saturating_sub(start.max(0) as u64)));
    let cwd = wish.goal.cwd.as_deref().map(PathBuf::from);
    let project = wish
        .goal
        .project
        .clone()
        .or_else(|| cwd.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let report = wish
        .runs
        .last()
        .and_then(|r| r.summary.as_ref())
        .filter(|s| !s.is_error)
        .and_then(|s| s.result.as_deref())
        .filter(|text| !text.trim().is_empty())
        .map(|text| TextSource::Inline(text.to_owned().into()));
    Run {
        session: wish.session.clone().into(),
        project: project.into(),
        cwd,
        wish: app::unmarked(&wish.goal.wish).to_owned().into(),
        status,
        hosted,
        started: first_stamp.map(started_text).unwrap_or_default().into(),
        started_at: started_unix.and_then(|s| u64::try_from(s).ok()),
        stages,
        // Nothing in the engine maps plan items to modules yet.
        goals: vec![],
        modules,
        units,
        usage: usage_of(wish, sources.ledger),
        needs,
        log: sources.hosted.map(|h| h.log.to_vec()).unwrap_or_default(),
        decisions: wish.decisions.iter().map(|d| d.clone().into()).collect(),
        report,
        elapsed,
        warnings,
    }
}

/// A run with no goal on disk yet: only what the app itself knows, and every figure unknown.
fn early_run(key: &str, entry: &Hosted) -> Run {
    Run {
        session: key.to_owned().into(),
        project: entry.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into(),
        cwd: Some(entry.cwd.clone()).filter(|cwd| !cwd.as_os_str().is_empty()),
        wish: entry.text.clone().into(),
        status: if entry.ended_at.is_some() { RunStatus::Interrupted } else { RunStatus::Active },
        hosted: true,
        started: started_text(&stamp_of(entry.started as i64)).into(),
        started_at: Some(entry.started),
        stages: STAGE_NAMES
            .iter()
            .map(|(_, name)| Stage { name: (*name).into(), phase: Phase::Waiting, note: None })
            .collect(),
        goals: vec![],
        modules: vec![],
        units: vec![],
        usage: Usage::default(),
        needs: vec![],
        log: entry.log.clone(),
        decisions: vec![],
        report: None,
        elapsed: None,
        warnings: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, prelude::v1::test};

    const EVIDENCE: &str = "kind\tcommand\texit_code\tcommit\tts\n";
    const UNITS: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
    const MAP: &str = "# Map\n\n## m1\nPURPOSE    core things\nPATHS      src/a/\nDEPENDS    none\nCONTRACT   a stays stable\nACCEPT     tests/a.py `pytest tests/a.py`\n\n## m2\nPURPOSE    second\nPATHS      src/b/\nDEPENDS    m1\nCONTRACT   b\nACCEPT     tests/b.py `pytest tests/b.py`\n\nFULL TEST  `pytest`\n";

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn goal_file(home: &Path, session: &str, value: Value) {
        write(&home.join("goals").join(format!("{session}.json")), &value.to_string());
    }

    /// A project whose map has m1 and m2, and one unit outside the map.
    fn store(home: &Path) -> PathBuf {
        let store = home.join("orchestrate").join("proj");
        write(&store.join("modules.md"), MAP);
        write(&store.join("briefs/a1.md"), "GOAL    Render every page\n\nSCOPE   src/a/\n\nVERIFY  pytest\n");
        write(&store.join("briefs/b1.md"), "GOAL\nSplit the table\n\nSCOPE\nsrc/b/\n\nVERIFY\npytest\n");
        write(
            &store.join("units.tsv"),
            &format!("{UNITS}b1\tm2\tpending\t\t\t\t\na1\tm1\tverified\t\t\t\t\nz1\tloose\tbuilt\t\t\t\t\n"),
        );
        write(
            &store.join("reports/a1.evidence.tsv"),
            &format!("{EVIDENCE}run\tcodex gpt-6\t0\tc1\t2026-11-09T10:00:00.000Z\nverify\tpytest\t0\tc1\t2026-11-09T10:01:00.000Z\naccept\tpytest\t0\tc1\t2026-11-09T10:02:00.000Z\nreview\tclaude-opus-5-5\t0\tc1\t2026-11-09T10:03:00.000Z\n"),
        );
        write(&store.join("decisions.md"), "Chose the smaller cache\n");
        store
    }

    fn base_goal(status: &str) -> Value {
        json!({"wish":"[no-architect] render pages","status":status,"project":"proj","cwd":"E:\\codex\\proj"})
    }

    fn map_one(home: &Path, hosted: Option<HostedView<'_>>, now: SystemTime) -> Run {
        let snapshot = Poller::new(home).poll();
        let wish = &snapshot.wishes[0];
        let sources = Sources { needs: &snapshot.needs_you, warnings: &snapshot.warnings, hosted, ledger: None, now };
        map_wish(wish, &sources, &mut HashMap::new())
    }

    #[test]
    fn a_snapshot_maps_to_modules_units_stages_and_needs() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path());
        goal_file(dir.path(), "s1", json!({"wish":"render","status":"blocked","note":"Need a key\nProvide STAGING_DSN","project":"proj","cwd":"E:\\codex\\proj"}));
        let run = map_one(dir.path(), None, SystemTime::now());

        assert_eq!(run.session.as_ref(), "s1");
        assert_eq!(run.project.as_ref(), "proj");
        assert_eq!(run.status, RunStatus::Blocked);
        assert!(!run.hosted);
        assert_eq!(run.needs.len(), 1);
        assert_eq!(run.needs[0].kind, NeedKind::Blocked);
        assert_eq!(run.needs[0].title.as_ref(), "Need a key");
        assert_eq!(run.needs[0].note.as_ref(), "Provide STAGING_DSN");
        assert_eq!(run.decisions.len(), 1);
        assert!(run.goals.is_empty(), "goals have no engine source yet");

        let names: Vec<&str> = run.stages.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names, ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report"]);

        let ids: Vec<&str> = run.modules.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids, ["m1", "m2", "loose"], "a unit outside the map still has a module");
        assert_eq!(run.modules[1].depends, vec![0]);
        assert_eq!(run.modules[2].purpose.as_ref(), "loose");
        match run.modules[0].contract.as_ref().unwrap() {
            TextSource::Inline(text) => {
                assert!(text.contains("a stays stable"));
                assert!(text.contains("pytest tests/a.py"));
            }
            other => panic!("{other:?}"),
        }

        let unit = |id: &str| run.units.iter().find(|u| u.id.as_ref() == id).unwrap();
        assert_eq!(unit("a1").title.as_ref(), "Render every page");
        assert_eq!(unit("b1").title.as_ref(), "Split the table");
        assert_eq!(unit("z1").title.as_ref(), "z1", "no brief means the id");
        assert_eq!(unit("a1").module, 0);
        assert_eq!(unit("b1").module, 1);
        assert_eq!(unit("z1").module, 2);
        for (m, module) in run.modules.iter().enumerate() {
            for &u in &module.units {
                assert_eq!(run.units[u].module, m, "module and unit indices agree");
            }
        }
        assert_eq!(unit("a1").state, UnitState::Verified);
        assert_eq!(unit("b1").state, UnitState::Pending);
        assert_eq!(unit("z1").state, UnitState::Built);
        assert_eq!(unit("a1").attempts, 1);
    }

    #[test]
    fn a_unit_stored_as_verified_without_evidence_is_not_called_verified() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write(&store.join("units.tsv"), &format!("{UNITS}b1\tm2\tverified\t\t\t\t\n"));
        goal_file(dir.path(), "s1", base_goal("done"));
        let run = map_one(dir.path(), None, SystemTime::now());
        assert_eq!(run.units[0].state, UnitState::Built);
        assert_eq!(run.units[0].raw_state.as_ref(), "verified");
    }

    #[test]
    fn a_finished_session_is_not_a_verified_change() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path());
        goal_file(dir.path(), "s1", base_goal("done"));
        let run = map_one(dir.path(), None, SystemTime::now());
        assert_eq!(run.status, RunStatus::Done);
        assert!(!run.verified_end());
        assert_eq!(run.status_label(), "Ended, needs attention");
    }

    #[test]
    fn a_full_test_older_than_the_units_does_not_count_as_passed() {
        let stage = app::Stage {
            name: "full_test",
            status: app::Status::Done,
            detail: Some("exit 0 pytest; stale: units changed since".into()),
        };
        assert_eq!(stage_of(&stage).phase, Phase::Waiting);
        let fresh = app::Stage { detail: Some("exit 0 pytest".into()), ..stage };
        assert_eq!(stage_of(&fresh).phase, Phase::Done);
    }

    #[test]
    fn a_skipped_stage_stays_skipped() {
        let stage = app::Stage { name: "architect", status: app::Status::Skipped, detail: None };
        assert_eq!(stage_of(&stage).phase, Phase::Skipped);
    }

    fn log_with(dir: &Path, session: &str, name: &str, events: &[Value]) {
        let text: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&dir.join("logs").join(name), &text);
        let _ = session;
    }

    #[test]
    fn figures_nobody_reported_stay_unknown() {
        let dir = tempfile::tempdir().unwrap();
        goal_file(dir.path(), "s1", base_goal("done"));
        log_with(
            dir.path(),
            "s1",
            "20261109-100000-wish.jsonl",
            &[
                json!({"type":"system","subtype":"init","session_id":"s1","cwd":"E:\\codex\\proj"}),
                json!({"type":"result","is_error":false,"result":"all done","modelUsage":{"claude-opus-5-5":{"costUSD":1.5,"inputTokens":98},"claude-haiku-5-5":{}}}),
            ],
        );
        let run = map_one(dir.path(), None, SystemTime::now());
        assert_eq!(run.usage.total_usd, None, "the result carried no total");
        let model = |name: &str| run.usage.models.iter().find(|m| m.model.as_ref() == name).unwrap();
        let opus = model("claude-opus-5-5");
        assert_eq!((opus.usd, opus.input, opus.cached, opus.output), (Some(1.5), Some(98), None, None));
        let haiku = model("claude-haiku-5-5");
        assert_eq!((haiku.usd, haiku.input), (None, None));
        assert!(run.usage.roles.is_empty(), "no ledger, so no roles");
        assert!(run.usage.contexts.is_empty());
        assert_eq!(run.started.as_ref(), "Nov 9, 10:00");
        assert_eq!(run.report, Some(TextSource::Inline("all done".into())));
    }

    #[test]
    fn a_run_without_a_result_has_no_cost_or_elapsed_guess() {
        let dir = tempfile::tempdir().unwrap();
        goal_file(dir.path(), "s1", base_goal("blocked"));
        let run = map_one(dir.path(), None, SystemTime::now());
        assert_eq!(run.usage, Usage::default());
        assert_eq!(run.elapsed, None);
        assert_eq!(run.started.as_ref(), "");
    }

    fn view<'a>(running: bool, limit: Option<i64>, ended: Option<&'a str>) -> HostedView<'a> {
        HostedView { running, limit_until: limit, ended_at: ended, log: &[] }
    }

    fn wish_active(dir: &Path) -> app::Wish {
        goal_file(dir, "s1", base_goal("active"));
        Poller::new(dir).poll().wishes.remove(0)
    }

    #[test]
    fn an_open_goal_is_active_only_while_something_can_be_working() {
        let dir = tempfile::tempdir().unwrap();
        let wish = wish_active(dir.path());
        let now = SystemTime::now();
        assert_eq!(status_of(&wish, None, now), (RunStatus::Active, false));
        let later = now + QUIET_LIMIT + Duration::from_secs(60);
        assert_eq!(status_of(&wish, None, later), (RunStatus::Interrupted, false));
        assert_eq!(status_of(&wish, Some(view(true, None, None)), later), (RunStatus::Active, true));
        assert_eq!(status_of(&wish, Some(view(true, Some(99), None)), later), (RunStatus::LimitWait, true));
        let ended = app::iso(now + Duration::from_secs(5));
        assert_eq!(status_of(&wish, Some(view(false, None, Some(&ended))), now), (RunStatus::Interrupted, true));
        let before = app::iso(now - Duration::from_secs(3600));
        assert_eq!(
            status_of(&wish, Some(view(false, None, Some(&before))), now),
            (RunStatus::Active, false),
            "activity after our process ended means someone else picked it up"
        );
    }

    #[test]
    fn closed_goals_keep_their_status_even_while_a_process_is_ending() {
        let dir = tempfile::tempdir().unwrap();
        goal_file(dir.path(), "s1", base_goal("done"));
        let wish = Poller::new(dir.path()).poll().wishes.remove(0);
        assert_eq!(status_of(&wish, Some(view(true, None, None)), SystemTime::now()), (RunStatus::Done, true));
    }

    fn sample(session: &'static str, units: &[&'static str]) -> Run {
        let mut run = super::super::demo::sample_run();
        run.session = session.into();
        run.modules = vec![Module { id: "m".into(), purpose: "p".into(), depends: vec![], units: (0..units.len()).collect(), contract: None }];
        let template = run.units[0].clone();
        run.units = units.iter().map(|id| Unit { id: (*id).into(), ..template.clone() }).collect();
        run
    }

    fn state_with(runs: Vec<Run>) -> AppState {
        let mut state = super::super::demo::state();
        state.runs = runs;
        state.current = None;
        state.selection = Selection::None;
        state
    }

    #[test]
    fn selection_follows_the_session_and_unit_when_the_order_changes() {
        let mut state = state_with(vec![sample("a", &["u1", "u2"]), sample("b", &["x", "y", "z"])]);
        state.current = Some(1);
        state.selection = Selection::Unit(2);
        state.replace_runs(vec![sample("c", &[]), sample("b", &["new", "x", "y", "z"]), sample("a", &[])], &[], None);
        assert_eq!(state.current, Some(1));
        assert_eq!(state.run().unwrap().session.as_ref(), "b");
        assert_eq!(state.selection, Selection::Unit(3), "z moved from 2 to 3");

        state.replace_runs(vec![sample("b", &["z", "x"]), sample("a", &[])], &[], None);
        assert_eq!(state.current, Some(0));
        assert_eq!(state.selection, Selection::Unit(0));
    }

    #[test]
    fn a_selected_module_follows_its_id_and_a_vanished_unit_clears() {
        let mut state = state_with(vec![sample("a", &["u1"])]);
        state.runs[0].modules.push(Module { id: "second".into(), purpose: "p".into(), depends: vec![], units: vec![], contract: None });
        state.current = Some(0);
        state.selection = Selection::Module(1);
        let mut next = sample("a", &["u1"]);
        next.modules.insert(0, Module { id: "second".into(), purpose: "p".into(), depends: vec![], units: vec![], contract: None });
        state.replace_runs(vec![next], &[], None);
        assert_eq!(state.selection, Selection::Module(0));

        state.selection = Selection::Unit(0);
        state.replace_runs(vec![sample("a", &["other"])], &[], None);
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn a_provisional_id_becoming_real_keeps_the_run_and_selection_open() {
        let mut state = state_with(vec![sample("pending-1", &["u1"])]);
        state.current = Some(0);
        state.selection = Selection::Unit(0);
        state.replace_runs(vec![sample("old", &[]), sample("real", &["u1"])], &[("pending-1".into(), "real".into())], None);
        assert_eq!(state.current, Some(1));
        assert_eq!(state.selection, Selection::Unit(0));
        assert_eq!(state.renames.get("real").map(|s| s.as_ref()), Some("pending-1"));
    }

    #[test]
    fn opening_a_new_run_moves_there_and_clears_the_selection() {
        let mut state = state_with(vec![sample("a", &["u1"])]);
        state.current = Some(0);
        state.selection = Selection::Unit(0);
        state.replace_runs(vec![sample("a", &["u1"]), sample("new", &["u1"])], &[], Some("new"));
        assert_eq!(state.current, Some(1));
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn with_nothing_open_a_live_hosted_run_is_chosen_and_a_vanished_one_falls_back() {
        let mut state = state_with(vec![]);
        let mut hosted = sample("here", &[]);
        hosted.status = RunStatus::Active;
        hosted.hosted = true;
        let mut other = sample("terminal", &[]);
        other.status = RunStatus::Done;
        other.hosted = false;
        state.replace_runs(vec![other.clone(), hosted], &[], None);
        assert_eq!(state.run().unwrap().session.as_ref(), "here");
        state.replace_runs(vec![other], &[], None);
        assert_eq!(state.run().unwrap().session.as_ref(), "terminal");
        state.replace_runs(vec![], &[], None);
        assert_eq!(state.current, None);
    }

    struct Rig {
        _dir: tempfile::TempDir,
        engine: Engine,
        inbox: Inbox,
        _receiver: mpsc::Receiver<Command>,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (home, projects) = (dir.path().to_path_buf(), dir.path().join("projects"));
        let config_home = home.clone();
        let config = Box::new(move || Config {
            claude: Err(Failure::new("not_installed", "no claude in tests")),
            home: config_home.clone(),
            bin: PathBuf::new(),
            clock: session::Clock::system(),
        });
        let (commands, receiver) = mpsc::channel();
        let inbox = Inbox::default();
        let engine = Engine::new(home, projects, config, commands, inbox.clone());
        Rig { _dir: dir, engine, inbox, _receiver: receiver }
    }

    impl Rig {
        fn updates(&self) -> Vec<Update> {
            std::mem::take(&mut self.inbox.lock().updates)
        }

        fn failures(&self) -> Vec<(Action, Option<String>, String)> {
            self.updates()
                .into_iter()
                .filter_map(|u| match u {
                    Update::Failed { action, session, message } => Some((action, session, message)),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn a_wish_that_cannot_start_shows_the_error_and_leaves_no_run_behind() {
        let mut rig = rig();
        rig.engine.handle(Command::Start(WishRequest {
            cwd: rig._dir.path().join("missing"),
            wish: "x".into(),
            creative: false,
            max_turns: None,
            advanced_research: false,
        }));
        let updates = rig.updates();
        let provisional = updates.iter().find_map(|u| match u {
            Update::Runs { runs, open, .. } => Some((runs.clone(), open.clone())),
            _ => None,
        });
        let (runs, open) = provisional.expect("the run appears before the process is started");
        assert_eq!(runs[0].session.as_ref(), "pending-1");
        assert_eq!(runs[0].status, RunStatus::Active);
        assert_eq!(runs[0].usage.total_usd, None);
        assert_eq!(open.as_deref(), Some("pending-1"));
        assert!(updates.iter().any(|u| matches!(u, Update::Failed { action: Action::StartWish, .. })));
        assert!(rig.engine.compose().is_empty());
    }

    #[test]
    fn the_provisional_run_takes_the_real_id_on_started() {
        let mut rig = rig();
        rig.engine.hosted.insert("pending-1".into(), Hosted::new(true, "wish".into(), PathBuf::from("E:\\p\\proj")));
        rig.engine.open = Some("pending-1".into());
        let mut key = "pending-1".to_owned();
        rig.engine.on_event(&mut key, Event::Started { session: "real".into() });
        assert_eq!(key, "real");
        rig.engine.publish();
        let (runs, renamed, open) = rig
            .updates()
            .into_iter()
            .find_map(|u| match u {
                Update::Runs { runs, renamed, open } => Some((runs, renamed, open)),
                _ => None,
            })
            .unwrap();
        assert_eq!(runs[0].session.as_ref(), "real");
        assert_eq!(runs[0].project.as_ref(), "proj");
        assert!(runs[0].hosted);
        assert_eq!(renamed, vec![("pending-1".to_owned(), "real".to_owned())]);
        assert_eq!(open.as_deref(), Some("real"));
        assert_eq!(rig.engine.resolve("pending-1"), "real");
    }

    #[test]
    fn a_model_session_finishing_logs_no_success_and_changes_no_status() {
        let mut rig = rig();
        rig.engine.hosted.insert("s".into(), Hosted::new(false, "w".into(), PathBuf::new()));
        let mut key = "s".to_owned();
        rig.engine.on_event(&mut key, Event::Result { text: "all good".into(), is_error: false });
        let entry = &rig.engine.hosted["s"];
        assert_eq!(entry.log[0].level, LogLevel::Info);
        assert!(!entry.log[0].text.contains("all good"));
        assert!(entry.ended_at.is_none());
    }

    #[test]
    fn a_limit_wait_and_a_failure_are_recorded() {
        let mut rig = rig();
        rig.engine.hosted.insert("s".into(), Hosted::new(false, "w".into(), PathBuf::new()));
        let mut key = "s".to_owned();
        rig.engine.on_event(&mut key, Event::Waiting { until: 1_800_000_000 });
        assert_eq!(rig.engine.hosted["s"].limit_until, Some(1_800_000_000));
        rig.engine.on_event(&mut key, Event::Started { session: "s".into() });
        assert_eq!(rig.engine.hosted["s"].limit_until, None, "the wish came back from its wait");
        rig.engine.on_event(&mut key, Event::Failed(Failure::new("io", "pipe closed")));
        assert!(rig.engine.hosted["s"].ended_at.is_some());
        assert_eq!(rig.failures(), vec![(Action::Session, Some("s".to_owned()), "pipe closed".to_owned())]);
    }

    #[test]
    fn commands_for_a_run_this_app_does_not_hold_report_an_error() {
        let mut rig = rig();
        rig.engine.handle(Command::Interject { session: "gone".into(), text: "hello".into() });
        rig.engine.handle(Command::Stop("gone".into()));
        let actions: Vec<Action> = rig.failures().into_iter().map(|f| f.0).collect();
        assert_eq!(actions, [Action::Interject, Action::Stop]);
    }

    #[test]
    fn resume_failures_reach_the_screen() {
        let mut rig = rig();
        rig.engine.handle(Command::Resume { session: "missing".into(), note: Some("go".into()) });
        let failures = rig.failures();
        assert_eq!(failures[0].0, Action::Resume);
        assert!(failures[0].2.contains("missing"), "{}", failures[0].2);
        assert!(rig.engine.hosted.is_empty(), "no goal was reopened, so nothing is held");
    }

    fn memory_entries(home: &Path) -> (i64, i64) {
        Memory::open(home).unwrap();
        let db = rusqlite::Connection::open(home.join("memory/memory.db")).unwrap();
        let insert = |title: &str| {
            db.execute(
                "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('proj', 'u1', ?, 'summary', 'first body', '', 'wish', '[]', '/nowhere', 1000.0)",
                [title],
            )
            .unwrap();
            db.last_insert_rowid()
        };
        (insert("one"), insert("two"))
    }

    #[test]
    fn memory_can_be_listed_corrected_and_deleted() {
        let mut rig = rig();
        let home = rig.engine.home.clone();
        assert_eq!(read_memory(&home).unwrap(), vec![], "an empty home has no memory");
        assert!(!home.join("memory").exists(), "looking must not create the database");
        let (first, second) = memory_entries(&home);
        rig.engine.handle(Command::Memory(read_memory(&home)));
        assert_eq!(rig.engine.memory.len(), 2);
        assert!(!rig.engine.memory[0].stale);

        rig.engine.handle(Command::CorrectMemory(first, "the right text".into()));
        let body = |engine: &Engine, id: i64| engine.memory.iter().find(|m| m.id == id).map(|m| m.body.clone());
        assert_eq!(body(&rig.engine, first).as_deref(), Some("the right text"));
        let stored = read_memory(&home).unwrap();
        assert_eq!(stored.iter().find(|m| m.id == first).unwrap().body.as_ref(), "the right text");

        rig.engine.handle(Command::DeleteMemory(second));
        assert!(body(&rig.engine, second).is_none());
        assert_eq!(read_memory(&home).unwrap().len(), 1);
        assert!(rig.failures().is_empty());

        rig.engine.handle(Command::DeleteMemory(second));
        assert_eq!(rig.failures()[0].0, Action::DeleteMemory);
        assert_eq!(rig.engine.memory.len(), 1, "a failed delete changes nothing");
    }

    #[test]
    fn an_unchanged_snapshot_is_not_sent_again() {
        let mut rig = rig();
        goal_file(&rig.engine.home.clone(), "s1", base_goal("blocked"));
        rig.engine.tick();
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Runs { .. })));
        rig.engine.tick();
        assert!(!rig.updates().iter().any(|u| matches!(u, Update::Runs { .. })));
    }

    #[test]
    fn a_session_that_ends_before_recording_a_goal_is_reported_and_removed() {
        let mut rig = rig();
        rig.engine.hosted.insert("pending-1".into(), Hosted::new(true, "w".into(), PathBuf::new()));
        let mut key = "pending-1".to_owned();
        rig.engine.on_event(&mut key, Event::Exited { code: 1 });
        rig.engine.tick();
        assert!(rig.engine.hosted.is_empty());
        let failures = rig.failures();
        assert_eq!(failures[0].0, Action::StartWish);
        assert!(failures[0].2.contains("code 1"), "{}", failures[0].2);
    }

    #[test]
    fn briefs_give_their_goal_line_in_both_layouts() {
        assert_eq!(goal_line("GOAL   do it\nSCOPE x").as_deref(), Some("do it"));
        assert_eq!(goal_line("intro\nGOAL\n\n  do it\nSCOPE x").as_deref(), Some("do it"));
        assert_eq!(goal_line("GOAL\nSCOPE x"), None);
        assert_eq!(goal_line("GOALS are nice"), None);
        assert_eq!(goal_line(""), None);
    }

    #[test]
    fn dates_and_durations_read_naturally() {
        assert_eq!(started_text("20261103-051500"), "Nov 3, 05:15");
        assert_eq!(started_text("bad"), "");
        assert_eq!(elapsed_text(30).as_ref(), "<1 m");
        assert_eq!(elapsed_text(61).as_ref(), "1 m");
        assert_eq!(elapsed_text(4 * 3600 + 36 * 60).as_ref(), "4 h 36 m");
    }
}
