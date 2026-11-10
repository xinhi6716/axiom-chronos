//! What the screens draw and the commands they send. Screens never read `~/.axiom` or start processes.
use super::i18n::{translate, Lang, Table};
use crate::base::Res;
use gpui_kit::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Active,
    Waiting,
    /// Claude hit its usage limit; the wish resumes by itself. Not the same as waiting on a batch.
    LimitWait,
    /// The goal is still open but the process behind it has gone.
    Interrupted,
    Blocked,
    Done,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Running",
            Self::Waiting => "Waiting on batch",
            Self::LimitWait => "Waiting on usage limit",
            Self::Interrupted => "Stopped unexpectedly",
            Self::Blocked => "Needs you",
            Self::Done => "Done",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    Pending,
    Running,
    Built,
    Verified,
    Failed,
}

impl UnitState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Built => "Awaiting review",
            Self::Verified => "Verified",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub name: SharedString,
    pub phase: Phase,
    pub note: Option<SharedString>,
}

/// Text shown in the detail panel: a file on disk, or text carried in the model.
#[derive(Clone, Debug, PartialEq)]
pub enum TextSource {
    Path(PathBuf),
    Inline(SharedString),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitFiles {
    pub brief: Option<TextSource>,
    pub report: Option<TextSource>,
    pub verify: Option<TextSource>,
    pub review: Option<TextSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub id: SharedString,
    /// The GOAL line of the unit's brief, or the id when there is none.
    pub title: SharedString,
    pub module: usize,
    pub state: UnitState,
    /// The state word exactly as stored, shown in the detail panel.
    pub raw_state: SharedString,
    pub attempts: u32,
    pub files: UnitFiles,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub id: SharedString,
    pub purpose: SharedString,
    pub depends: Vec<usize>,
    pub units: Vec<usize>,
    pub contract: Option<TextSource>,
}

/// A numbered item of the plan, and the modules that deliver it.
#[derive(Clone, Debug, PartialEq)]
pub struct GoalItem {
    pub id: SharedString,
    pub title: SharedString,
    pub modules: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeedKind {
    Blocked,
    Waiting,
    Interjection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Need {
    pub kind: NeedKind,
    pub title: SharedString,
    pub note: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelCost {
    pub model: SharedString,
    /// `None` when the run has not reported a figure.
    pub usd: Option<f64>,
    /// Each count is `None` when the run did not report it.
    pub input: Option<u64>,
    pub cached: Option<u64>,
    pub output: Option<u64>,
}

/// Spend attributed to one role. Left empty when a run has no per-role record.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleCost {
    pub role: SharedString,
    /// `None` when the run has not reported a figure.
    pub usd: Option<f64>,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentContext {
    pub agent: SharedString,
    pub peak_tokens: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub total_usd: Option<f64>,
    pub models: Vec<ModelCost>,
    pub roles: Vec<RoleCost>,
    pub contexts: Vec<AgentContext>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Ok,
    Warn,
    Error,
}

/// The source of a log line the user wrote. Screens translate it.
pub const YOU: &str = "You";

#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub time: SharedString,
    pub source: SharedString,
    pub text: SharedString,
    pub level: LogLevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub session: SharedString,
    pub project: SharedString,
    pub cwd: Option<PathBuf>,
    pub wish: SharedString,
    pub status: RunStatus,
    /// Started from this app, so it can be interjected or stopped from here.
    pub hosted: bool,
    pub started: SharedString,
    /// Unix seconds, for ordering runs; `started` is display text with no year.
    pub started_at: Option<u64>,
    pub stages: Vec<Stage>,
    pub goals: Vec<GoalItem>,
    pub modules: Vec<Module>,
    pub units: Vec<Unit>,
    pub usage: Usage,
    pub needs: Vec<Need>,
    pub log: Vec<LogLine>,
    /// Choices Chronos made on the user's behalf.
    pub decisions: Vec<SharedString>,
    pub report: Option<TextSource>,
    pub elapsed: Option<SharedString>,
    /// Every warning to show, as text in the language of the day. It includes the text of `notes`.
    pub warnings: Vec<SharedString>,
    /// The warnings the usage ledger raised, grouped and still structured, so a screen can
    /// translate them or leave out the ones it already shows another way.
    pub notes: Vec<Note>,
}

impl Run {
    /// Warnings that are not the text of a ledger note.
    pub fn other_warnings(&self) -> impl Iterator<Item = &SharedString> {
        let texts: Vec<String> = self.notes.iter().flat_map(|n| [n.text(Lang::En), n.text(Lang::ZhTw)]).collect();
        self.warnings.iter().filter(move |w| !texts.iter().any(|t| t == w.as_ref()))
    }

    pub fn count(&self, state: UnitState) -> usize {
        self.units.iter().filter(|u| u.state == state).count()
    }

    pub fn module_done(&self, module: &Module) -> usize {
        module
            .units
            .iter()
            .filter(|&&u| self.units[u].state == UnitState::Verified)
            .count()
    }

    /// True while something is moving, so screens animate only when there is something to show.
    pub fn is_live(&self) -> bool {
        self.status == RunStatus::Active && self.units.iter().any(|u| u.state == UnitState::Running)
    }

    /// The run ended with every unit verified and the full test passed. A goal closed as done
    /// by a model session is not enough.
    pub fn verified_end(&self) -> bool {
        self.status == RunStatus::Done
            && !self.units.is_empty()
            && self.count(UnitState::Verified) == self.units.len()
            && self.stages.iter().any(|s| s.name.eq_ignore_ascii_case("Full test") && s.phase == Phase::Done)
    }

    /// The status word to show: an ending that was not verified is never called done.
    pub fn status_label(&self) -> &'static str {
        if self.status == RunStatus::Done && !self.verified_end() {
            "Ended, needs attention"
        } else {
            self.status.label()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryEntry {
    pub id: i64,
    pub project: SharedString,
    pub title: SharedString,
    pub body: SharedString,
    /// True only when the files it was learned from are known to have changed.
    pub stale: bool,
    /// The wish it was learned from, in the user's words. Empty when none was recorded.
    pub wish: SharedString,
    /// When it was last confirmed, as `Run::started` is written. Empty when not known.
    pub confirmed: SharedString,
    /// A person rewrote the text, so it is no longer what the engine first wrote.
    pub corrected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: SharedString,
    pub status: CheckStatus,
    pub detail: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Run,
    Wishes,
    Memory,
    Usage,
    Doctor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    Module(usize),
    Unit(usize),
}

/// A request to make a new wish.
#[derive(Clone, Debug)]
pub struct WishRequest {
    pub cwd: PathBuf,
    pub wish: String,
    pub creative: bool,
    /// `None` leaves the turn count uncapped, as `axiom wish` does without `--max-turns`.
    pub max_turns: Option<u32>,
    /// The council of Astra, Fable 5.2 and Argon, as `axiom wish --advanced-research`. It costs
    /// real money, so it is on only when the user turned it on for this wish.
    pub advanced_research: bool,
}

/// What the app was doing when something went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    StartWish,
    Interject,
    Stop,
    Resume,
    DeleteMemory,
    CorrectMemory,
    Refresh,
    Recheck,
    Session,
    ReadData,
    Engine,
}

impl Action {
    /// What happened and what it affects.
    pub fn headline(self) -> &'static str {
        match self {
            Self::StartWish => "The wish could not start",
            Self::Interject => "Your message was not delivered",
            Self::Stop => "The wish was not stopped",
            Self::Resume => "The wish did not resume",
            Self::DeleteMemory => "The memory was not deleted",
            Self::CorrectMemory => "The correction was not saved",
            Self::Refresh => "The runs could not be refreshed",
            Self::Recheck => "The environment could not be checked",
            Self::Session => "A wish stopped with an error",
            Self::ReadData => "Some saved data could not be read",
            Self::Engine => "The connection to the engine stopped",
        }
    }

    /// What to do next.
    pub fn advice(self) -> &'static str {
        match self {
            Self::StartWish => "Nothing was changed. Check Settings, then try again.",
            Self::Interject => "The run did not receive it. Check that it is still going, then send it again.",
            Self::Stop => "It may still be working. Try again, or stop it where it was started.",
            Self::Resume => "It is still stopped. Read the details, then try again.",
            Self::DeleteMemory => "It is still stored. Try again.",
            Self::CorrectMemory => "The old text is still in use. Try again.",
            Self::Refresh => "What you see may be out of date.",
            Self::Recheck => "The results shown may be out of date.",
            Self::Session => "Open it to see how far it got. It can be resumed.",
            Self::ReadData => "The rest of the app keeps working; the affected panel may be incomplete.",
            Self::Engine => "Nothing will update any more. Restart Chronos.",
        }
    }
}

/// A command or background read that failed, kept until the screens have shown it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppError {
    pub id: u64,
    pub action: Action,
    pub session: Option<SharedString>,
    /// The raw message, offered behind Copy.
    pub message: SharedString,
}

/// How many failures are kept for the screens.
const ERRORS_KEPT: usize = 50;

/// Commands a screen can send. They return as soon as the command is queued, so the interface
/// never waits on the engine: `Err` means it could not be queued, and a failure that happens
/// later arrives in `AppState::errors`.
pub trait Backend: 'static {
    fn start_wish(&self, request: WishRequest) -> Res<()>;
    fn interject(&self, session: &str, text: &str) -> Res<()>;
    fn stop(&self, session: &str) -> Res<()>;
    fn resume(&self, session: &str, note: Option<&str>) -> Res<()>;
    fn delete_memory(&self, id: i64) -> Res<()>;
    fn correct_memory(&self, id: i64, text: &str) -> Res<()>;
    /// Read the runs from disk now instead of at the next poll.
    fn refresh(&self) -> Res<()>;
    /// Run the environment checks again.
    fn recheck(&self) -> Res<()>;
    /// The user is looking at this run: read its logs and usage in full.
    fn follow(&self, session: &str) -> Res<()>;
    /// The window is closing: end the engine, which stops every wish this app hosts, and wait at
    /// most `wait` for that to finish. Safe to call more than once.
    fn shutdown(&self, _wait: Duration) {}
}

