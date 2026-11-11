//! Connects the screens to the real engine. One worker thread owns the `Poller` and every hosted
//! `host::Wish`; it takes commands over a channel and sends finished `Run`s back. The UI thread
//! never reads the disk, starts a process or drops a `Wish` (dropping one waits for its process).
use super::model::*;
use crate::{
    app::{self, Poller, Snapshot},
    base::{self, Failure, Res},
    cli::doctor,
    goal,
    evolve::{self, digest::{self, Digest}},
    host::{self, Event},
    memory::{Memory, MemoryPriorRow},
    session::{self, Config},
    usage::{self, Ledger, Locations, Warning},
};

mod board;
mod quota;
#[cfg(test)]
mod accept;
use crate::usage::quota::{Provider, ProviderQuota, Quotas};
use gpui_kit::{App, Entity, SharedString};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}},
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
    Board { choice: crate::board::Choice, lang: crate::board::Lang },
    BoardDone { session: String, revision: u64, result: Res<crate::board::Summary>, real_call: bool },
    EvolveCheck,
    EvolveChecked(Res<(Digest, EvolvePreview, Vec<PastEvolve>)>),
    EvolveStart(PathBuf),
    EvolveStarted { result: Res<EvolveOutcome>, runs: Res<Vec<PastEvolve>> },
    Start(WishRequest),
    Interject { session: String, text: String },
    Stop(String),
    Resume { session: String, note: Option<String> },
    DeleteMemory(i64),
    CorrectMemory(i64, String),
    RestoreMemory { id: i64, prior: MemoryPrior },
    Reload(FeedKind),
    Refresh,
    Recheck,
    Follow(String),
    Ledger { session: String, activity: Option<String>, result: Res<Ledger> },
    Memory(Res<Vec<MemoryEntry>>),
    Checks(Vec<Check>),
    CodexRead { result: Res<Value>, generation: u64, at: i64 },
    CodexUpdate { params: Value, at: i64 },
    Quit,
}

enum Update {
    Board { session: String, entry: BoardEntry },
    BoardSpent(crate::board::TokenUse),
    EvolveChecked(Res<(EvolvePreview, Vec<PastEvolve>)>),
    EvolveStarted { result: Res<EvolveOutcome>, runs: Res<Vec<PastEvolve>> },
    EvolveBusy,
    Runs { runs: Vec<Run>, renamed: Vec<(String, String)>, open: Option<String> },
    Memory(Vec<MemoryEntry>),
    Corrected { id: i64, prior: MemoryPrior },
    Feed { kind: FeedKind, error: Option<String> },
    Quota(Quotas),
    CodexSlots(Option<u64>),
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
    /// Receives one message, or disconnects, when the worker has finished and dropped everything it owned.
    finished: Mutex<mpsc::Receiver<()>>,
}

impl LiveBackend {
    fn send(&self, command: Command) -> Res<()> {
        self.commands
            .send(command)
            .map_err(|_| Failure::new("engine_stopped", "the engine worker is no longer running"))
    }
}

impl Backend for LiveBackend {
    fn evolve_check(&self) -> Res<()> { self.send(Command::EvolveCheck) }
    fn evolve_start(&self, repo: &Path) -> Res<()> { self.send(Command::EvolveStart(repo.to_path_buf())) }
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

    fn restore_memory(&self, id: i64, prior: &MemoryPrior) -> Res<()> {
        self.send(Command::RestoreMemory { id, prior: prior.clone() })
    }

    fn reload(&self, kind: FeedKind) -> Res<()> { self.send(Command::Reload(kind)) }

    fn refresh(&self) -> Res<()> {
        self.send(Command::Refresh)
    }

    fn recheck(&self) -> Res<()> {
        self.send(Command::Recheck)
    }

    fn follow(&self, session: &str) -> Res<()> {
        self.send(Command::Follow(session.into()))
    }

    fn shutdown(&self, wait: Duration) {
        let _ = self.commands.send(Command::Quit);
        // Dropping a hosted wish waits for its process tree, so the worker needs a moment.
        let finished = self.finished.lock().unwrap_or_else(|e| e.into_inner());
        let _ = finished.recv_timeout(wait);
    }
}

impl Drop for LiveBackend {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
    }
}

/// Runs `engine` on its own thread. The returned backend's `shutdown` ends it and waits.
fn launch(engine: Engine, commands: mpsc::Sender<Command>, receiver: mpsc::Receiver<Command>) -> LiveBackend {
    let (done, finished) = mpsc::channel();
    std::thread::Builder::new()
        .name("chronos-engine".into())
        .spawn(move || {
            // `run` owns the engine, so every hosted wish is gone when it returns.
            engine.run(receiver);
            let _ = done.send(());
        })
        .expect("could not start the engine worker");
    LiveBackend { commands, finished: Mutex::new(finished) }
}

