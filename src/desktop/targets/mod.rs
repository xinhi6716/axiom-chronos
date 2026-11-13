use super::{model::{AppState, Screen}, ux::Page};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState { Live, Empty, Error, Action }
impl Default for CaptureState { fn default() -> Self { Self::Live } }
impl CaptureState {
 pub fn key(self) -> &'static str { match self { Self::Live => "live", Self::Empty => "empty", Self::Error => "error", Self::Action => "action" } }
 pub fn from_key(s: &str) -> Option<Self> { match s { "live" => Some(Self::Live), "empty" => Some(Self::Empty), "error" => Some(Self::Error), "action" => Some(Self::Action), _ => None } }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Opening { Screen(Screen), Page(&'static str), ResearchTab(&'static str), PaperStep(&'static str), Window(&'static str), Overlay(&'static str), Dialog(&'static str) }
#[derive(Clone, Debug)]
pub struct Target { pub id: &'static str, pub group: &'static str, pub opening: Opening, pub panels: &'static [&'static str], pub actions: &'static [&'static str], pub states: &'static [CaptureState] }
#[derive(Debug, PartialEq)]
pub struct Unregistered { pub target: String, pub state: CaptureState }
mod shell;
mod overlays;
mod run;
mod mesh;
mod wishes;
mod panels;
mod mini;
mod sessions_settings;
mod research;
mod paper;
mod modes;
#[cfg(test)] mod accept;
fn entry(id: &'static str, group: &'static str, opening: Opening) -> Target {
 let states: &'static [CaptureState] = match opening {
  Opening::Screen(Screen::Run | Screen::Wishes | Screen::Memory | Screen::Usage) => &[CaptureState::Live, CaptureState::Empty, CaptureState::Error],
  Opening::Screen(_) | Opening::Page(_) | Opening::Window("mini" | "focus.graph" | "focus.checklist") | Opening::Overlay("palette" | "composer") | Opening::Dialog("close" | "composer") => &[CaptureState::Live],
  _ => &[],
 };
 // IDs describe existing UI; out-of-scope view owners can adopt them.
 let (panels, actions): (&'static [&'static str], &'static [&'static str]) = match id {
  "run" => (&["run.overview"], &["action:run.make-wish", "action:run.open-wishes"]),
  "wishes" => (&["wishes.list"], &["action:wishes.make-wish", "action:wishes.mark-all-read"]),
  "memory" => (&["memory.list"], &["action:memory.correct", "action:memory.delete"]),
  "usage" => (&["usage.summary"], &["action:usage.make-wish"]),
  "settings" => (&["settings.page"], &["action:settings.toggle-extreme"]),
  "research" => (&["research.page"], &["action:research.back", "action:research.open"]),
  "storage" => (&["storage.page"], &["action:storage.preview", "action:storage.confirm"]),
  "chat" => (&["chat.page"], &["action:chat.send", "action:chat.promote"]),
  "quick" => (&["quick.page"], &["action:quick.start", "action:quick.review", "action:quick.promote"]),
  "plaza" => (&["plaza.page"], &["action:plaza.send", "action:plaza.round", "action:plaza.debate", "action:plaza.vote", "action:plaza.promote"]),
  "session" => (&["session.page"], &["action:session.back", "action:session.send", "action:session.takeover", "action:session.handback", "action:session.reconnect"]),
  "paper" => (&["paper.page"], &["action:paper.back", "action:paper.start", "action:paper.cancel"]),
  "routine" => (&["routine.page"], &[]),
  "research.map" => (&["research.map.panel"], &["action:research.map.open"]),
  "research.sources" => (&["research.sources.panel"], &["action:research.sources.previous", "action:research.sources.next", "action:research.sources.sample"]),
  "research.evidence" => (&["research.evidence.panel"], &["action:research.evidence.select-layer", "action:research.evidence.select-entity", "action:research.evidence.previous", "action:research.evidence.next", "action:research.evidence.open-dataset", "action:research.evidence.correct"]),
  "research.dataset" => (&["research.dataset.panel"], &["action:research.dataset.correct"]),
  "research.exports" => (&["research.exports.panel"], &["action:research.exports.csv", "action:research.exports.bundle", "action:research.exports.snapshot"]),
  "paper.form" => (&["paper.form.panel"], &["action:paper.form.shortlist", "action:paper.form.save-models"]),
  "paper.shortlist" => (&["paper.shortlist.list"], &["action:paper.shortlist.choose"]),
  "paper.estimate" => (&["paper.estimate.summary"], &["action:paper.estimate.request", "action:paper.estimate.acknowledge"]),
  "paper.progress" => (&["paper.progress.passes"], &[]),
  "paper.changelog" => (&["paper.changelog.log"], &[]),
  "paper.preview" => (&["paper.preview.document"], &["action:paper.preview.previous", "action:paper.preview.next", "action:paper.preview.open-pdf"]),
  "paper.readiness" => (&["paper.readiness.status"], &[]),
  "mini" => (&["mini.body"], &["action:mini.pin", "action:mini.back", "action:mini.open-wish"]),
  "focus.graph" => (&["focus.graph.panel"], &["action:focus.graph.exit"]),
  "focus.checklist" => (&["focus.checklist.panel"], &["action:focus.checklist.exit"]),
  "palette" => (&["palette.results"], &["action:palette.choose"]),
  "composer" => (&["composer.form"], &["action:composer.start", "action:composer.cancel"]),
  "dialog.close" => (&["dialog.close.prompt"], &["action:dialog.close.cancel", "action:dialog.close.confirm"]),
  "dialog.control" => (&["dialog.control.panel"], &["action:dialog.control.send", "action:dialog.control.stop"]),
  "dialog.interjection" => (&["dialog.interjection.prompt"], &["action:dialog.interjection.send"]),
  "dialog.composer" => (&["dialog.composer.form"], &["action:dialog.composer.start", "action:dialog.composer.cancel"]),
  "dialog.evolve" => (&["dialog.evolve.prompt"], &["action:dialog.evolve.confirm", "action:dialog.evolve.cancel"]),
  _ => unreachable!("target inventory must declare its UI ids: {id}"),
 };
 Target { id, group, opening, panels, actions, states }
}
pub fn all() -> Vec<Target> { let mut v = Vec::new();
 v.extend(shell::targets());
 v.extend(overlays::targets());
 v.extend(run::targets());
 v.extend(mesh::targets());
 v.extend(wishes::targets());
 v.extend(panels::targets());
 v.extend(mini::targets());
 v.extend(sessions_settings::targets());
 v.extend(research::targets());
 v.extend(paper::targets());
 v.extend(modes::targets());
 v }