/// Words to put in the interjection box of one run, naming what the person is talking about.
#[derive(Clone, Debug, PartialEq)]
pub struct Address {
    pub session: SharedString,
    pub text: SharedString,
}

/// The single shared model. Screens read it and call its command methods.
pub struct AppState {
    pub view: Screen,
    pub runs: Vec<Run>,
    pub current: Option<usize>,
    pub selection: Selection,
    pub memory: Vec<MemoryEntry>,
    pub checks: Vec<Check>,
    pub reduce_motion: bool,
    pub composing: bool,
    pub backend: Option<Box<dyn Backend>>,
    /// Demo only: a scene name that a screen acts on at startup, so states can be captured.
    pub scene: Option<String>,
    /// Newest last. Ids only grow, so a screen can tell which ones it has already shown.
    pub errors: Vec<AppError>,
    pub next_error: u64,
    /// A run's real session id to the provisional id it had before `Event::Started`.
    pub renames: HashMap<SharedString, SharedString>,
    /// Set by the details panel; the control bar takes it and opens its box with the text in it.
    pub address: Option<Address>,
}

/// The run to open when none is selected: live work first, otherwise the newest.
fn default_run(runs: &[Run]) -> Option<usize> {
    let live = |run: &Run| matches!(run.status, RunStatus::Active | RunStatus::LimitWait);
    runs.iter()
        .position(|run| live(run) && run.hosted)
        .or_else(|| runs.iter().position(live))
        .or((!runs.is_empty()).then_some(0))
}