/// Starts the worker and keeps `state` up to date from it. The task holds `state` weakly: the
/// state owns the backend, and the worker only exits once the backend has told it to quit.
pub fn connect(state: Entity<AppState>, cx: &mut App) {
    let (commands, receiver) = mpsc::channel();
    let inbox = Inbox::default();
    let locations = Locations::from_env();
    let engine = Engine::new(locations.home, locations.projects, Box::new(Config::from_env), commands.clone(), inbox.clone());
    let prefs = settings_board(cx);
    let mut prior = None;
    if board_settings_changed(&mut prior, prefs) {
        let _ = commands.send(Command::Board { choice: prefs.0, lang: prefs.1 });
    }
    let board_commands = commands.clone();
    let observer = cx.observe_global::<super::settings::Store>(move |cx| {
        let next = settings_board(cx);
        if board_settings_changed(&mut prior, next) {
            let _ = board_commands.send(Command::Board { choice: next.0, lang: next.1 });
        }
    });
    let backend = launch(engine, commands, receiver);
    state.update(cx, |s, _| s.backend = Some(Box::new(backend)));
    let state = state.downgrade();
    cx.spawn(async move |cx| {
        let _observer = observer;
        let mut followed: Option<SharedString> = None;
        while let Some(updates) = inbox.next().await {
            let alive = cx.update(|cx| {
                let mut spent = Vec::new();
                let result = state.update(cx, |s, cx| {
                    spent = apply(s, updates);
                    let viewing = s.run().map(|run| run.session.clone());
                    if viewing != followed {
                        followed = viewing;
                        s.follow_current();
                    }
                    cx.notify();
                });
                for tokens in spent { super::settings::add_board_tokens(tokens.input, tokens.output, cx); }
                result
            });
            if alive.is_err() {
                return;
            }
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

fn board_settings_changed(prior: &mut Option<(crate::board::Choice, crate::board::Lang)>, next: (crate::board::Choice, crate::board::Lang)) -> bool {
    if *prior == Some(next) { return false; }
    *prior = Some(next);
    true
}

fn next_wake(now: Instant, next_poll: Instant, board_wake: Option<Instant>) -> Duration {
    board_wake.map_or(next_poll, |at| at.min(next_poll)).saturating_duration_since(now)
}

fn settings_board(cx: &App) -> (crate::board::Choice, crate::board::Lang) {
    let settings = super::settings::Store::current(cx);
    map_board_settings(settings.board, settings.language.resolve())
}

fn map_board_settings(choice: super::settings::BoardChoice, lang: super::i18n::Lang) -> (crate::board::Choice, crate::board::Lang) {
    use super::{i18n, settings::BoardChoice};
    (match choice { BoardChoice::Luna => crate::board::Choice::Luna, BoardChoice::Haiku => crate::board::Choice::Haiku, BoardChoice::Off => crate::board::Choice::Off },
     match lang { i18n::Lang::En => crate::board::Lang::En, i18n::Lang::ZhTw => crate::board::Lang::ZhTw })
}

fn apply(state: &mut AppState, updates: Vec<Update>) -> Vec<crate::board::TokenUse> {
    let mut spent = Vec::new();
    for update in updates {
        match update {
            Update::Board { session, entry } => { state.board.insert(session.into(), entry); }
            Update::BoardSpent(tokens) => spent.push(tokens),
            Update::EvolveChecked(result) => {
                state.evolve.checking = false;
                match result {
                    Ok((preview, runs)) => { state.evolve.preview = Some(preview); state.evolve.runs = runs; }
                    Err(failure) => state.evolve.error = Some(failure.into()),
                }
            }
            Update::EvolveStarted { result, runs } => {
                state.evolve.starting = false;
                match result { Ok(outcome) => { state.evolve.error = None; state.evolve.outcome = Some(outcome); }, Err(failure) => { state.evolve.outcome = None; state.evolve.error = Some(failure.into()); } }
                match runs { Ok(runs) => state.evolve.runs = runs, Err(failure) if state.evolve.error.is_none() => state.evolve.error = Some(failure.into()), Err(_) => {} }
            }
            Update::EvolveBusy => state.evolve.error = Some(Failure::new("busy", "an evolve run is already in progress").into()),
            Update::Runs { runs, renamed, open } => state.replace_runs(runs, &renamed, open.as_deref()),
            Update::Memory(memory) => state.memory = memory,
            Update::Quota(quotas) => {
                state.quota = quotas;
                state.feeds.quota.loaded = true;
            },
            Update::CodexSlots(used) => state.codex_slots = used,
            Update::Corrected { id, prior } => state.record_undo(Undoable::Memory { id, prior }),
            Update::Feed { kind, error } => {
                let feed = state.feeds.get_mut(kind);
                match error {
                    None => { feed.loaded = true; feed.error = None; }
                    Some(message) => feed.error = Some((message.into(), SystemTime::now())),
                }
            }
            Update::Checks(checks) => state.checks = checks,
            Update::Failed { action, session, message } => state.report(action, session.map(Into::into), message),
        }
    }
    spent
}

fn evolve_preview(digest: &Digest, repo: Option<PathBuf>) -> EvolvePreview {
    EvolvePreview {
        issues: digest.problems.iter().take(digest::MAX_PROBLEMS).map(|p| EvolveIssue { id: p.id.clone().into(), title: p.title.clone().into(), count: p.count, cost_usd: p.cost_usd }).collect(),
        more_issues: digest.problems.len().saturating_sub(digest::MAX_PROBLEMS),
        estimate_usd: digest.estimate.median_usd, estimate_samples: digest.estimate.samples,
        gaps: digest.gaps.iter().map(|g| EvolveGap { family: g.family.clone().into(), location: g.location.clone().into(), reason: g.reason.clone().into() }).collect(),
        repo, text: digest::render(digest).into(),
    }
}

fn evolve_runs(home: &Path, digest: &Digest) -> Res<Vec<PastEvolve>> {
    let runs = evolve::list_runs(home)?;
    Ok(digest::verdicts(&runs, digest).into_iter().map(|v| PastEvolve {
        id: v.run.id.into(), started_ms: v.run.started_ms, repo: v.run.repo.into(), branch: v.run.branch.into(), state: v.run.state.into(), verdict: v.verdict,
    }).collect())
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

type EvolveJob = dyn Fn(&evolve::Inputs, &Path, &Config) -> Res<evolve::StartOutcome> + Send + Sync;

/// Everything that is owned by the worker thread.
struct Engine {
    board_scheduler: crate::board::Scheduler,
    board_cache: Arc<Mutex<crate::board::Cache>>,
    board_model: Option<Arc<dyn crate::board::Model>>,
    board_worker: Option<mpsc::Sender<board::Job>>,
    board_cancel: Arc<AtomicBool>,
    board_choice: crate::board::Choice,
    board_lang: crate::board::Lang,
    board_epoch: u64,
    board_known: HashSet<String>,
    board_flights: HashMap<String, u64>,
    board_generations: HashMap<String, u64>,
    board_entries: HashMap<String, BoardEntry>,
    quotas: Quotas,
    codex: Arc<quota::SharedCodex>,
    rate_source: Arc<dyn quota::RateSource>,
    quota_stop: Arc<AtomicBool>,
    quota_events_started: Arc<AtomicBool>,
    quota_busy: bool,
    quota_reload_pending: bool,
    quota_started: Instant,
    quota_last: Option<(Instant, u64, u64)>,
    runs_revision: u64,
    evolve_digest: Option<Digest>,
    evolve_starting: bool,
    evolve_job: Arc<EvolveJob>,
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
    ledger_reload_pending: bool,
    memory_busy: bool,
    memory_read: Option<Instant>,
    memory_dirty: bool,
    memory: Vec<MemoryEntry>,
    followed: Option<String>,
    titles: HashMap<PathBuf, SharedString>,
    reported: HashSet<String>,
    feed_sent: Vec<(FeedKind, Option<String>)>,
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
        let codex = Arc::new(quota::SharedCodex::default());
        Self {
            board_scheduler: crate::board::Scheduler::new(Duration::from_secs(120)),
            board_cache: Arc::new(Mutex::new(crate::board::Cache::new())),
            board_model: None, board_worker: None,
            board_cancel: Arc::new(AtomicBool::new(false)),
            board_choice: crate::board::Choice::Off, board_lang: crate::board::Lang::ZhTw,
            board_epoch: 0, board_known: HashSet::new(), board_flights: HashMap::new(), board_generations: HashMap::new(), board_entries: HashMap::new(),
            quotas: Quotas::default(),
            rate_source: codex.clone(),
            codex,
            quota_stop: Arc::new(AtomicBool::new(false)),
            quota_events_started: Arc::new(AtomicBool::new(false)),
            quota_busy: false,
            quota_reload_pending: false,
            quota_started: Instant::now(),
            quota_last: None,
            runs_revision: 0,
            evolve_digest: None,
            evolve_starting: false,
            evolve_job: Arc::new(|inputs, repo, config| evolve::start(inputs, Some(repo), config, Box::new(std::io::empty()), &mut std::io::sink())),
            poller: Poller::new(&home),
            home,
            projects,
            config,
            hosted: HashMap::new(),
            aliases: HashMap::new(),
            snapshot: None,
            ledgers: HashMap::new(),
            ledger_busy: false,
            ledger_reload_pending: false,
            memory_busy: false,
            memory_read: None,
            memory_dirty: true,
            memory: vec![],
            followed: None,
            titles: HashMap::new(),
            reported: HashSet::new(),
            feed_sent: vec![],
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
        'running: loop {
            let now = Instant::now();
            let wake = next_wake(now, next_poll, self.board_scheduler.next_wake(now));
            match commands.recv_timeout(wake) {
                Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(command) => {
                    let mut poll_now = self.handle(command);
                    while let Ok(command) = commands.try_recv() {
                        if matches!(command, Command::Quit) {
                            break 'running;
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
                    let now = Instant::now();
                    if now >= next_poll {
                        self.tick();
                        next_poll = Instant::now() + self.interval();
                    } else { self.board_due(now); }
                }
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.board_cancel.store(true, Ordering::Release);
        self.board_worker.take();
        self.quota_stop.store(true, Ordering::Release);
        self.codex.close();
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
        self.merge_snapshot_quotas();
        self.reap();
        self.report_warnings();
        self.schedule_ledger();
        self.schedule_memory();
        self.publish();
        self.schedule_quota(false);
    }

    /// Runs one command. Returns whether the disk should be read again right away.
    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Board { choice, lang } => { self.set_board(choice, lang, Instant::now()); false }
            Command::BoardDone { session, revision, result, real_call } => {
                self.board_done(session, revision, result, real_call, Instant::now()); false
            }
            Command::EvolveCheck => { self.evolve_check(); false }
            Command::EvolveChecked(result) => {
                let result = result.map(|(digest, preview, runs)| { self.evolve_digest = Some(digest); (preview, runs) });
                self.inbox.push(Update::EvolveChecked(result));
                false
            }
            Command::EvolveStart(repo) => { self.evolve_start(repo); false }
            Command::EvolveStarted { result, runs } => {
                self.evolve_starting = false;
                self.inbox.push(Update::EvolveStarted { result, runs });
                false
            }
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
            Command::RestoreMemory { id, prior } => { self.restore_memory(id, prior); false }
            Command::Reload(kind) => match kind {
                FeedKind::Runs => true,
                FeedKind::Quota => { self.schedule_quota(true); true }
                FeedKind::Memory => { self.memory_dirty = true; self.schedule_memory(); false }
                FeedKind::Checks => { self.recheck(); false }
                FeedKind::Usage => {
                    self.ledgers.clear();
                    if self.ledger_busy { self.ledger_reload_pending = true; }
                    else { self.schedule_ledger(); }
                    false
                }
            },
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
                if self.ledger_reload_pending {
                    self.ledger_reload_pending = false;
                    self.schedule_ledger();
                    return false;
                }
                let ledger = match result {
                    Ok(ledger) => { self.feed(FeedKind::Usage, None); Some(ledger) },
                    Err(e) if e.kind == "not_found" => { self.feed(FeedKind::Usage, None); None },
                    Err(e) => {
                        self.feed(FeedKind::Usage, Some(e.message.clone()));
                        self.warn_once(format!("usage for {session}: {}", e.message));
                        None
                    }
                };
                self.ledgers.insert(session, LedgerEntry { ledger, activity, at: Instant::now() });
                false
            }
            Command::Memory(result) => {
                let superseded = self.memory_busy && self.memory_dirty;
                self.memory_busy = false;
                if superseded { self.schedule_memory(); return false; }
                match result {
                    Ok(memory) => {
                        self.feed(FeedKind::Memory, None);
                        if memory != self.memory {
                            self.memory = memory.clone();
                            self.inbox.push(Update::Memory(memory));
                        }
                    }
                    Err(e) => { self.feed(FeedKind::Memory, Some(e.message.clone())); self.warn_once(format!("memory: {}", e.message)); },
                }
                if self.memory_dirty { self.schedule_memory(); }
                false
            }
            Command::Checks(checks) => {
                self.inbox.push(Update::Checks(checks));
                self.feed(FeedKind::Checks, None);
                false
            }
            Command::CodexRead { result, generation, at } => {
                self.quota_busy = false;
                if let Some(last) = &mut self.quota_last { last.1 = generation; }
                match result {
                    Ok(value) => {
                        self.feed(FeedKind::Quota, None);
                        self.merge_quota(Provider::Codex, crate::usage::quota::parse_codex_read(&value, at));
                    }
                    Err(error) => self.feed(FeedKind::Quota, Some(error.message)),
                }
                if self.quota_reload_pending {
                    self.quota_reload_pending = false;
                    self.schedule_quota(true);
                }
                false
            }
            Command::CodexUpdate { params, at } => {
                self.merge_quota(Provider::Codex, crate::usage::quota::parse_codex_update(&params, at));
                false
            }
            Command::Quit => false,
        }
    }

    fn merge_quota(&mut self, provider: Provider, quota: ProviderQuota) {
        if self.quotas.merge(provider, quota) {
            self.inbox.push(Update::Quota(self.quotas.clone()));
        }
    }

    fn merge_snapshot_quotas(&mut self) {
        let quotas: Vec<_> = self.snapshot.iter().flat_map(|s| &s.wishes)
            .flat_map(|w| &w.runs).filter_map(|r| r.scan.as_ref()?.rate_limit.clone()).collect();
        for quota in quotas { self.merge_quota(Provider::Claude, quota); }
    }

    fn schedule_quota(&mut self, force: bool) {
        if self.quota_busy {
            if force { self.quota_reload_pending = true; }
            return;
        }
        let now = Instant::now();
        let active = self.sent.as_ref().is_some_and(|runs| runs.iter()
            .any(|run| matches!(run.status, RunStatus::Active | RunStatus::LimitWait)));
        let generation = self.rate_source.generation();
        if !force && !quota::due(now, self.quota_started, self.quota_last, generation, self.runs_revision, active) { return; }
        self.inbox.push(Update::CodexSlots(crate::codex::slots::in_use(&self.home).ok()));
        self.quota_busy = true;
        self.quota_last = Some((now, generation, self.runs_revision));
        let source = self.rate_source.clone();
        let commands = self.commands.clone();
        let stop = self.quota_stop.clone();
        let listening = self.quota_events_started.clone();
        std::thread::spawn(move || {
            // Subscribe before the first read, so notifications during startup are not lost.
            if !listening.swap(true, Ordering::AcqRel) {
                match source.events() {
                    Ok(events) => {
                        let tx = commands.clone();
                        let stop = stop.clone();
                        let listening = listening.clone();
                        std::thread::spawn(move || {
                            while !stop.load(Ordering::Acquire) {
                                match events.recv_timeout(Duration::from_millis(200)) {
                                    Ok(message) if message.get("method").and_then(Value::as_str) == Some("account/rateLimits/updated") => {
                                        let at = unix_now();
                                        if tx.send(Command::CodexUpdate { params: message.get("params").cloned().unwrap_or(Value::Null), at }).is_err() { break; }
                                    }
                                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                }
                            }
                            listening.store(false, Ordering::Release);
                        });
                    }
                    Err(_) => { listening.store(false, Ordering::Release); }
                }
            }
            if stop.load(Ordering::Acquire) { return; }
            let result = source.read();
            let generation = source.generation();
            let at = unix_now();
            let _ = commands.send(Command::CodexRead { result, generation, at });
        });
    }

    fn fail(&self, action: Action, session: Option<&str>, message: impl Into<String>) {
        self.inbox.push(Update::Failed { action, session: session.map(str::to_owned), message: message.into() });
    }

    fn feed(&mut self, kind: FeedKind, error: Option<String>) {
        if let Some((_, sent)) = self.feed_sent.iter_mut().find(|(k, _)| *k == kind) {
            if *sent == error { return; }
            *sent = error.clone();
        } else {
            self.feed_sent.push((kind, error.clone()));
        }
        self.inbox.push(Update::Feed { kind, error });
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
        let options = host::Options { creative: request.creative, max_turns: request.max_turns, advanced_research: request.advanced_research, council_budgets:request.council_budgets, codex_max_parallel: request.codex_max_parallel };
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
        let result = Memory::open(&self.home).and_then(|m| {
            let prior = m.prior(id)?;
            m.correct(id, text)?;
            Ok(prior)
        });
        match result {
            Ok(prior) => {
                if let Some(entry) = self.memory.iter_mut().find(|m| m.id == id) {
                    entry.body = text.trim().to_owned().into();
                    entry.corrected = true;
                    entry.confirmed = confirmed_text(app::unix(SystemTime::now()).unwrap_or(0) as f64).into();
                }
                self.inbox.push(Update::Memory(self.memory.clone()));
                self.inbox.push(Update::Corrected { id, prior: MemoryPrior {
                    body: prior.body, summary: prior.summary, corrected: prior.corrected, confirmed: prior.confirmed,
                }});
                self.memory_dirty = true;
            }
            Err(failure) => self.fail(Action::CorrectMemory, None, failure.message),
        }
    }

    fn restore_memory(&mut self, id: i64, prior: MemoryPrior) {
        let row = MemoryPriorRow { body: prior.body, summary: prior.summary, corrected: prior.corrected, confirmed: prior.confirmed };
        match Memory::open(&self.home).and_then(|m| m.restore(id, &row)) {
            Ok(()) => {
                if let Some(entry) = self.memory.iter_mut().find(|m| m.id == id) {
                    entry.body = row.body.into();
                    entry.corrected = row.corrected.is_some();
                    entry.confirmed = confirmed_text(row.confirmed).into();
                }
                self.inbox.push(Update::Memory(self.memory.clone()));
                self.memory_dirty = true;
            }
            Err(failure) => self.fail(Action::CorrectMemory, None, failure.message),
        }
    }

    fn evolve_inputs(&self) -> evolve::Inputs {
        let now_ms = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64;
        evolve::Inputs { home: self.home.clone(), projects: self.projects.clone(), now_ms, since_days: evolve::DEFAULT_SINCE_DAYS }
    }

    fn evolve_check(&self) {
        let inputs = self.evolve_inputs();
        let commands = self.commands.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let preview = evolve::preview(&inputs)?;
                let runs = evolve_runs(&inputs.home, &preview.digest)?;
                let repo = evolve::repo::remembered(&inputs.home)?;
                let view = evolve_preview(&preview.digest, repo);
                Ok((preview.digest, view, runs))
            })();
            let _ = commands.send(Command::EvolveChecked(result));
        });
    }

    fn evolve_start(&mut self, repo: PathBuf) {
        if self.evolve_starting { self.inbox.push(Update::EvolveBusy); return; }
        self.evolve_starting = true;
        let inputs = self.evolve_inputs();
        let config = (self.config)();
        let digest = self.evolve_digest.clone();
        let commands = self.commands.clone();
        let job = self.evolve_job.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&inputs, &repo, &config)))
                .unwrap_or_else(|_| Err(Failure::new("engine_stopped", "evolve start worker panicked")))
                .map(|outcome| match outcome {
                    evolve::StartOutcome::NoOp { reason } => EvolveOutcome::NoOp { reason: reason.into() },
                    evolve::StartOutcome::Ran { exit_code, .. } => EvolveOutcome::Ran { exit_code },
                });
            let runs = (|| {
                let digest = match digest { Some(digest) => digest, None => evolve::preview(&inputs)?.digest };
                evolve_runs(&inputs.home, &digest)
            })();
            let _ = commands.send(Command::EvolveStarted { result, runs });
        });
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
            Event::RateLimit(quota) => self.merge_quota(Provider::Claude, quota),
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
        let error = if snapshot.wishes.is_empty() { loose.first().cloned() } else { None };
        if !self.feed_sent.iter().any(|(kind, _)| *kind == FeedKind::Runs) {
            self.feed(FeedKind::Runs, None);
        }
        self.feed(FeedKind::Runs, error);
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
        let Some(wish) = due else {
            if !self.feed_sent.iter().any(|(kind, _)| *kind == FeedKind::Usage) {
                self.feed(FeedKind::Usage, None);
            }
            return
        };
        let (session, activity) = (wish.session.clone(), wish.last_activity.clone());
        let locations = Locations { home: self.home.clone(), projects: self.projects.clone() };
        let commands = self.commands.clone();
        self.ledger_busy = true;
        std::thread::spawn(move || {
            let result = usage::ledger(&locations, Some(&session));
            let _ = commands.send(Command::Ledger { session, activity, result });
        });
    }

    /// Listing hashes every file an entry names, so it runs off this thread and at most every `MEMORY_EVERY`.
    fn schedule_memory(&mut self) {
        if self.memory_busy {
            return;
        }
        // Whether an entry is stale depends on its source files, which the database's own time
        // does not show, so it is listed again on this cadence even when the database is unchanged.
        let due = self.memory_dirty
            || match self.memory_read {
                None => true,
                Some(at) => at.elapsed() >= MEMORY_EVERY,
            };
        if !due {
            return;
        }
        self.memory_dirty = false;
        self.memory_read = Some(Instant::now());
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
        if changed { self.runs_revision = self.runs_revision.wrapping_add(1); }
        self.sent = Some(runs.clone());
        self.inbox.push(Update::Runs { runs, renamed: std::mem::take(&mut self.renamed), open: self.open.take() });
        self.observe_board(Instant::now());
    }

    fn board_entry(&mut self, session: &str, status: BoardStatus) {
        let entry = self.board_entries.entry(session.to_owned()).or_insert(BoardEntry { text: None, status: BoardStatus::Idle });
        entry.status = status;
        self.inbox.push(Update::Board { session: session.to_owned(), entry: entry.clone() });
    }

    fn set_board(&mut self, choice: crate::board::Choice, lang: crate::board::Lang, now: Instant) {
        if self.board_choice == choice && self.board_lang == lang { return; }
        // A choice or language change alone is not a new run event. It affects the next job.
        if self.board_choice != crate::board::Choice::Off && choice != crate::board::Choice::Off {
            self.board_choice = choice;
            self.board_lang = lang;
            return;
        }
        self.board_cancel.store(true, Ordering::Release);
        self.board_cancel = Arc::new(AtomicBool::new(false));
        if choice == crate::board::Choice::Off || self.board_choice == crate::board::Choice::Off {
            self.board_scheduler = crate::board::Scheduler::new(Duration::from_secs(120));
            self.board_known.clear();
            self.board_flights.clear();
        }
        self.board_epoch = self.board_epoch.wrapping_add(1);
        self.board_choice = choice;
        self.board_lang = lang;
        self.observe_board(now);
    }

    fn observe_board(&mut self, now: Instant) {
        let runs = self.sent.clone().unwrap_or_default();
        let active: HashSet<String> = runs.iter().filter(|run| run.status != RunStatus::Done).map(|run| run.session.to_string()).collect();
        for session in self.board_known.difference(&active) {
            self.board_scheduler.forget(session);
            self.board_flights.remove(session);
            let generation = self.board_generations.entry(session.clone()).or_default();
            *generation = generation.wrapping_add(1);
        }
        self.board_known = active;
        for run in runs.iter().filter(|run| run.status != RunStatus::Done) {
            if self.board_choice == crate::board::Choice::Off {
                if !matches!(self.board_entries.get(run.session.as_ref()).map(|e| &e.status), Some(BoardStatus::Off)) {
                    self.board_entry(run.session.as_ref(), BoardStatus::Off);
                }
            } else {
                let generation = self.board_generations.get(run.session.as_ref()).copied().unwrap_or(0);
                self.board_scheduler.observe(run.session.as_ref(), board_revision(run, self.board_epoch, generation), now);
            }
        }
        self.board_due(now);
    }

    fn board_due(&mut self, now: Instant) {
        if self.board_choice == crate::board::Choice::Off { return; }
        for (session, revision) in self.board_scheduler.due(now) {
            let Some(run) = self.sent.as_ref().and_then(|runs| runs.iter().find(|r| r.session.as_ref() == session)) else {
                self.board_scheduler.forget(&session);
                continue;
            };
            let job = board::Job {
                store: self.snapshot.as_ref().and_then(|s| s.wishes.iter().find(|w| w.session == session)).and_then(|w| w.store.clone()),
                repo: run.cwd.clone(), events: run.log.iter().skip(run.log.len().saturating_sub(20)).map(|l| format!("{}: {}", l.source, l.text)).collect(),
                session: session.clone(), revision, choice: self.board_choice, lang: self.board_lang, cancel: self.board_cancel.clone(),
            };
            if self.board_worker.is_none() {
                let (sender, receiver) = mpsc::channel::<board::Job>();
                let commands = self.commands.clone();
                let cache = self.board_cache.clone();
                let codex = self.codex.clone();
                let claude = (self.config)().claude;
                let mut model = self.board_model.clone();
                std::thread::Builder::new().name("chronos-board".into()).spawn(move || {
                    let build = || -> Res<Arc<dyn crate::board::Model>> {
                        Ok(Arc::new(crate::board::Real::new(codex.server()?, claude.clone().unwrap_or_default())))
                    };
                    while let Ok(job) = receiver.recv() {
                        let (result, real_call) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| board::execute(&job, &cache, &mut model, &build)))
                            .unwrap_or_else(|_| (Err(Failure::new("board_worker", "board worker panicked")), false));
                        let _ = commands.send(Command::BoardDone { session: job.session, revision: job.revision, result, real_call });
                    }
                }).expect("could not start board worker");
                self.board_worker = Some(sender);
            }
            self.board_entry(&session, BoardStatus::Writing);
            self.board_flights.insert(session.clone(), revision);
            if self.board_worker.as_ref().unwrap().send(job).is_err() {
                self.board_worker.take();
                self.board_flights.remove(&session);
                self.board_scheduler.finished(&session, revision, now);
                self.board_entry(&session, BoardStatus::Failed("board worker stopped".into()));
            }
        }
    }

    fn board_done(&mut self, session: String, revision: u64, result: Res<crate::board::Summary>, real_call: bool, now: Instant) {
        // A reply from before Off (or from a forgotten run) must not consume a newer flight.
        if self.board_flights.get(&session) != Some(&revision) {
            if !self.board_flights.contains_key(&session) { self.board_idle_if_writing(&session); }
            return;
        }
        self.board_flights.remove(&session);
        if !self.board_scheduler.finished(&session, revision, now) {
            self.board_idle_if_writing(&session);
            return;
        }
        match result {
            Ok(summary) => {
                let entry = BoardEntry { text: Some(BoardText { recent: summary.recent.into(), next: summary.next.map(Into::into), written_unix: unix_now() }), status: BoardStatus::Idle };
                self.board_entries.insert(session.clone(), entry.clone());
                self.inbox.push(Update::Board { session, entry });
                if real_call { self.inbox.push(Update::BoardSpent(summary.tokens)); }
            }
            Err(error) if error.kind == "cancelled" || error.kind == "off" => self.board_idle_if_writing(&session),
            Err(error) => self.board_entry(&session, BoardStatus::Failed(error.message.into())),
        }
    }
    fn board_idle_if_writing(&mut self, session: &str) {
        if matches!(self.board_entries.get(session).map(|entry| &entry.status), Some(BoardStatus::Writing)) {
            self.board_entry(session, BoardStatus::Idle);
        }
    }

}