pub fn find(id: &str) -> Option<Target> { all().into_iter().find(|t| t.id == id) }
pub(crate) fn unavailable(t: &Target, s: CaptureState) -> Unregistered { Unregistered { target: t.id.into(), state: s } }
pub fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> {
 match t.group {
  "shell" => shell::prepare(t, s, state),
  "overlays" => overlays::prepare(t, s, state),
  "run" => run::prepare(t, s, state),
  "mesh" => mesh::prepare(t, s, state),
  "wishes" => wishes::prepare(t, s, state),
  "panels" => panels::prepare(t, s, state),
  "mini" => mini::prepare(t, s, state),
  "sessions_settings" => sessions_settings::prepare(t, s, state),
  "research" => research::prepare(t, s, state),
  "paper" => paper::prepare(t, s, state),
  "modes" => modes::prepare(t, s, state),
  _ => Err(unavailable(t,s)),
 }
}
fn prepare_basic(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> {
 if s != CaptureState::Live {
  if !matches!(t.opening, Opening::Screen(Screen::Run | Screen::Wishes | Screen::Memory | Screen::Usage)) { return Err(unavailable(t,s)); }
  match s {
   CaptureState::Empty => {
    state.runs.clear(); state.current = None; state.memory.clear(); state.checks.clear();
    state.quota = Default::default(); state.codex_slots = None; state.codex_models.clear(); state.live_models.clear();
    for kind in [super::model::FeedKind::Runs, super::model::FeedKind::Memory, super::model::FeedKind::Checks, super::model::FeedKind::Usage, super::model::FeedKind::Quota] { let feed = state.feeds.get_mut(kind); feed.loaded = true; feed.error = None; }
   },
   CaptureState::Error => { super::demo::apply_data_scene(state, "feed-error"); },
   _ => return Err(unavailable(t,s)),
  }
 }
 match t.opening {
  Opening::Screen(screen) => { state.view = screen; state.ux.page = None; },
  Opening::Page(name) => {
    let page = match name { "research" => Page::Research, "storage" => Page::Storage, "chat" => Page::Chat, "quick" => Page::QuickTask, "plaza" => Page::Plaza, "routine" => Page::Routine,
      "session" => Page::Session(state.ux.seams.sessions.as_ref().and_then(|service| service.sessions().into_iter().next()).ok_or_else(|| unavailable(t,s))?.id.into()),
      "paper" => { if state.ux.seams.paper.is_none() { return Err(unavailable(t,s)); } Page::Paper(state.runs.first().ok_or_else(|| unavailable(t,s))?.session.clone()) },
      _ => return Err(unavailable(t,s)) };
    state.ux.page = Some(page);
  },
  Opening::Window("mini" | "focus.graph" | "focus.checklist") => { state.view = Screen::Run; state.ux.page = None; },
  Opening::Overlay("palette") | Opening::Overlay("composer") | Opening::Dialog("close" | "composer") => {},
  _ => return Err(unavailable(t,s)),
 }
 Ok(())
}