impl AppState {
    pub fn run(&self) -> Option<&Run> {
        self.current.and_then(|i| self.runs.get(i))
    }

    pub fn set_view(&mut self, view: Screen, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub fn open_run(&mut self, index: usize, cx: &mut Context<Self>) {
        self.current = Some(index);
        self.selection = Selection::None;
        self.view = Screen::Run;
        self.follow_current();
        cx.notify();
    }

    /// Tells the engine which run is on screen, so its logs and usage are read in full.
    pub fn follow_current(&mut self) {
        let Some(session) = self.run().map(|run| run.session.clone()) else { return };
        self.command(Action::Refresh, None, |backend| backend.follow(&session));
    }

    /// Takes a fresh list of runs. The poller re-sorts runs and units may be added, so the
    /// open run and its selection are carried over by session and unit id, never by position.
    pub fn replace_runs(&mut self, runs: Vec<Run>, renamed: &[(String, String)], open: Option<&str>) {
        let current_id = |session: &SharedString| -> SharedString {
            renamed
                .iter()
                .find(|(old, _)| old.as_str() == session.as_ref())
                .map_or_else(|| session.clone(), |(_, new)| new.clone().into())
        };
        let viewing = self.run().map(|run| current_id(&run.session));
        enum Anchor {
            Unit(SharedString),
            Module(SharedString),
        }
        let anchor = self.run().and_then(|run| match self.selection {
            Selection::None => None,
            Selection::Unit(i) => run.units.get(i).map(|unit| Anchor::Unit(unit.id.clone())),
            Selection::Module(i) => run.modules.get(i).map(|module| Anchor::Module(module.id.clone())),
        });
        for (old, new) in renamed {
            self.renames.insert(new.clone().into(), old.clone().into());
        }
        self.runs = runs;
        let target: Option<SharedString> = open.map(|s| s.to_owned().into()).or_else(|| viewing.clone());
        let same_run = target.is_some() && target == viewing;
        let found = target.and_then(|session| self.runs.iter().position(|run| run.session == session));
        self.current = found.or_else(|| default_run(&self.runs));
        let run = found.filter(|_| same_run).and_then(|i| self.runs.get(i));
        self.selection = match (run, anchor) {
            (Some(run), Some(Anchor::Unit(id))) => {
                run.units.iter().position(|unit| unit.id == id).map_or(Selection::None, Selection::Unit)
            }
            (Some(run), Some(Anchor::Module(id))) => {
                run.modules.iter().position(|module| module.id == id).map_or(Selection::None, Selection::Module)
            }
            _ => Selection::None,
        };
    }

    /// Records a failure for the screens to show.
    pub fn report(&mut self, action: Action, session: Option<SharedString>, message: impl Into<SharedString>) {
        self.next_error += 1;
        self.errors.push(AppError { id: self.next_error, action, session, message: message.into() });
        if self.errors.len() > ERRORS_KEPT {
            self.errors.remove(0);
        }
    }

    fn command(&mut self, action: Action, session: Option<SharedString>, send: impl FnOnce(&dyn Backend) -> Res<()>) {
        let Some(backend) = self.backend.as_deref() else { return };
        if let Err(failure) = send(backend) {
            self.report(action, session, failure.message);
        }
    }

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = selection;
        cx.notify();
    }