fn board_revision(run: &Run, epoch: u64, generation: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    epoch.hash(&mut hash);
    generation.hash(&mut hash);
    format!("{:?}{:?}{:?}{:?}", run.status, run.units.iter().map(|u| (&u.id, u.state, &u.raw_state, u.attempts)).collect::<Vec<_>>(), run.stages.iter().map(|s| (&s.name, s.phase)).collect::<Vec<_>>(), run.needs).hash(&mut hash);
    for line in run.log.iter().skip(run.log.len().saturating_sub(20)) { line.time.hash(&mut hash); line.source.hash(&mut hash); line.text.hash(&mut hash); format!("{:?}", line.level).hash(&mut hash); }
    hash.finish()
}

fn unix_now() -> i64 { app::unix(SystemTime::now()).unwrap_or(0).min(i64::MAX as u64) as i64 }

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
                wish: clean_wish(entry["wish"].as_str().unwrap_or("")).into(),
                confirmed: entry["confirmed"].as_f64().map(confirmed_text).unwrap_or_default().into(),
                corrected: entry["corrected"].as_f64().is_some(),
            })
        })
        .collect())
}

/// "Nov 3, 05:15" for a unix time in seconds, or nothing when the time is not a plausible one.
fn confirmed_text(unix: f64) -> String {
    if unix.is_finite() && unix > 0.0 {
        started_text(&stamp_of(unix as i64))
    } else {
        String::new()
    }
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

/// The plan's items as the checklist shows them. A module id the map does not hold is left out
/// rather than guessed at, and a plan with no items gives no goals at all.
fn goal_items(goals: &[app::GoalView], modules: &[Module]) -> Vec<GoalItem> {
    goals
        .iter()
        .map(|goal| GoalItem {
            id: goal.id.clone().into(),
            title: goal.title.clone().into(),
            modules: goal
                .modules
                .iter()
                .filter_map(|id| modules.iter().position(|m| m.id.as_ref() == id.as_str()))
                .collect(),
        })
        .collect()
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
        let mut rows: Vec<ModelCost> = summary
            .models
            .iter()
            .map(|(name, model)| ModelCost {
                model: name.clone().into(),
                usd: model.cost_usd,
                input: model.input,
                cached: model.cache_read,
                cache_write: model.cache_creation,
                output: model.output,
                total: sum_u64(sum_u64(model.input, model.cache_read), sum_u64(model.cache_creation, model.output)),
            })
            .collect();
        if let Some(ledger) = ledger {
            for row in &ledger.by_agent {
                if !rows.iter().any(|m| m.model.as_ref() == row.model) {
                    rows.push(ModelCost { model: row.model.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None });
                }
            }
        }
        return rows;
    }
    if let Some(ledger) = ledger {
        let mut rows: Vec<ModelCost> = vec![];
        for row in &ledger.by_agent {
            let tally = &row.tally;
            match rows.iter_mut().find(|r| r.model.as_ref() == row.model.as_str()) {
                Some(found) => {
                    found.usd = add_known(found.usd, tally.usd);
                    found.input = sum_u64(found.input, (!tally.missing[0]).then_some(tally.input));
                    found.cached = sum_u64(found.cached, (!tally.missing[1]).then_some(tally.cache_read));
                    found.cache_write = sum_u64(found.cache_write, (!tally.missing[2]).then_some(tally.cache_write));
                    found.output = sum_u64(found.output, (!tally.missing[3]).then_some(tally.output));
                    found.total = sum_u64(found.total, (!tally.missing.iter().any(|m| *m)).then_some(tally.input + tally.cache_read + tally.cache_write + tally.output));
                }
                None => rows.push(ModelCost {
                    model: row.model.clone().into(),
                    usd: tally.usd,
                    input: (!tally.missing[0]).then_some(tally.input),
                    cached: (!tally.missing[1]).then_some(tally.cache_read),
                    cache_write: (!tally.missing[2]).then_some(tally.cache_write),
                    output: (!tally.missing[3]).then_some(tally.output),
                    total: (!tally.missing.iter().any(|m| *m)).then_some(tally.input + tally.cache_read + tally.cache_write + tally.output),
                }),
            }
        }
        for name in wish.runs.iter().filter_map(|r| r.summary.as_ref()).flat_map(|s| s.models.keys()) {
            if !rows.iter().any(|m| m.model.as_ref() == name.as_str()) {
                rows.push(ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None });
            }
        }
        return rows;
    }
    let mut names: Vec<&String> = wish.runs.iter().filter_map(|r| r.summary.as_ref()).flat_map(|s| s.models.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None })
        .collect()
}

