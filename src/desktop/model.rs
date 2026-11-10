//! What the screens draw and the commands they send. Screens never read `~/.axiom` or start processes.
use crate::base::Res;
use gpui_kit::*;
use std::collections::HashMap;
use std::path::PathBuf;

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
    pub warnings: Vec<SharedString>,
}

impl Run {
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

    /// `text` becomes the entry's new body, which is what later briefs see.
    pub fn correct_memory(&mut self, id: i64, text: &str, cx: &mut Context<Self>) {
        self.command(Action::CorrectMemory, None, |backend| backend.correct_memory(id, text));
        cx.notify();
    }

    /// Runs the environment checks again.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        self.command(Action::Recheck, None, |backend| backend.recheck());
        cx.notify();
    }
}