    pub fn set_composing(&mut self, composing: bool, cx: &mut Context<Self>) {
        self.composing = composing;
        cx.notify();
    }

    pub fn start_wish(&mut self, request: WishRequest, cx: &mut Context<Self>) {
        self.composing = false;
        self.command(Action::StartWish, None, |backend| backend.start_wish(request));
        cx.notify();
    }

    pub fn interject(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.run().map(|run| run.session.clone()) {
            self.command(Action::Interject, Some(session.clone()), |backend| backend.interject(&session, text));
        }
        cx.notify();
    }

    pub fn stop_current(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.run().map(|run| run.session.clone()) {
            self.stop(&session);
        }
        cx.notify();
    }

    /// Asks one run to stop. Does not notify; callers do.
    pub fn stop(&mut self, session: &SharedString) {
        self.command(Action::Stop, Some(session.clone()), |backend| backend.stop(session));
    }

    pub fn resume_current(&mut self, note: Option<&str>, cx: &mut Context<Self>) {
        if let Some(session) = self.run().map(|run| run.session.clone()) {
            self.command(Action::Resume, Some(session.clone()), |backend| backend.resume(&session, note));
        }
        cx.notify();
    }

    /// With an engine connected the entry stays listed until the engine reports it gone.
    pub fn delete_memory(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.backend.is_some() {
            self.command(Action::DeleteMemory, None, |backend| backend.delete_memory(id));
        } else {
            self.memory.retain(|m| m.id != id);
        }
        cx.notify();
    }

    /// `text` becomes the entry's new body, which is what later briefs see. With an engine
    /// connected the entry changes when the engine reports it; without one it changes here.
    pub fn correct_memory(&mut self, id: i64, text: &str, cx: &mut Context<Self>) {
        if self.backend.is_some() {
            self.command(Action::CorrectMemory, None, |backend| backend.correct_memory(id, text));
        } else if let Some(entry) = self.memory.iter_mut().find(|m| m.id == id) {
            entry.body = text.trim().to_owned().into();
            entry.corrected = true;
        }
        cx.notify();
    }

    /// Asks the control bar to open its box for the current run, starting with `text`.
    pub fn address_interjection(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        if let Some(session) = self.run().map(|run| run.session.clone()) {
            self.address = Some(Address { session, text: text.into() });
        }
        cx.notify();
    }

    /// Runs the environment checks again.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        self.command(Action::Recheck, None, |backend| backend.recheck());
        cx.notify();
    }
}