/// Codex rows. Codex's input count includes its cached tokens; the screens add the columns
/// together, so the cached part is taken out of input here.
fn codex_models(wish: &app::Wish, ledger: Option<&Ledger>) -> Vec<ModelCost> {
    let row = |model: &str, input: u64, cached: u64, output: u64, missing: [bool; 3]| ModelCost {
        model: model.to_owned().into(),
        usd: (!missing.iter().any(|m| *m)).then(|| crate::usage::price::codex_usd(model, &crate::usage::price::CodexUsage { input, cached_input: cached, output })).flatten(),
        input: (!missing[0] && !missing[1]).then_some(input.saturating_sub(cached)),
        cached: (!missing[1]).then_some(cached),
        cache_write: None,
        output: (!missing[2]).then_some(output),
        total: (!missing[0] && !missing[2]).then_some(input + output),
    };
    let scans: Vec<&app::Scan> = wish.runs.iter().filter_map(|r| r.scan.as_ref()).collect();
    if !scans.is_empty() && scans.len() == wish.runs.len() && scans.iter().all(|s| s.complete) {
        let mut totals = std::collections::BTreeMap::<&str, app::Tokens>::new();
        for (name, tokens) in scans.iter().flat_map(|s| s.codex.iter()) {
            let total = totals.entry(name.as_str()).or_default();
            total.input += tokens.input;
            total.cached += tokens.cached;
            total.output += tokens.output;
            for (missing, part) in total.missing.iter_mut().zip(tokens.missing) { *missing |= part; }
        }
        let mut rows: Vec<ModelCost> = totals.iter().map(|(name, t)| row(name, t.input, t.cached, t.output, t.missing)).collect();
        if let Some(ledger) = ledger {
            for c in &ledger.codex {
                if !rows.iter().any(|m| m.model.as_ref() == c.model) {
                    rows.push(row(&c.model, c.tokens.input, c.tokens.cached_input, c.tokens.output, c.tokens.missing));
                }
            }
            add_missing_codex_models(&mut rows, ledger);
        }
        return rows;
    }
    if let Some(ledger) = ledger {
        let mut rows: Vec<ModelCost> = ledger.codex.iter().map(|c| row(&c.model, c.tokens.input, c.tokens.cached_input, c.tokens.output, c.tokens.missing)).collect();
        add_missing_codex_models(&mut rows, ledger);
        for name in scans.iter().flat_map(|s| s.codex.keys()) {
            if !rows.iter().any(|m| m.model.as_ref() == name.as_str()) {
                rows.push(ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None });
            }
        }
        return rows;
    }
    let mut names: Vec<&String> = scans.iter().flat_map(|s| s.codex.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None })
        .collect()
}

fn add_missing_codex_models(rows: &mut Vec<ModelCost>, ledger: &Ledger) {
    for name in &ledger.codex_missing_models {
        if let Some(row) = rows.iter_mut().find(|row| row.model.as_ref() == name) {
            row.usd = None;
            row.input = None;
            row.cached = None;
            row.output = None;
            row.total = None;
        } else {
            rows.push(ModelCost { model: name.clone().into(), usd: None, input: None, cached: None, cache_write: None, output: None, total: None });
        }
    }
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
        usd: if row.tokens.missing.iter().any(|missing| *missing) || ledger.codex_missing_models.contains(&row.model) { None } else {
            crate::usage::price::codex_usd(&row.model, &crate::usage::price::CodexUsage { input: row.tokens.input, cached_input: row.tokens.cached_input, output: row.tokens.output })
        },
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
    let codex = codex_models(wish, ledger);
    let codex_usd: f64 = codex.iter().filter_map(|m| m.usd).sum();
    models.extend(codex);
    Usage {
        total_usd: reported.or_else(|| ledger.and_then(|l| l.totals.usd)).map(|usd| usd + codex_usd),
        models,
        roles: ledger.map(roles_of).unwrap_or_default(),
        contexts: contexts_of(wish, ledger),
    }
}

fn note_of(warning: &Warning) -> Note {
    match warning {
        Warning::ContextHigh { agent_type, model, peak_context, .. } => {
            Note::ContextHigh { agent: agent_type.clone(), model: model.clone(), peak: *peak_context }
        }
        Warning::CacheRewritesAfterGap { agent_type, count, tokens, usd, .. } => {
            Note::CacheRewrites { agent: agent_type.clone(), count: *count, tokens: *tokens, usd: *usd }
        }
        Warning::LargeFileRead { agent_type, path, reads, largest_bytes, .. } => {
            Note::FileReads { agent: agent_type.clone(), path: path.clone(), reads: *reads, largest: *largest_bytes }
        }
        Warning::CopiedHistoryNotSeparable { file, zeroed_calls } => Note::CopiedHistory { file: file.clone(), calls: *zeroed_calls },
    }
}

fn map_wish(wish: &app::Wish, sources: &Sources<'_>, titles: &mut HashMap<PathBuf, SharedString>) -> Run {
    let (status, hosted) = status_of(wish, sources.hosted, sources.now);
    let mut warnings: Vec<SharedString> = vec![];
    if let Some(prefix) = store_prefix(wish) {
        warnings.extend(sources.warnings.iter().filter(|w| w.starts_with(&prefix)).map(|w| w.clone().into()));
    }
    let notes = sources.ledger.map(|ledger| group_notes(ledger.warnings.iter().map(note_of))).unwrap_or_default();
    let lang = super::i18n::data_lang();
    warnings.extend(notes.iter().map(|note| SharedString::from(note.text(lang))));
    let (modules, units) = structure(wish, titles, &mut warnings);
    let goals = goal_items(&wish.goals, &modules);
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
        wish: app::unmarked(&clean_wish(&wish.goal.wish)).to_owned().into(),
        status,
        hosted,
        started: first_stamp.map(started_text).unwrap_or_default().into(),
        started_at: started_unix.and_then(|s| u64::try_from(s).ok()),
        stages,
        goals,
        modules,
        units,
        usage: usage_of(wish, sources.ledger),
        needs,
        log: sources.hosted.map(|h| h.log.to_vec()).unwrap_or_default(),
        decisions: wish.decisions.iter().filter_map(|d| plain_line(d)).map(SharedString::from).collect(),
        report,
        elapsed,
        warnings,
        notes,
    }
}

