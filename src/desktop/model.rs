//! What the screens draw and the commands they send. Screens never read `~/.axiom` or start processes.
use gpui_kit::*;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Active,
    Waiting,
    Blocked,
    Done,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Running",
            Self::Waiting => "Waiting on batch",
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

#[derive(Clone, Debug)]
pub struct Stage {
    pub name: SharedString,
    pub phase: Phase,
    pub note: Option<SharedString>,
}

/// Text shown in the detail panel: a file on disk, or text carried in the model.
#[derive(Clone, Debug)]
pub enum TextSource {
    Path(PathBuf),
    Inline(SharedString),
}

#[derive(Clone, Debug, Default)]
pub struct UnitFiles {
    pub brief: Option<TextSource>,
    pub report: Option<TextSource>,
    pub verify: Option<TextSource>,
    pub review: Option<TextSource>,
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct Module {
    pub id: SharedString,
    pub purpose: SharedString,
    pub depends: Vec<usize>,
    pub units: Vec<usize>,
    pub contract: Option<TextSource>,
}

/// A numbered item of the plan, and the modules that deliver it.
#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct Need {
    pub kind: NeedKind,
    pub title: SharedString,
    pub note: SharedString,
}

#[derive(Clone, Debug)]
pub struct ModelCost {
    pub model: SharedString,
    /// `None` when the run has not reported a figure.
    pub usd: Option<f64>,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

/// Spend attributed to one role. Left empty when a run has no per-role record.
#[derive(Clone, Debug)]
pub struct RoleCost {
    pub role: SharedString,
    /// `None` when the run has not reported a figure.
    pub usd: Option<f64>,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

#[derive(Clone, Debug)]
pub struct AgentContext {
    pub agent: SharedString,
    pub peak_tokens: u64,
}

#[derive(Clone, Debug, Default)]
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

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: SharedString,
    pub source: SharedString,
    pub text: SharedString,
    pub level: LogLevel,
}

#[derive(Clone, Debug)]
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
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub id: i64,
    pub project: SharedString,
    pub title: SharedString,
    pub body: SharedString,
    pub stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Debug)]
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

/// Commands a screen can send. The app shell connects these to the session host.
pub trait Backend: 'static {
    fn start_wish(&self, request: WishRequest);
    fn interject(&self, session: &str, text: &str);
    fn stop(&self, session: &str);
    fn resume(&self, session: &str, note: Option<&str>);
    fn delete_memory(&self, id: i64);
    fn refresh(&self);
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
        cx.notify();
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
        if let Some(backend) = &self.backend {
            backend.start_wish(request);
        }
        cx.notify();
    }

    pub fn interject(&mut self, text: &str, cx: &mut Context<Self>) {
        if let (Some(run), Some(backend)) = (self.run(), &self.backend) {
            backend.interject(&run.session, text);
        }
        cx.notify();
    }

    pub fn stop_current(&mut self, cx: &mut Context<Self>) {
        if let (Some(run), Some(backend)) = (self.run(), &self.backend) {
            backend.stop(&run.session);
        }
        cx.notify();
    }

    pub fn resume_current(&mut self, note: Option<&str>, cx: &mut Context<Self>) {
        if let (Some(run), Some(backend)) = (self.run(), &self.backend) {
            backend.resume(&run.session, note);
        }
        cx.notify();
    }

    pub fn delete_memory(&mut self, id: i64, cx: &mut Context<Self>) {
        self.memory.retain(|m| m.id != id);
        if let Some(backend) = &self.backend {
            backend.delete_memory(id);
        }
        cx.notify();
    }
}