/// A wish as the screens show it: the markup a prompt arrives wrapped in is taken off and the
/// words inside stay. The stored wish is never changed, only what is drawn.
///
/// A tag counts as wrapper markup when it is one of the known wrappers, or is written in
/// snake_case or kebab-case (`<pasted_content id="a0d5">`, `</user-query>`). Plain words such as
/// `<div>` or `Vec<T>` are text and stay.
pub fn clean_wish(wish: &str) -> String {
    const WRAPPERS: [&str; 6] = ["pasted_content", "pasted_text", "pasted_image", "attachment", "system-reminder", "user_query"];
    let mut out = String::with_capacity(wish.len());
    let mut rest = wish;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        match wrapper_tag_len(tail, &WRAPPERS) {
            Some(len) => rest = &tail[len..],
            None => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

/// The length of the wrapper tag at the start of `text`, if it begins with one.
fn wrapper_tag_len(text: &str, known: &[&str]) -> Option<usize> {
    let close = text.find('>')?;
    let inner = &text[1..close];
    if inner.contains('<') {
        return None;
    }
    let inner = inner.strip_prefix('/').unwrap_or(inner);
    let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
    let name = &inner[..name_end];
    let tag_like = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    let wrapper = known.contains(&name) || name.contains('_') || name.contains('-');
    (tag_like && wrapper).then_some(close + 1)
}

/// A line of a markdown file as plain text for a list that draws no markdown: headings are
/// dropped, bullet and number markers and bold marks come off.
pub fn plain_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let numbered = digits > 0 && line[digits..].starts_with(". ");
    let body = if numbered {
        &line[digits + 2..]
    } else {
        ["- ", "* ", "+ "].iter().find_map(|marker| line.strip_prefix(marker)).unwrap_or(line)
    };
    let body = body.replace("**", "").replace("__", "");
    let body = body.trim();
    (!body.is_empty()).then(|| body.to_owned())
}

/// A warning the usage ledger raised, kept as facts so each screen can word it for its reader.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    ContextHigh { agent: String, model: String, peak: u64 },
    CacheRewrites { agent: String, count: u64, tokens: u64, usd: f64 },
    FileReads { agent: String, path: String, reads: u64, largest: u64 },
    CopiedHistory { file: String, calls: u64 },
}

const NOTE_TEXT: Table = &[
    ("{agent} reached {peak} tokens of context on {model}.", "{agent}的上下文累積到 {peak} 個 token（模型 {model}）。"),
    (
        "{agent} rewrote its cache {count} times after long waits ({tokens} tokens, about ${usd}).",
        "{agent}在長時間等待後重寫快取 {count} 次（{tokens} 個 token，約 ${usd}）。",
    ),
    ("{agent} read {file} {reads} times, up to {size}.", "{agent}讀了 {file} {reads} 次，最多 {size}。"),
    (
        "{calls} calls in {file} could not be told apart from copied history and were not counted.",
        "{file} 中有 {calls} 次呼叫無法和複製來的歷史區分開，沒有計入。",
    ),
    ("{n} bytes", "{n} 位元組"),
];

fn commas(value: u64) -> String {
    let mut text = value.to_string();
    let mut at = text.len();
    while at > 3 {
        at -= 3;
        text.insert(at, ',');
    }
    text
}

/// "100 KB" for people reading Chinese; English keeps the exact byte count.
fn size_text(lang: Lang, bytes: u64) -> String {
    let figure = |value: f64| if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
    match lang {
        Lang::En => format!("{} bytes", commas(bytes)),
        Lang::ZhTw if bytes < 1024 => translate(lang, NOTE_TEXT, "{n} bytes").replace("{n}", &bytes.to_string()),
        Lang::ZhTw if bytes < 1024 * 1024 => format!("{} KB", figure(bytes as f64 / 1024.0)),
        Lang::ZhTw => format!("{} MB", figure(bytes as f64 / (1024.0 * 1024.0))),
    }
}

/// The role under its one name, shared with the usage screen and the log.
fn agent_label(lang: Lang, agent: &str) -> String {
    super::i18n::role_name(lang, agent).to_string()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(path)
}