/// A run with no goal on disk yet: only what the app itself knows, and every figure unknown.
fn early_run(key: &str, entry: &Hosted) -> Run {
    Run {
        session: key.to_owned().into(),
        project: entry.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into(),
        cwd: Some(entry.cwd.clone()).filter(|cwd| !cwd.as_os_str().is_empty()),
        wish: clean_wish(&entry.text).into(),
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
        notes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, prelude::v1::test};

    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct BoardFake {
        luna: AtomicUsize,
        haiku: AtomicUsize,
        active: AtomicUsize,
        maximum: AtomicUsize,
        fail_luna: bool,
        fail_haiku: bool,
        unknown_usage: bool,
        delay: bool,
    }
    impl BoardFake {
        fn call(&self, luna: bool) -> Res<(String, crate::board::TokenUse)> {
            let count = if luna { &self.luna } else { &self.haiku };
            count.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            if self.delay { std::thread::sleep(Duration::from_millis(40)); }
            self.active.fetch_sub(1, Ordering::SeqCst);
            if (luna && self.fail_luna) || (!luna && self.fail_haiku) {
                return Err(Failure::new("fake", "model failed"));
            }
            Ok(("RECENT: finished work\nNEXT: unknown".into(), crate::board::TokenUse {
                input: (!self.unknown_usage).then_some(12), output: (!self.unknown_usage).then_some(3),
            }))
        }
    }
    impl crate::board::Model for BoardFake {
        fn luna(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, crate::board::TokenUse)> { self.call(true) }
        fn haiku(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, crate::board::TokenUse)> { self.call(false) }
    }
    fn board_run(id: &str) -> Run { early_run(id, &Hosted::new(false, "wish".into(), PathBuf::new())) }
    fn board_event(run: &mut Run, text: &str) {
        run.log.push(LogLine { time: "now".into(), source: "test".into(), text: text.to_owned().into(), level: LogLevel::Info });
    }
    fn board_reply(rig: &Rig) -> Command {
        rig._receiver.recv_timeout(Duration::from_secs(5)).expect("board reply")
    }
    fn board_start(rig: &mut Rig, fake: Arc<BoardFake>, runs: Vec<Run>, choice: crate::board::Choice, now: Instant) {
        rig.engine.board_model = Some(fake);
        rig.engine.sent = Some(runs);
        rig.engine.set_board(choice, crate::board::Lang::En, now);
    }
    fn board_finish(rig: &mut Rig, now: Instant) {
        match board_reply(rig) {
            Command::BoardDone { session, revision, result, real_call } => rig.engine.board_done(session, revision, result, real_call, now),
            _ => panic!("unexpected reply"),
        }
    }

    struct BlockingBoard { started: mpsc::Sender<()> }
    impl crate::board::Model for BlockingBoard {
        fn luna(&self, _: &str, _: Instant, cancel: &AtomicBool) -> Res<(String, crate::board::TokenUse)> {
            self.started.send(()).unwrap();
            while !cancel.load(Ordering::Acquire) { std::thread::yield_now(); }
            Err(Failure::new("cancelled", "cancelled"))
        }
        fn haiku(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, crate::board::TokenUse)> { panic!("unexpected fallback") }
    }
    fn start_blocking_board(rig: &mut Rig) -> Arc<AtomicBool> {
        let (started, wait) = mpsc::channel();
        rig.engine.board_model = Some(Arc::new(BlockingBoard { started }));
        rig.engine.sent = Some(vec![board_run("a")]);
        rig.engine.set_board(crate::board::Choice::Luna, crate::board::Lang::En, Instant::now());
        wait.recv_timeout(Duration::from_secs(5)).expect("board model did not start");
        rig.engine.board_cancel.clone()
    }

    #[test]
    fn board_off_cancels_in_flight_and_replaces_flag() {
        let mut rig = rig();
        let flag = start_blocking_board(&mut rig);
        rig.engine.set_board(crate::board::Choice::Off, crate::board::Lang::En, Instant::now());
        assert!(flag.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&flag, &rig.engine.board_cancel));
        assert!(!rig.engine.board_cancel.load(Ordering::Acquire));
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Off);
        let reply = board_reply(&rig);
        rig.engine.handle(reply);
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Off);
    }

    #[test]
    fn shutdown_cancels_board_before_closing_codex() {
        let mut rig = rig();
        let flag = start_blocking_board(&mut rig);
        let checked = Arc::new(AtomicBool::new(false));
        let (observed_flag, observed_check) = (flag.clone(), checked.clone());
        *rig.engine.codex.on_close.lock().unwrap() = Some(Box::new(move || {
            assert!(observed_flag.load(Ordering::Acquire), "closed before cancellation");
            observed_check.store(true, Ordering::Release);
        }));
        rig.engine.shutdown();
        assert!(checked.load(Ordering::Acquire));
        assert!(rig.engine.codex.server().is_err());
        board_reply(&rig);
    }

    #[test]
    fn board_worker_failure_does_not_leave_a_flight() {
        struct Panics;
        impl crate::board::Model for Panics {
            fn luna(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, crate::board::TokenUse)> { panic!("model panic") }
            fn haiku(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, crate::board::TokenUse)> { panic!("model panic") }
        }
        let mut rig = rig();
        rig.engine.board_model = Some(Arc::new(Panics));
        rig.engine.sent = Some(vec![board_run("a")]);
        let now = Instant::now();
        rig.engine.set_board(crate::board::Choice::Luna, crate::board::Lang::En, now);
        board_finish(&mut rig, now);
        assert!(!rig.engine.board_flights.contains_key("a"));
        assert!(matches!(rig.engine.board_entries["a"].status, BoardStatus::Failed(_)));

        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        rig.engine.board_worker = Some(sender);
        rig.engine.sent = Some(vec![board_run("b")]);
        rig.engine.observe_board(now);
        assert!(!rig.engine.board_flights.contains_key("b"));
        assert!(matches!(rig.engine.board_entries["b"].status, BoardStatus::Failed(_)));
    }

    #[test]
    fn board_missing_run_is_forgotten_and_can_be_scheduled_again() {
        let mut rig = rig();
        let now = Instant::now();
        rig.engine.board_choice = crate::board::Choice::Luna;
        rig.engine.board_scheduler.observe("a", 1, now);
        rig.engine.board_due(now);
        rig.engine.board_scheduler.observe("a", 1, now);
        assert_eq!(rig.engine.board_scheduler.due(now), vec![("a".to_string(), 1)]);
    }

    #[test]
    fn cancelled_and_stale_board_results_restore_old_text() {
        for kind in ["cancelled", "off", "stale"] {
            let mut rig = rig();
            let old = BoardText { recent: "old".into(), next: None, written_unix: 1 };
            rig.engine.board_entries.insert("a".into(), BoardEntry { text: Some(old.clone()), status: BoardStatus::Idle });
            let now = Instant::now();
            board_start(&mut rig, Arc::new(BoardFake::default()), vec![board_run("a")], crate::board::Choice::Luna, now);
            let reply = board_reply(&rig);
            let revision = rig.engine.board_flights["a"];
            rig.updates();
            if kind == "stale" {
                board_event(&mut rig.engine.sent.as_mut().unwrap()[0], "new event");
                rig.engine.observe_board(now);
                rig.engine.handle(reply);
            } else {
                rig.engine.board_done("a".into(), revision, Err(Failure::new(kind, "cancelled")), false, now);
            }
            let updates = rig.updates();
            assert!(updates.iter().any(|u| matches!(u, Update::Board { entry, .. } if entry.status == BoardStatus::Idle && entry.text == Some(old.clone()))), "{kind}");
        }
    }

    #[test]
    fn wake_chooses_the_earlier_deadline() {
        let now = Instant::now();
        let poll = now + Duration::from_secs(10);
        assert_eq!(next_wake(now, poll, Some(now + Duration::from_secs(3))), Duration::from_secs(3));
        assert_eq!(next_wake(now, poll, Some(now + Duration::from_secs(30))), Duration::from_secs(10));
        assert_eq!(next_wake(now, poll, None), Duration::from_secs(10));
    }

    #[test]
    fn board_settings_only_send_on_first_or_changed_value() {
        use crate::board::{Choice, Lang};
        let mut prior = None;
        assert!(board_settings_changed(&mut prior, (Choice::Luna, Lang::En)));
        assert!(!board_settings_changed(&mut prior, (Choice::Luna, Lang::En)));
        assert!(board_settings_changed(&mut prior, (Choice::Haiku, Lang::En)));
        assert!(board_settings_changed(&mut prior, (Choice::Haiku, Lang::ZhTw)));
    }

    #[test]
    fn board_revision_and_engine_gap_require_new_events() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake::default());
        let now = Instant::now();
        board_start(&mut rig, fake.clone(), vec![board_run("a")], crate::board::Choice::Luna, now);
        board_finish(&mut rig, now);
        rig.updates();
        rig.engine.observe_board(now + Duration::from_secs(119));
        assert!(rig._receiver.try_recv().is_err());
        board_event(&mut rig.engine.sent.as_mut().unwrap()[0], "new");
        rig.engine.observe_board(now + Duration::from_secs(119));
        assert!(rig._receiver.try_recv().is_err());
        assert!(rig.updates().is_empty());
        rig.engine.board_due(now + Duration::from_secs(120));
        board_finish(&mut rig, now + Duration::from_secs(120));
        assert_eq!(fake.luna.load(Ordering::SeqCst), 2);
        rig.updates();
        rig.engine.observe_board(now + Duration::from_secs(500));
        assert!(rig._receiver.try_recv().is_err());
        assert!(rig.updates().is_empty());
    }

    #[test]
    fn board_cache_hit_sends_entry_without_spend() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake::default());
        let now = Instant::now();
        board_start(&mut rig, fake.clone(), vec![board_run("a")], crate::board::Choice::Luna, now);
        board_finish(&mut rig, now);
        assert!(rig.updates().iter().any(|u| matches!(u, Update::BoardSpent(_))));
        // Session is intentionally excluded from the input cache key.
        rig.engine.sent = Some(vec![board_run("b")]);
        rig.engine.observe_board(now);
        board_finish(&mut rig, now);
        let updates = rig.updates();
        assert!(updates.iter().any(|u| matches!(u, Update::Board { session, entry } if session == "b" && entry.status == BoardStatus::Idle)));
        assert!(!updates.iter().any(|u| matches!(u, Update::BoardSpent(_))));
        assert_eq!(fake.luna.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn board_off_never_builds_and_reenable_is_immediate() {
        let mut rig = rig();
        let now = Instant::now();
        rig.engine.sent = Some(vec![board_run("a")]);
        rig.engine.observe_board(now);
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Board { entry, .. } if entry.status == BoardStatus::Off)));
        assert!(rig.engine.board_worker.is_none());
        assert!(!rig.engine.codex.created());
        let fake = Arc::new(BoardFake::default());
        rig.engine.board_model = Some(fake.clone());
        rig.engine.set_board(crate::board::Choice::Haiku, crate::board::Lang::En, now);
        board_finish(&mut rig, now);
        assert_eq!(fake.luna.load(Ordering::SeqCst), 0);
        assert_eq!(fake.haiku.load(Ordering::SeqCst), 1);
        rig.engine.set_board(crate::board::Choice::Off, crate::board::Lang::En, now);
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Board { entry, .. } if entry.status == BoardStatus::Off && entry.text.is_some())));
        assert!(!rig.engine.codex.created());
    }

    #[test]
    fn board_fallback_and_missing_usage_are_reported_once() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake { fail_luna: true, unknown_usage: true, ..Default::default() });
        let now = Instant::now();
        board_start(&mut rig, fake.clone(), vec![board_run("a")], crate::board::Choice::Luna, now);
        match board_reply(&rig) {
            Command::BoardDone { session, revision, result, real_call } => {
                let summary = result.as_ref().unwrap();
                assert_eq!(summary.provider, crate::board::Provider::Haiku);
                assert_eq!(summary.tokens, crate::board::TokenUse::default());
                rig.engine.board_done(session, revision, result, real_call, now);
            }
            _ => panic!("unexpected reply"),
        }
        assert_eq!(fake.luna.load(Ordering::SeqCst), 1);
        assert_eq!(fake.haiku.load(Ordering::SeqCst), 1);
        let mut state = state_with(vec![]);
        let spent = apply(&mut state, rig.updates());
        assert_eq!(spent, vec![crate::board::TokenUse::default()]);
        assert_eq!(state.board["a"].text.as_ref().unwrap().next, None);
    }

    #[test]
    fn board_failure_keeps_old_text_without_spend() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake { fail_luna: true, fail_haiku: true, ..Default::default() });
        let old = BoardText { recent: "old".into(), next: None, written_unix: 1 };
        rig.engine.board_entries.insert("a".into(), BoardEntry { text: Some(old.clone()), status: BoardStatus::Idle });
        let now = Instant::now();
        board_start(&mut rig, fake, vec![board_run("a")], crate::board::Choice::Luna, now);
        board_finish(&mut rig, now);
        let mut state = state_with(vec![]);
        assert!(apply(&mut state, rig.updates()).is_empty());
        assert_eq!(state.board["a"].text, Some(old));
        assert_eq!(state.board["a"].status, BoardStatus::Failed("model failed".into()));
    }

    #[test]
    fn board_stale_and_done_replies_are_dropped() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake::default());
        let now = Instant::now();
        board_start(&mut rig, fake, vec![board_run("a")], crate::board::Choice::Luna, now);
        let reply = board_reply(&rig);
        rig.updates();
        board_event(&mut rig.engine.sent.as_mut().unwrap()[0], "changed");
        rig.engine.observe_board(now);
        match reply {
            Command::BoardDone { session, revision, result, real_call } => rig.engine.board_done(session, revision, result, real_call, now),
            _ => panic!("unexpected reply"),
        }
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Board { entry, .. } if entry.status == BoardStatus::Idle)));
        rig.engine.board_due(now + Duration::from_secs(120));
        let reply = board_reply(&rig);
        rig.updates();
        rig.engine.sent.as_mut().unwrap()[0].status = RunStatus::Done;
        rig.engine.observe_board(now + Duration::from_secs(120));
        rig.engine.handle(reply);
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Board { entry, .. } if entry.status == BoardStatus::Idle)));
        assert!(rig.engine.board_known.is_empty());
    }

    #[test]
    fn board_jobs_are_serial_and_cancelled_old_reply_cannot_consume_new_flight() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake { delay: true, ..Default::default() });
        let now = Instant::now();
        let mut b = board_run("b"); board_event(&mut b, "different");
        board_start(&mut rig, fake.clone(), vec![board_run("a"), b], crate::board::Choice::Luna, now);
        let first = board_reply(&rig);
        let second = board_reply(&rig);
        rig.engine.handle(first); rig.engine.handle(second);
        assert_eq!(fake.maximum.load(Ordering::SeqCst), 1);
        assert_eq!(fake.luna.load(Ordering::SeqCst), 2);
        rig.engine.sent = Some(vec![board_run("a")]);
        rig.engine.set_board(crate::board::Choice::Off, crate::board::Lang::En, now);
        rig.engine.set_board(crate::board::Choice::Luna, crate::board::Lang::En, now);
        let new_revision = rig.engine.board_flights["a"];
        rig.engine.board_done("a".into(), new_revision.wrapping_sub(1), Err(Failure::new("cancelled", "old")), false, now);
        assert_eq!(rig.engine.board_flights.get("a"), Some(&new_revision));
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Writing);
        board_finish(&mut rig, now);
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Idle);
    }

    #[test]
    fn board_revision_tracks_facts_but_not_time_or_cost() {
        let mut run = board_run("a");
        let original = board_revision(&run, 0, 0);
        run.elapsed = Some("1 h".into());
        run.usage.total_usd = Some(42.0);
        assert_eq!(original, board_revision(&run, 0, 0));
        run.status = RunStatus::Blocked;
        assert_ne!(original, board_revision(&run, 0, 0));
        let changed = board_revision(&run, 0, 0);
        board_event(&mut run, "progress");
        assert_ne!(changed, board_revision(&run, 0, 0));
    }

    #[test]
    fn board_reappearing_session_does_not_accept_its_old_reply() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake { delay: true, ..Default::default() });
        let now = Instant::now();
        board_start(&mut rig, fake, vec![board_run("a")], crate::board::Choice::Luna, now);
        let old_revision = rig.engine.board_flights["a"];
        rig.engine.sent = Some(vec![]);
        rig.engine.observe_board(now);
        rig.engine.sent = Some(vec![board_run("a")]);
        rig.engine.observe_board(now);
        let new_revision = rig.engine.board_flights["a"];
        assert_ne!(old_revision, new_revision);
        let old_reply = board_reply(&rig);
        rig.engine.handle(old_reply);
        assert_eq!(rig.engine.board_flights.get("a"), Some(&new_revision));
        board_finish(&mut rig, now);
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Idle);
    }

    #[test]
    fn board_setting_changes_alone_do_not_start_another_job() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake::default());
        let now = Instant::now();
        board_start(&mut rig, fake.clone(), vec![board_run("a")], crate::board::Choice::Luna, now);
        board_finish(&mut rig, now);
        rig.updates();
        rig.engine.set_board(crate::board::Choice::Haiku, crate::board::Lang::ZhTw, now + Duration::from_secs(200));
        rig.engine.observe_board(now + Duration::from_secs(200));
        assert!(rig._receiver.try_recv().is_err());
        assert!(rig.updates().is_empty());
        board_event(&mut rig.engine.sent.as_mut().unwrap()[0], "new");
        rig.engine.observe_board(now + Duration::from_secs(200));
        board_finish(&mut rig, now + Duration::from_secs(200));
        assert_eq!(fake.luna.load(Ordering::SeqCst), 1);
        assert_eq!(fake.haiku.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn board_old_reply_after_off_does_not_finish_new_flight() {
        let mut rig = rig();
        let fake = Arc::new(BoardFake { delay: true, ..Default::default() });
        let now = Instant::now();
        board_start(&mut rig, fake, vec![board_run("a")], crate::board::Choice::Luna, now);
        rig.engine.set_board(crate::board::Choice::Off, crate::board::Lang::En, now);
        rig.engine.set_board(crate::board::Choice::Luna, crate::board::Lang::En, now);
        let revision = rig.engine.board_flights["a"];
        let old_reply = board_reply(&rig);
        rig.engine.handle(old_reply);
        assert_eq!(rig.engine.board_flights.get("a"), Some(&revision));
        board_finish(&mut rig, now);
        assert_eq!(rig.engine.board_entries["a"].status, BoardStatus::Idle);
    }

    #[test]
    fn board_settings_mapping_covers_choices_and_languages() {
        use super::super::{i18n::Lang, settings::BoardChoice};
        assert_eq!(map_board_settings(BoardChoice::Luna, Lang::ZhTw), (crate::board::Choice::Luna, crate::board::Lang::ZhTw));
        assert_eq!(map_board_settings(BoardChoice::Haiku, Lang::En), (crate::board::Choice::Haiku, crate::board::Lang::En));
        assert_eq!(map_board_settings(BoardChoice::Off, Lang::En), (crate::board::Choice::Off, crate::board::Lang::En));
    }

    const EVIDENCE: &str = "kind\tcommand\texit_code\tcommit\tts\n";
    const UNITS: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
    const MAP: &str = "# Map\n\n## m1\nPURPOSE    core things\nPATHS      src/a/\nDEPENDS    none\nCONTRACT   a stays stable\nACCEPT     tests/a.py `pytest tests/a.py`\n\n## m2\nPURPOSE    second\nPATHS      src/b/\nDEPENDS    m1\nCONTRACT   b\nACCEPT     tests/b.py `pytest tests/b.py`\n\nFULL TEST  `pytest`\n";

    #[test]
    fn live_backend_enqueues_evolve_only_for_explicit_commands() {
        let (commands, receiver) = mpsc::channel();
        let (_done, finished) = mpsc::channel();
        let backend = LiveBackend { commands, finished: Mutex::new(finished) };
        let repo = PathBuf::from("chosen-repo");

        Backend::evolve_check(&backend).unwrap();
        Backend::evolve_start(&backend, &repo).unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Command::EvolveCheck)));
        assert!(matches!(receiver.try_recv(), Ok(Command::EvolveStart(path)) if path == repo));
        assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));

        backend.start_wish(WishRequest {
            cwd: PathBuf::from("project"), wish: "wish".into(), creative: false,
            max_turns: None, advanced_research: false, council_budgets:None, codex_max_parallel:None,
        }).unwrap();
        backend.refresh().unwrap();
        backend.follow("session").unwrap();
        backend.recheck().unwrap();
        backend.interject("session", "note").unwrap();
        backend.stop("session").unwrap();
        backend.resume("session", None).unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Command::Start(_))));
        assert!(matches!(receiver.try_recv(), Ok(Command::Refresh)));
        assert!(matches!(receiver.try_recv(), Ok(Command::Follow(_))));
        assert!(matches!(receiver.try_recv(), Ok(Command::Recheck)));
        assert!(matches!(receiver.try_recv(), Ok(Command::Interject { .. })));
        assert!(matches!(receiver.try_recv(), Ok(Command::Stop(_))));
        assert!(matches!(receiver.try_recv(), Ok(Command::Resume { .. })));
        assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

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
    fn the_wish_is_shown_without_its_wrapper_markup_and_stays_as_stored() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path());
        let raw = "<pasted_content id=\"a0d5\"> 工作目錄在：[E:/x] 做完整測試 </pasted_content>";
        goal_file(dir.path(), "s1", json!({"wish":raw,"status":"active","project":"proj","cwd":"E:/codex/proj"}));
        let run = map_one(dir.path(), None, SystemTime::now());
        assert_eq!(run.wish.as_ref(), "工作目錄在：[E:/x] 做完整測試");
        let stored = std::fs::read_to_string(dir.path().join("goals").join("s1.json")).unwrap();
        assert!(stored.contains("pasted_content"), "the file on disk is untouched");
    }

    #[test]
    fn ledger_warnings_for_one_file_map_to_one_note() {
        let read = |instance: &str, reads, largest_bytes| Warning::LargeFileRead {
            agent_type: "chronos:reviewer".into(),
            instance: instance.into(),
            path: "p/schema.json".into(),
            reads,
            largest_bytes,
        };
        let notes = group_notes([read("a", 1, 10), read("b", 1, 30), read("c", 3, 20)].iter().map(note_of));
        assert_eq!(
            notes,
            vec![Note::FileReads { agent: "chronos:reviewer".into(), path: "p/schema.json".into(), reads: 5, largest: 30 }]
        );
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
        assert!(run.goals.is_empty(), "a store with no plan-final.md has no goals to show");

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
    fn summary_splits_cache_and_preserves_missing_and_zero() {
        let dir = tempfile::tempdir().unwrap();
        goal_file(dir.path(), "s1", base_goal("done"));
        log_with(dir.path(), "s1", "20261109-100000-wish.jsonl", &[
            json!({"type":"system","subtype":"init","session_id":"s1"}),
            json!({"type":"result","modelUsage":{
                "complete":{"inputTokens":0,"cacheReadInputTokens":3,"cacheCreationInputTokens":4,"outputTokens":5},
                "partial":{"inputTokens":0,"cacheReadInputTokens":0,"outputTokens":0}
            }})
        ]);
        let run = map_one(dir.path(), None, SystemTime::now());
        let m = |name: &str| run.usage.models.iter().find(|m| m.model.as_ref() == name).unwrap();
        assert_eq!((m("complete").input, m("complete").cached, m("complete").cache_write, m("complete").total), (Some(0), Some(3), Some(4), Some(12)));
        assert_eq!((m("partial").input, m("partial").cache_write, m("partial").total), (Some(0), None, None));
    }

    fn codex_events(session: &str, model: &str, input: u64, cached: u64, output: u64) -> Vec<Value> {
        vec![
            json!({"type":"system","subtype":"init","session_id":session,"cwd":"E:/project"}),
            json!({"type":"assistant","message":{"id":"a","model":"claude-sonnet-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":30,"cache_creation_input_tokens":20,"output_tokens":40},"content":[{"type":"tool_use","id":"c","name":"mcp__plugin_chronos_bridge__codex_run","input":{}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c","content":json!({"status":"completed","model":model,"usage":{"inputTokens":input,"cachedInputTokens":cached,"outputTokens":output,"reasoningOutputTokens":output/2,"totalTokens":input+output}}).to_string()}]}}),
        ]
    }

    #[test]
    fn actual_ledger_splits_claude_and_prices_codex_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let events = codex_events(session, "gpt-6.1-terra", 1_000_000, 400_000, 200_000);
        let contents: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let wish = Poller::new(&home).poll().wishes.remove(0);
        let usage = usage_of(&wish, Some(&ledger));
        let claude = usage.models.iter().find(|m| m.model.as_ref() == "claude-sonnet-5-5").unwrap();
        assert_eq!((claude.input, claude.cached, claude.cache_write, claude.output, claude.total), (Some(10), Some(30), Some(20), Some(40), Some(100)));
        let codex = usage.models.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        assert_eq!((codex.input, codex.cached, codex.cache_write, codex.output, codex.total), (Some(600_000), Some(400_000), None, Some(200_000), Some(1_200_000)));
        assert!((codex.usd.unwrap() - 1.62).abs() < 1e-9);
        let role = usage.roles.iter().find(|r| r.role.as_ref() == "codex gpt-6.1-terra").unwrap();
        assert_eq!(role.usd, codex.usd);
        assert!((usage.total_usd.unwrap() - ledger.totals.usd.unwrap() - 1.62).abs() < 1e-9);
    }

    #[test]
    fn full_terra_model_and_role_have_equal_nonzero_cost() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let contents: String = codex_events(session, "gpt-6.1-terra", 1_000_000, 400_000, 200_000)
            .iter().map(|e| format!("{e}\n")).collect();
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let wish = Poller::new(&home).poll().wishes.remove(0);
        let usage = usage_of(&wish, Some(&ledger));
        let model = usage.models.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap().usd.unwrap();
        let role = usage.roles.iter().find(|r| r.role.as_ref() == "codex gpt-6.1-terra").unwrap().usd.unwrap();
        assert!(model > 0.0 && role > 0.0);
        assert_eq!(model, role);
    }

    #[test]
    fn ledger_distinguishes_missing_claude_fields_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let events = [json!({"type":"assistant","message":{"id":"a","model":"claude-sonnet-5-5","usage":{"input_tokens":0,"cache_read_input_tokens":3,"output_tokens":4},"content":[]}})];
        let contents: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let wish = Poller::new(&home).poll().wishes.remove(0);
        let rows = claude_models(&wish, Some(&ledger));
        let model = rows.iter().find(|m| m.model.as_ref() == "claude-sonnet-5-5").unwrap();
        assert_eq!((model.input, model.cached, model.cache_write, model.output, model.total), (Some(0), Some(3), None, Some(4), None));
    }

    #[test]
    fn actual_scans_sum_models_across_runs_and_keep_unknown_unpriced() {
        let dir = tempfile::tempdir().unwrap();
        goal_file(dir.path(), "s1", base_goal("active"));
        log_with(dir.path(), "s1", "20261109-100000-wish.jsonl", &codex_events("s1", "gpt-6.1-terra", 100, 40, 20));
        let mut second = codex_events("s1", "gpt-6.1-terra", 200, 50, 30);
        let mut unknown = codex_events("s1", "gpt-unpriced", 10, 2, 4);
        second.append(&mut unknown);
        log_with(dir.path(), "s1", "20261109-110000-wish.jsonl", &second);
        let wish = Poller::new(dir.path()).poll().wishes.remove(0);
        assert_eq!(wish.runs.len(), 2);
        assert!(wish.runs.iter().all(|r| r.scan.as_ref().is_some_and(|s| s.complete)));
        let rows = codex_models(&wish, None);
        let priced = rows.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        assert_eq!((priced.input, priced.cached, priced.output, priced.total), (Some(210), Some(90), Some(50), Some(350)));
        let unknown = rows.iter().find(|m| m.model.as_ref() == "gpt-unpriced").unwrap();
        assert_eq!(unknown.usd, None);
        assert_eq!(Usage { models: rows, ..Default::default() }.unpriced(), vec![SharedString::from("gpt-unpriced")]);
    }

    #[test]
    fn codex_missing_fields_and_zero_survive_scan_and_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let mut events = codex_events(session, "gpt-6.1-terra", 0, 0, 0);
        let report = json!({"status":"completed","model":"gpt-6.1-terra","usage":{"inputTokens":0,"outputTokens":0}});
        events[2]["message"]["content"][0]["content"] = report.to_string().into();
        let contents: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&home.join("logs").join("20261109-100000-wish.jsonl"), &contents);
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let wish = Poller::new(&home).poll().wishes.remove(0);
        for rows in [codex_models(&wish, None), codex_models(&wish, Some(&ledger))] {
            let model = rows.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
            assert_eq!((model.input, model.cached, model.output, model.total, model.usd), (None, None, Some(0), Some(0), None));
        }
        let usage = usage_of(&wish, Some(&ledger));
        assert!(!usage.unpriced().iter().any(|m| m.as_ref() == "gpt-6.1-terra"));
        assert!(usage.incomplete().iter().any(|m| m.as_ref() == "gpt-6.1-terra"));
        assert_eq!(roles_of(&ledger).iter().find(|r| r.role.as_ref() == "codex gpt-6.1-terra").unwrap().usd, None);
        // Force the ledger path rather than the complete scan path.
        let mut incomplete = wish.clone();
        incomplete.runs[0].scan.as_mut().unwrap().complete = false;
        let rows = codex_models(&incomplete, Some(&ledger));
        let model = rows.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        assert_eq!((model.input, model.cached, model.output, model.total, model.usd), (None, None, Some(0), Some(0), None));
    }

    #[test]
    fn full_zero_codex_usage_prices_model_and_role_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let events = codex_events(session, "gpt-6.1-terra", 0, 0, 0);
        let contents: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let wish = Poller::new(&home).poll().wishes.remove(0);
        let usage = usage_of(&wish, Some(&ledger));
        assert_eq!(usage.models.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap().usd, Some(0.0));
        assert_eq!(usage.roles.iter().find(|r| r.role.as_ref() == "codex gpt-6.1-terra").unwrap().usd, Some(0.0));
    }

    #[test]
    fn codex_model_without_usage_is_retained_on_scan_and_ledger_paths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let projects = dir.path().join("projects");
        let session = "11111111-2222-3333-4444-555555555555";
        goal_file(&home, session, json!({"wish":"w","status":"active","cwd":"E:/project"}));
        let mut events = codex_events(session, "gpt-6.1-terra", 0, 0, 0);
        events[2]["message"]["content"][0]["content"] = json!({"model":"gpt-6.1-terra"}).to_string().into();
        let contents: String = events.iter().map(|e| format!("{e}\n")).collect();
        write(&home.join("logs").join("20261109-100000-wish.jsonl"), &contents);
        write(&projects.join("E--project").join(format!("{session}.jsonl")), &contents);
        let ledger = crate::usage::ledger(&crate::usage::Locations { home: home.clone(), projects }, Some(session)).unwrap();
        let mut wish = Poller::new(&home).poll().wishes.remove(0);
        let rows = codex_models(&wish, None);
        let model = rows.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        assert_eq!((model.input, model.total, model.usd), (None, None, None));
        wish.runs[0].scan.as_mut().unwrap().complete = false;
        let rows = codex_models(&wish, Some(&ledger));
        let model = rows.iter().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        assert_eq!((model.input, model.total, model.usd), (None, None, None));
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

    pub(super) fn state_with(runs: Vec<Run>) -> AppState {
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

    pub(super) struct Rig {
        pub(super) _dir: tempfile::TempDir,
        pub(super) engine: Engine,
        pub(super) inbox: Inbox,
        pub(super) _receiver: mpsc::Receiver<Command>,
    }

    pub(super) fn rig() -> Rig {
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
        pub(super) fn updates(&self) -> Vec<Update> {
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

    fn quota_window(kind: crate::usage::quota::WindowKind, used: f32, at: i64) -> crate::usage::quota::Window {
        crate::usage::quota::Window { kind, used, resets_at: None, observed_at: at, status: None }
    }

    fn event_quota(used: f32, at: i64) -> ProviderQuota {
        ProviderQuota { five_hour: Some(quota_window(crate::usage::quota::WindowKind::FiveHour, used, at)), weekly: None }
    }

    #[test]
    fn claude_event_deduplicates_and_rejects_older_windows() {
        let mut rig = rig();
        let mut key = "s".to_owned();
        rig.engine.hosted.insert(key.clone(), Hosted::new(false, String::new(), PathBuf::new()));
        rig.engine.on_event(&mut key, Event::RateLimit(event_quota(0.4, 20)));
        rig.engine.on_event(&mut key, Event::RateLimit(event_quota(0.4, 20)));
        rig.engine.on_event(&mut key, Event::RateLimit(event_quota(0.9, 19)));
        let updates = rig.updates();
        let quotas: Vec<_> = updates.iter().filter_map(|u| match u { Update::Quota(q) => Some(q), _ => None }).collect();
        assert_eq!(quotas.len(), 1);
        assert_eq!(quotas[0].claude.five_hour.as_ref().unwrap().used, 0.4);
        let mut state = state_with(vec![]);
        apply(&mut state, updates);
        assert_eq!(state.quota.claude.five_hour.unwrap().used, 0.4);
        assert!(state.feeds.quota.loaded && state.feeds.quota.error.is_none());
    }

    #[test]
    fn snapshot_and_live_claude_quotas_both_reach_ui() {
        let mut rig = rig();
        let home = rig.engine.home.clone();
        goal_file(&home, "s1", json!({"wish":"x", "status":"done", "amendments":[]}));
        let line = json!({"type":"rate_limit_event", "rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":0.2,"resetsAt":1,"status":"allowed"}}}});
        write(&home.join("logs/20261110-120000-wish-1.jsonl"), &format!("{}\n{}\n{}\n", json!({"type":"system","subtype":"init","session_id":"s1"}), json!({"type":"user","timestamp":"2026-11-10T13:17:08Z"}), line));
        rig.engine.poller.follow("s1");
        rig.engine.tick();
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Quota(q) if q.claude.weekly.as_ref().is_some_and(|w| w.used == 0.2))));
        let mut key = "s1".to_string();
        rig.engine.hosted.insert(key.clone(), Hosted::new(false, String::new(), PathBuf::new()));
        rig.engine.on_event(&mut key, Event::RateLimit(event_quota(0.7, 1_800_000_000)));
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Quota(q) if q.claude.five_hour.as_ref().is_some_and(|w| w.used == 0.7) && q.claude.weekly.is_some())));
    }

    struct FakeRateSource {
        generation: std::sync::atomic::AtomicU64,
        reads: std::sync::atomic::AtomicUsize,
        answers: Mutex<std::collections::VecDeque<Res<Value>>>,
        events: Mutex<Option<mpsc::Receiver<Value>>>,
    }
    impl quota::RateSource for FakeRateSource {
        fn read(&self) -> Res<Value> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.answers.lock().unwrap().pop_front().unwrap_or_else(|| Ok(json!({})))
        }
        fn generation(&self) -> u64 { self.generation.load(Ordering::SeqCst) }
        fn events(&self) -> Res<mpsc::Receiver<Value>> { Ok(self.events.lock().unwrap().take().unwrap()) }
    }
    fn fake_source(rig: &mut Rig, answers: Vec<Res<Value>>) -> (Arc<FakeRateSource>, mpsc::Sender<Value>) {
        let (tx, rx) = mpsc::channel();
        let source = Arc::new(FakeRateSource { generation: std::sync::atomic::AtomicU64::new(1), reads: std::sync::atomic::AtomicUsize::new(0), answers: Mutex::new(answers.into()), events: Mutex::new(Some(rx)) });
        rig.engine.rate_source = source.clone();
        (source, tx)
    }
    fn receive_quota_command(rig: &mut Rig) {
        let command = rig._receiver.recv_timeout(Duration::from_secs(5)).expect("quota helper did not answer");
        rig.engine.handle(command);
    }

    #[test]
    fn slots_refresh_only_when_the_existing_quota_read_is_due() {
        let mut rig = rig();
        let (_source, _events) = fake_source(&mut rig, vec![Err(Failure::new("test", "offline")), Err(Failure::new("test", "offline")), Err(Failure::new("test", "offline"))]);
        let folder = rig.engine.home.join("codex-slots");
        fs::create_dir_all(&folder).unwrap();
        let f = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(folder.join("0.lock")).unwrap();
        f.lock().unwrap();
        rig.engine.schedule_quota(false);
        assert!(!rig.updates().iter().any(|u| matches!(u, Update::CodexSlots(_))));
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        let mut state = state_with(vec![]);
        apply(&mut state, rig.updates());
        assert_eq!(state.codex_slots, Some(1));
        drop(f);
        rig.engine.schedule_quota(false);
        assert!(!rig.updates().iter().any(|u| matches!(u, Update::CodexSlots(_))));
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        apply(&mut state, rig.updates());
        assert_eq!(state.codex_slots, Some(0));
        fs::remove_file(folder.join("0.lock")).unwrap(); fs::remove_dir(&folder).unwrap(); fs::write(&folder, "file").unwrap();
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        apply(&mut state, rig.updates());
        assert_eq!(state.codex_slots, None);
        rig.engine.shutdown();
    }

    #[test]
    fn claude_merge_does_not_clear_codex_error_or_dedupe() {
        let mut rig = rig();
        let mut state = state_with(vec![]);
        let failed = || Err(Failure::new("bad", "read failed"));
        let (_source, _events) = fake_source(&mut rig, vec![failed(), failed()]);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        apply(&mut state, rig.updates());
        rig.engine.merge_quota(Provider::Claude, event_quota(0.4, 20));
        apply(&mut state, rig.updates());
        assert!(state.feeds.quota.loaded);
        assert_eq!(state.feeds.quota.error.as_ref().unwrap().0.as_ref(), "read failed");
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        assert!(!rig.updates().iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Quota, .. })));
        rig.engine.shutdown();
    }

    #[test]
    fn missing_codex_is_a_feed_error_not_an_action_failure() {
        let mut rig = rig();
        let missing = crate::base::resolve_command("AXIOM_TEST_MISSING_CODEX_CMD_7F19", "axiom-test-missing-codex-7f19");
        assert!(missing.is_err());
        *rig.engine.codex.resolve_override.lock().unwrap() = Some(missing);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        let updates = rig.updates();
        assert!(updates.iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Quota, error: Some(e) } if e.contains("axiom-test-missing-codex-7f19"))));
        assert!(!updates.iter().any(|u| matches!(u, Update::Failed { .. })));
        rig.engine.shutdown();
        assert!(rig.engine.codex.server().is_err());
    }

    #[test]
    fn events_ignore_login_notifications() {
        let mut rig = rig();
        let (_source, events) = fake_source(&mut rig, vec![]);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        events.send(json!({"method":"account/login/completed", "params":{"rateLimits":{"primary":{"usedPercent":90}}}})).unwrap();
        events.send(json!({"method":"account/rateLimits/updated", "params":{}})).unwrap();
        assert!(matches!(rig._receiver.recv_timeout(Duration::from_secs(5)).unwrap(), Command::CodexUpdate { params, .. } if params == json!({})));
        assert!(rig._receiver.try_recv().is_err());
        rig.engine.shutdown();
    }

    #[test]
    fn tick_with_active_run_starts_one_quota_read() {
        let mut rig = rig();
        let (source, _events) = fake_source(&mut rig, vec![]);
        rig.engine.hosted.insert("active".into(), Hosted::new(false, "wish".into(), PathBuf::new()));
        rig.engine.tick();
        assert!(rig.engine.sent.as_ref().unwrap().iter().any(|r| r.status == RunStatus::Active));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let cmd = rig._receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("quota read");
            if matches!(cmd, Command::CodexRead { .. }) { rig.engine.handle(cmd); break; }
            rig.engine.handle(cmd);
        }
        rig.engine.schedule_quota(false);
        assert_eq!(source.reads.load(Ordering::SeqCst), 1);
        assert!(!rig.engine.quota_busy);
        rig.engine.shutdown();
    }

    #[test]
    fn reload_commands_invalidate_or_trigger_expected_work() {
        let mut rig = rig();
        rig.engine.ledgers.insert("a".into(), LedgerEntry { ledger: None, activity: None, at: Instant::now() });
        assert!(!rig.engine.handle(Command::Reload(FeedKind::Usage)));
        assert!(rig.engine.ledgers.is_empty());
        assert!(!rig.engine.handle(Command::Reload(FeedKind::Checks)));
        assert!(matches!(rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap(), Command::Checks(_)));
        assert!(rig.engine.handle(Command::Reload(FeedKind::Runs)));
        let (_source, _events) = fake_source(&mut rig, vec![]);
        assert!(rig.engine.handle(Command::Reload(FeedKind::Quota)));
        receive_quota_command(&mut rig);
        rig.engine.shutdown();
    }

    #[test]
    fn backend_sends_reload_and_restore_memory_commands() {
        let (commands, receiver) = mpsc::channel();
        let (_done, finished) = mpsc::channel();
        let backend = LiveBackend { commands, finished: Mutex::new(finished) };
        let prior = MemoryPrior { body: "body".into(), summary: "summary".into(), corrected: Some(1.0), confirmed: 2.0 };
        backend.reload(FeedKind::Usage).unwrap();
        backend.restore_memory(42, &prior).unwrap();
        assert!(matches!(receiver.try_recv().unwrap(), Command::Reload(FeedKind::Usage)));
        assert!(matches!(receiver.try_recv().unwrap(), Command::RestoreMemory { id: 42, prior: p } if p.body == prior.body && p.summary == prior.summary && p.corrected == prior.corrected && p.confirmed == prior.confirmed));
    }

    #[test]
    fn generation_change_causes_exactly_one_extra_read() {
        let mut rig = rig();
        let (source, _events) = fake_source(&mut rig, vec![]);
        let mut run = sample("active", &[]);
        run.status = RunStatus::Active;
        rig.engine.sent = Some(vec![run]);
        rig.engine.schedule_quota(false);
        receive_quota_command(&mut rig);
        assert_eq!(source.reads.load(Ordering::SeqCst), 1);
        rig.engine.schedule_quota(false);
        assert!(!rig.engine.quota_busy);
        source.generation.store(2, Ordering::SeqCst);
        rig.engine.schedule_quota(false);
        receive_quota_command(&mut rig);
        rig.engine.schedule_quota(false);
        assert!(!rig.engine.quota_busy);
        assert_eq!(source.reads.load(Ordering::SeqCst), 2);
        rig.engine.shutdown();
    }

    #[test]
    fn reload_during_read_runs_one_more_read_without_overlap() {
        let mut rig = rig();
        let (source, _events) = fake_source(&mut rig, vec![]);
        rig.engine.schedule_quota(true);
        assert!(rig.engine.quota_busy);
        rig.engine.handle(Command::Reload(FeedKind::Quota));
        rig.engine.handle(Command::Reload(FeedKind::Quota));
        assert!(rig.engine.quota_reload_pending);
        receive_quota_command(&mut rig);
        assert!(rig.engine.quota_busy);
        receive_quota_command(&mut rig);
        assert!(!rig.engine.quota_busy);
        assert_eq!(source.reads.load(Ordering::SeqCst), 2);
        rig.engine.shutdown();
        assert!(rig.engine.quota_stop.load(Ordering::Acquire));
    }

    #[test]
    fn failed_read_preserves_windows_and_next_success_clears_feed_error() {
        let mut rig = rig();
        let good = json!({"rateLimits":{"primary":{"windowDurationMins":300,"usedPercent":30}}});
        let (_source, _events) = fake_source(&mut rig, vec![Ok(good.clone()), Err(Failure::new("bad", "read failed")), Ok(good)]);
        let mut state = state_with(vec![]);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        apply(&mut state, rig.updates());
        let first = state.quota.clone();
        assert_eq!(first.codex.five_hour.as_ref().unwrap().used, 0.3);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        let updates = rig.updates();
        assert!(!updates.iter().any(|u| matches!(u, Update::Failed { .. })));
        apply(&mut state, updates);
        assert_eq!(state.quota, first);
        assert_eq!(state.feeds.quota.error.as_ref().unwrap().0.as_ref(), "read failed");
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        apply(&mut state, rig.updates());
        assert!(state.feeds.quota.error.is_none());
        assert_eq!(state.quota.codex.five_hour.as_ref().unwrap().used, 0.3);
        rig.engine.shutdown();
    }

    #[test]
    fn codex_is_not_created_until_a_read_is_due() {
        let mut rig = rig();
        assert!(!rig.engine.codex.created());
        rig.engine.tick();
        assert!(!rig.engine.codex.created());
        let mut key = "s".to_owned();
        rig.engine.hosted.insert(key.clone(), Hosted::new(false, String::new(), PathBuf::new()));
        rig.engine.on_event(&mut key, Event::RateLimit(event_quota(0.5, 10)));
        assert!(!rig.engine.codex.created());
    }

    #[test]
    fn sparse_account_notification_updates_only_its_window() {
        let mut rig = rig();
        let (_source, events) = fake_source(&mut rig, vec![Ok(json!({"rateLimits":{"secondary":{"windowDurationMins":10080,"usedPercent":60}}}))]);
        rig.engine.schedule_quota(true);
        receive_quota_command(&mut rig);
        let weekly = rig.engine.quotas.codex.weekly.clone();
        assert!(weekly.is_some());
        events.send(json!({"jsonrpc":"2.0","method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"windowDurationMins":300,"usedPercent":25}}}})).unwrap();
        receive_quota_command(&mut rig);
        assert_eq!(rig.engine.quotas.codex.weekly, weekly);
        assert_eq!(rig.engine.quotas.codex.five_hour.as_ref().unwrap().used, 0.25);
        assert!(rig.updates().iter().any(|u| matches!(u, Update::Quota(q) if q.codex.weekly == weekly && q.codex.five_hour.as_ref().is_some_and(|w| w.used == 0.25))));
        rig.engine.shutdown();
    }

    #[test]
    fn evolve_preview_limits_issues_and_preserves_details() {
        let problems = (0..12).map(|i| digest::Problem {
            id: format!("issue-{i}"), title: format!("Issue {i}"), family: "test".into(),
            count: i, cost_usd: Some(i as f64), unpriced: 0, snapshot: false,
            metric: digest::Metric { value: 0.0, samples: 1 }, sources: vec![],
        }).collect();
        let digest = Digest {
            now_ms: 10, since_days: 7, start_ms: 0, problems,
            gaps: vec![evolve::Gap { family: "f".into(), location: "here".into(), reason: "missing".into() }],
            sources: vec![], cost: vec![], comparisons: vec![],
            estimate: digest::Estimate { median_usd: None, samples: 3, unpriced_runs: 1, method: "median".into() },
        };
        let repo = PathBuf::from("repo");
        let preview = evolve_preview(&digest, Some(repo.clone()));
        assert_eq!(preview.issues.len(), 10);
        assert_eq!(preview.issues[0].id.as_ref(), "issue-0");
        assert_eq!(preview.issues[9].id.as_ref(), "issue-9");
        assert_eq!(preview.more_issues, 2);
        assert_eq!((preview.estimate_usd, preview.estimate_samples), (None, 3));
        assert_eq!(preview.gaps[0], EvolveGap { family: "f".into(), location: "here".into(), reason: "missing".into() });
        assert_eq!(preview.repo, Some(repo));
        assert_eq!(preview.text.as_ref(), digest::render(&digest));
    }

    #[test]
    fn evolve_check_is_read_only_and_populates_state() {
        let mut rig = rig();
        let record = evolve::history::RunRecord {
            id: "old-run".into(), started_ms: 42, repo: "repo".into(), base_commit: "abc".into(),
            branch: "evolve/old".into(), since_days: 7, start_ms: 0, end_ms: 42,
            problems: vec![], metrics: vec![], state: "ended".into(), session: None,
            goal_status: None, exit_code: Some(0), error: None,
        };
        evolve::history::save(rig._dir.path(), &record).unwrap();
        fn listing(path: &Path, root: &Path, entries: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                entries.push(path.strip_prefix(root).unwrap().to_path_buf());
                if path.is_dir() { listing(&path, root, entries); }
            }
        }
        let mut before = vec![];
        listing(rig._dir.path(), rig._dir.path(), &mut before);
        before.sort();
        let mut state = state_with(vec![]);
        state.evolve.checking = true;
        rig.engine.handle(Command::EvolveCheck);
        let reply = rig._receiver.recv_timeout(Duration::from_secs(10)).expect("check completes");
        rig.engine.handle(reply);
        apply(&mut state, rig.updates());
        assert!(!state.evolve.checking);
        assert!(state.evolve.error.is_none(), "{:?}", state.evolve.error);
        assert!(state.evolve.preview.is_some());
        assert_eq!(state.evolve.runs.len(), 1);
        assert_eq!(state.evolve.runs[0].id.as_ref(), "old-run");
        assert_eq!(state.evolve.runs[0].verdict, Verdict::Unknown);
        let mut after = vec![];
        listing(rig._dir.path(), rig._dir.path(), &mut after);
        after.sort();
        assert_eq!(before, after);
    }

    #[test]
    fn evolve_start_rejects_non_repo_and_guards_duplicate() {
        let mut rig = rig();
        let mut state = state_with(vec![]);
        state.evolve.starting = true;
        rig.engine.handle(Command::EvolveStart(rig._dir.path().to_path_buf()));
        rig.engine.handle(Command::EvolveStart(rig._dir.path().to_path_buf()));
        apply(&mut state, rig.updates());
        assert!(state.evolve.starting);
        assert_eq!(state.evolve.error.as_ref().unwrap().kind.as_ref(), "busy");
        state.evolve.error = None;
        let reply = rig._receiver.recv_timeout(Duration::from_secs(10)).expect("start completes");
        rig.engine.handle(reply);
        apply(&mut state, rig.updates());
        assert!(!state.evolve.starting);
        assert_eq!(state.evolve.error.as_ref().unwrap().kind.as_ref(), "not_git_repo");
        assert!(state.evolve.outcome.is_none());
    }

    #[test]
    fn evolve_start_panic_clears_starting_and_allows_retry() {
        let mut rig = rig();
        rig.engine.evolve_job = Arc::new(|_, _, _| panic!("injected evolve panic"));
        let mut state = state_with(vec![]);
        state.evolve.starting = true;
        let repo = rig._dir.path().to_path_buf();
        rig.engine.handle(Command::EvolveStart(repo.clone()));
        let reply = rig._receiver.recv_timeout(Duration::from_secs(3)).expect("panicked start replies");
        rig.engine.handle(reply);
        apply(&mut state, rig.updates());
        assert!(!state.evolve.starting);
        assert_eq!(state.evolve.error.as_ref().unwrap().kind.as_ref(), "engine_stopped");

        rig.engine.evolve_job = Arc::new(|_, _, _| Err(Failure::new("test_reply", "retry reached job")));
        state.evolve.starting = true;
        state.evolve.error = None;
        rig.engine.handle(Command::EvolveStart(repo));
        apply(&mut state, rig.updates());
        assert!(state.evolve.error.is_none(), "retry must not be busy");
        let reply = rig._receiver.recv_timeout(Duration::from_secs(3)).expect("retry replies");
        rig.engine.handle(reply);
        apply(&mut state, rig.updates());
        assert!(!state.evolve.starting);
        assert_eq!(state.evolve.error.as_ref().unwrap().kind.as_ref(), "test_reply");
    }

    #[test]
    fn a_wish_that_cannot_start_shows_the_error_and_leaves_no_run_behind() {
        let mut rig = rig();
        rig.engine.handle(Command::Start(WishRequest {
            cwd: rig._dir.path().join("missing"),
            wish: "x".into(),
            creative: false,
            max_turns: None,
            advanced_research: false, council_budgets:None, codex_max_parallel:None,
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
    fn a_failed_start_keeps_the_wish_exactly_as_typed() {
        let mut rig = rig();
        let wish = "<attachment>fix <my-widget></attachment> now";
        rig.engine.handle(Command::Start(WishRequest {
            cwd: rig._dir.path().join("missing"),
            wish: wish.into(),
            creative: false,
            max_turns: None,
            advanced_research: false, council_budgets:None, codex_max_parallel:None,
        }));
        let message = rig
            .updates()
            .into_iter()
            .find_map(|u| match u {
                Update::Failed { action: Action::StartWish, message, .. } => Some(message),
                _ => None,
            })
            .expect("the failure is reported");
        assert!(message.ends_with(&format!("Your wish: {wish}")), "{message}");
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

    pub(super) fn memory_entries(home: &Path) -> (i64, i64) {
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
        let untouched = |engine: &Engine, id: i64| engine.memory.iter().find(|m| m.id == id).unwrap().clone();
        let before = untouched(&rig.engine, first);
        assert_eq!(before.wish.as_ref(), "wish", "the wish it came from");
        assert!(!before.confirmed.is_empty(), "an entry never confirmed again was confirmed when recorded");
        assert!(!before.corrected);

        rig.engine.handle(Command::CorrectMemory(first, "the right text".into()));
        assert!(untouched(&rig.engine, first).corrected);
        assert!(!untouched(&rig.engine, second).corrected, "only the corrected entry is marked");
        let reread = read_memory(&home).unwrap();
        let after = reread.iter().find(|m| m.id == first).unwrap();
        assert!(after.corrected, "the store records the correction");
        assert_ne!(after.confirmed, before.confirmed, "a correction is a fresh confirmation");
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

    fn module(id: &str) -> Module {
        Module { id: id.into(), purpose: "p".into(), depends: vec![], units: vec![], contract: None }
    }

    fn goal(id: &str, title: &str, modules: &[&str]) -> app::GoalView {
        app::GoalView { id: id.into(), title: title.into(), modules: modules.iter().map(|m| (*m).into()).collect() }
    }

    #[test]
    fn goals_point_at_the_modules_that_deliver_them() {
        let modules = [module("m1"), module("m2"), module("loose")];
        let goals = goal_items(&[goal("G1", "Render pages", &["m2", "m1"]), goal("G2", "Docs", &[])], &modules);
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].id.as_ref(), "G1");
        assert_eq!(goals[0].title.as_ref(), "Render pages");
        assert_eq!(goals[0].modules, vec![1, 0], "indices into the run's modules, in the plan's order");
        assert!(goals[1].modules.is_empty(), "a goal no module serves stays listed and unserved");
    }

    #[test]
    fn a_module_the_run_does_not_have_is_dropped_not_invented() {
        let goals = goal_items(&[goal("G1", "x", &["m1", "ghost"])], &[module("m1")]);
        assert_eq!(goals[0].modules, vec![0]);
        assert!(goal_items(&[], &[module("m1")]).is_empty(), "no plan items, no goals");
    }

    #[test]
    fn a_stored_plan_reaches_the_run_as_goals() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write(
            &store.join("modules.md"),
            &MAP.replace("PURPOSE    core things", "PURPOSE    core things
SERVES     G1")
                .replace("PURPOSE    second", "PURPOSE    second
SERVES     G2"),
        );
        write(&store.join("astra/plan-final.md"), "# Plan

### G1 Render every page

### G2 Split the table
");
        goal_file(dir.path(), "s1", base_goal("active"));
        let run = map_one(dir.path(), None, SystemTime::now());
        let shown: Vec<(&str, &str, Vec<usize>)> =
            run.goals.iter().map(|g| (g.id.as_ref(), g.title.as_ref(), g.modules.clone())).collect();
        assert_eq!(shown, [("G1", "Render every page", vec![0]), ("G2", "Split the table", vec![1])]);
    }

    /// A worker with a real thread whose engine owns a sentinel that reports when it is dropped.
    struct DropFlag(mpsc::Sender<&'static str>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            let _ = self.0.send("engine dropped");
        }
    }

    #[test]
    fn shutdown_reaches_the_worker_and_waits_until_it_has_dropped_what_it_owned() {
        let dir = tempfile::tempdir().unwrap();
        let (home, projects) = (dir.path().to_path_buf(), dir.path().join("projects"));
        let (dropped, seen) = mpsc::channel();
        let flag = DropFlag(dropped);
        let config_home = home.clone();
        let config = Box::new(move || {
            let _ = &flag;
            Config {
                claude: Err(Failure::new("not_installed", "no claude in tests")),
                home: config_home.clone(),
                bin: PathBuf::new(),
                clock: session::Clock::system(),
            }
        });
        let (commands, receiver) = mpsc::channel();
        let inbox = Inbox::default();
        let engine = Engine::new(home, projects, config, commands.clone(), inbox.clone());
        let backend = launch(engine, commands, receiver);
        backend.shutdown(Duration::from_secs(30));
        assert_eq!(
            seen.try_recv(),
            Ok("engine dropped"),
            "shutdown returned before the worker had dropped its engine and hosted wishes"
        );
        assert!(inbox.lock().closed, "the screen is told the worker is gone");
        backend.shutdown(Duration::from_secs(30));
        assert!(backend.refresh().is_err(), "nothing listens any more");
    }

    #[test]
    fn a_stale_source_is_noticed_while_the_database_is_unchanged() {
        use sha2::{Digest, Sha256};
        let mut rig = rig();
        let home = rig.engine.home.clone();
        let project = rig._dir.path().join("src-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("a.txt"), "one").unwrap();
        Memory::open(&home).unwrap();
        let db = rusqlite::Connection::open(home.join("memory/memory.db")).unwrap();
        let files = json!([{"path":"a.txt","sha256":format!("{:x}", Sha256::digest(b"one"))}]).to_string();
        db.execute(
            "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('proj', 'u1', 't', 's', 'b', '', 'w', ?, ?, 1000.0)",
            [files, project.display().to_string()],
        )
        .unwrap();
        drop(db);
        let listed = |rig: &mut Rig| {
            rig.engine.schedule_memory();
            match rig._receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(Command::Memory(result)) => {
                    rig.engine.memory_busy = false;
                    result.unwrap()
                }
                _ => panic!("the memory list was not read"),
            }
        };
        assert!(!listed(&mut rig)[0].stale);
        let db_stamp = fs::metadata(home.join("memory/memory.db")).unwrap().modified().unwrap();
        fs::write(project.join("a.txt"), "changed").unwrap();
        assert_eq!(fs::metadata(home.join("memory/memory.db")).unwrap().modified().unwrap(), db_stamp, "the database is untouched");

        rig.engine.schedule_memory();
        assert!(!rig.engine.memory_busy, "not read again on every poll");
        rig.engine.memory_read = Some(Instant::now() - MEMORY_EVERY - Duration::from_secs(1));
        assert!(listed(&mut rig)[0].stale, "the changed source is noticed on the next cadence");
    }

    #[test]
    fn memory_times_read_as_dates_and_unknown_times_stay_blank() {
        assert!(confirmed_text(1_793_767_800.0).contains(", "), "{}", confirmed_text(1_793_767_800.0));
        assert_eq!(confirmed_text(0.0), "");
        assert_eq!(confirmed_text(f64::NAN), "");
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