impl Note {
    /// The warning as a sentence, with its figures.
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Self::ContextHigh { agent, model, peak } => {
                translate(lang, NOTE_TEXT, "{agent} reached {peak} tokens of context on {model}.")
                    .replace("{agent}", &agent_label(lang, agent))
                    .replace("{peak}", &commas(*peak))
                    .replace("{model}", model)
            }
            Self::CacheRewrites { agent, count, tokens, usd } => translate(
                lang,
                NOTE_TEXT,
                "{agent} rewrote its cache {count} times after long waits ({tokens} tokens, about ${usd}).",
            )
            .replace("{agent}", &agent_label(lang, agent))
            .replace("{count}", &count.to_string())
            .replace("{tokens}", &commas(*tokens))
            .replace("{usd}", &format!("{usd:.2}")),
            Self::FileReads { agent, path, reads, largest } => {
                translate(lang, NOTE_TEXT, "{agent} read {file} {reads} times, up to {size}.")
                    .replace("{agent}", &agent_label(lang, agent))
                    .replace("{file}", if lang == Lang::En { path } else { file_name(path) })
                    .replace("{reads}", &reads.to_string())
                    .replace("{size}", &size_text(lang, *largest))
            }
            Self::CopiedHistory { file, calls } => translate(
                lang,
                NOTE_TEXT,
                "{calls} calls in {file} could not be told apart from copied history and were not counted.",
            )
            .replace("{calls}", &calls.to_string())
            .replace("{file}", file),
        }
    }

    pub fn weight(&self) -> (u8, u64) {
        match self {
            Self::ContextHigh { peak, .. } => (3, *peak),
            Self::CacheRewrites { tokens, .. } => (2, *tokens),
            Self::FileReads { largest, .. } => (1, *largest),
            Self::CopiedHistory { calls, .. } => (0, *calls),
        }
    }
}

/// One note per fact: the same agent reading the same file is one line with the reads added and
/// the largest size kept; an agent's context warning or cache rewrites are one line each. The
/// largest come first.
pub fn group_notes(notes: impl IntoIterator<Item = Note>) -> Vec<Note> {
    let mut grouped: Vec<Note> = vec![];
    for note in notes {
        let same = grouped.iter_mut().find(|have| match (&**have, &note) {
            (Note::ContextHigh { agent: a, .. }, Note::ContextHigh { agent: b, .. }) => a == b,
            (Note::CacheRewrites { agent: a, .. }, Note::CacheRewrites { agent: b, .. }) => a == b,
            (Note::FileReads { agent: a, path: p, .. }, Note::FileReads { agent: b, path: q, .. }) => a == b && p == q,
            (Note::CopiedHistory { file: a, .. }, Note::CopiedHistory { file: b, .. }) => a == b,
            _ => false,
        });
        match (same, note) {
            (None, note) => grouped.push(note),
            (Some(Note::ContextHigh { model, peak, .. }), Note::ContextHigh { model: m, peak: p, .. }) => {
                if p > *peak {
                    (*model, *peak) = (m, p);
                }
            }
            (Some(Note::CacheRewrites { count, tokens, usd, .. }), Note::CacheRewrites { count: c, tokens: t, usd: u, .. }) => {
                *count += c;
                *tokens += t;
                *usd += u;
            }
            (Some(Note::FileReads { reads, largest, .. }), Note::FileReads { reads: r, largest: l, .. }) => {
                *reads += r;
                *largest = (*largest).max(l);
            }
            (Some(Note::CopiedHistory { calls, .. }), Note::CopiedHistory { calls: c, .. }) => *calls = (*calls).max(c),
            (Some(_), _) => {}
        }
    }
    grouped.sort_by(|a, b| b.weight().cmp(&a.weight()));
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn wrapper_tags_come_off_and_the_words_inside_stay() {
        let raw = "<pasted_content id=\"a0d5\"> 工作目錄在：[E:\\x]\n做完整測試 </pasted_content>";
        assert_eq!(clean_wish(raw), "工作目錄在：[E:\\x]\n做完整測試");
        assert_eq!(clean_wish("<user-query>fix it</user-query> now"), "fix it now");
        assert_eq!(clean_wish("<pasted_content/>plain"), "plain");
    }

    #[test]
    fn ordinary_angle_brackets_are_not_markup() {
        for text in ["return Vec<T> from <div> and a < b > c", "if x<5 and y>3", "use <Foo> or <b>bold</b>", "<3 love"] {
            assert_eq!(clean_wish(text), text);
        }
    }

    #[test]
    fn an_unclosed_bracket_keeps_the_rest() {
        assert_eq!(clean_wish("<pasted_content id=\"a"), "<pasted_content id=\"a");
    }

    fn reads(agent: &str, path: &str, reads: u64, largest: u64) -> Note {
        Note::FileReads { agent: agent.into(), path: path.into(), reads, largest }
    }

    #[test]
    fn reads_of_one_file_become_one_note_largest_first() {
        let grouped = group_notes([
            reads("chronos:reviewer", "E:/p/schema.json", 1, 90_000),
            reads("chronos:reviewer", "E:/p/big.log", 1, 900_000),
            reads("chronos:reviewer", "E:/p/schema.json", 4, 102_400),
            reads("chronos:architect", "E:/p/schema.json", 2, 50),
        ]);
        assert_eq!(
            grouped,
            vec![
                reads("chronos:reviewer", "E:/p/big.log", 1, 900_000),
                reads("chronos:reviewer", "E:/p/schema.json", 5, 102_400),
                reads("chronos:architect", "E:/p/schema.json", 2, 50),
            ]
        );
        assert_eq!(grouped[1].text(Lang::ZhTw), "審查員讀了 schema.json 5 次，最多 100 KB。");
    }

    #[test]
    fn context_and_cache_notes_are_one_per_agent() {
        let high = |peak: u64| Note::ContextHigh { agent: "d".into(), model: format!("m{peak}"), peak };
        let rewrites = |count, tokens, usd| Note::CacheRewrites { agent: "d".into(), count, tokens, usd };
        let grouped = group_notes([high(900_000), high(965_312), rewrites(2, 10, 1.5), rewrites(3, 20, 1.5)]);
        assert_eq!(grouped, vec![high(965_312), rewrites(5, 30, 3.0)]);
    }

    #[test]
    fn every_kind_has_a_chinese_sentence_that_keeps_its_figures() {
        let notes = [
            Note::ContextHigh { agent: "chronos:dispatcher".into(), model: "claude-sonnet-5-5".into(), peak: 965_312 },
            Note::CacheRewrites { agent: "chronos:dispatcher".into(), count: 6, tokens: 21_000_000, usd: 12.5 },
            reads("chronos:reviewer", "a/schema.json", 5, 100),
            Note::CopiedHistory { file: "s.jsonl".into(), calls: 7 },
        ];
        let figures = [
            vec!["965,312", "claude-sonnet-5-5", "調度員"],
            vec!["6", "21,000,000", "12.50"],
            vec!["schema.json", "5", "100 位元組"],
            vec!["7", "s.jsonl", "複製"],
        ];
        for (note, figures) in notes.iter().zip(figures) {
            let zh = note.text(Lang::ZhTw);
            let en = note.text(Lang::En);
            assert_ne!(zh, en);
            assert!(!zh.contains('{') && !en.contains('{'), "{zh} / {en}");
            for figure in figures {
                assert!(zh.contains(figure), "{zh} lacks {figure}");
            }
        }
        assert_eq!(notes[2].text(Lang::En), "Reviewer read a/schema.json 5 times, up to 100 bytes.");
    }

    #[test]
    fn other_warnings_leave_out_the_text_of_ledger_notes() {
        let note = reads("chronos:reviewer", "a/b.json", 2, 5000);
        let run = Run {
            warnings: vec![note.text(Lang::ZhTw).into(), "Module x depends on y".into()],
            notes: vec![note],
            ..crate::desktop::demo::state().runs[0].clone()
        };
        let other: Vec<&str> = run.other_warnings().map(|w| w.as_ref()).collect();
        assert_eq!(other, ["Module x depends on y"]);
    }

    #[test]
    fn markdown_lines_lose_their_markup_and_headings_vanish() {
        assert_eq!(plain_line("# Decisions"), None);
        assert_eq!(plain_line("- 精讀範圍：先以 **關鍵詞** 判定"), Some("精讀範圍：先以 關鍵詞 判定".into()));
        assert_eq!(plain_line("  12. second  "), Some("second".into()));
        assert_eq!(plain_line("1.5 million tokens"), Some("1.5 million tokens".into()));
        assert_eq!(plain_line("   "), None);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(Lang::ZhTw, 100 * 1024), "100 KB");
        assert_eq!(size_text(Lang::ZhTw, 1536), "1.5 KB");
        assert_eq!(size_text(Lang::ZhTw, 3 * 1024 * 1024), "3.0 MB");
        assert_eq!(size_text(Lang::En, 1_234_567), "1,234,567 bytes");
    }
}
