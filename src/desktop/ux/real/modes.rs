//! Real ModeEngine over src/modes (chat, plaza, quick, promotion).
use crate::{
    base::{self, Failure, Res},
    desktop::ux::seams::modes::{
        ChatThread, Message, ModeEngine, ModeRef, Pin, PlazaAsk, PlazaRoom, PlazaSeat, QuickRequest,
        QuickStep, QuickTask, RouteDecision, Speaker,
    },
    modes::{
        self,
        chat::{Chat, ChatOptions},
        context,
        launch::{LaunchKind, LaunchRequest, Launcher},
        plaza::{
            self, access,
            pins::{self, ConclusionSink, PinKind},
            router::{Pick, Router},
        },
        promotion,
        providers::{Member, Provider},
        quick::{Brief, QuickConfig, QuickLauncher},
        readonly::ReadOnly,
        runs,
        store::Transcript,
        Mode, Snapshot,
    },
    service::launcher::ServiceLauncher,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAIN_THREAD: &str = "main";

fn member(s: Speaker) -> Option<Member> {
    match s {
        Speaker::You => None,
        Speaker::Core => Some(Member::Opus),
        Speaker::Astra => Some(Member::Astra),
        Speaker::Fable => Some(Member::Fable),
        Speaker::Argon => Some(Member::Argon),
    }
}
fn speaker(m: Member) -> Speaker {
    match m {
        Member::Opus => Speaker::Core,
        Member::Astra => Speaker::Astra,
        Member::Fable => Speaker::Fable,
        Member::Argon => Speaker::Argon,
    }
}
fn author(name: &str) -> Option<Speaker> {
    if name == "person" {
        Some(Speaker::You)
    } else {
        Member::parse(name).map(speaker)
    }
}

/// Builds the quick launcher from the environment when a task starts, using the session project.
pub struct LazyQuick {
    pub home: PathBuf,
}
impl Launcher for LazyQuick {
    fn launch(&self, req: &LaunchRequest) -> Res<modes::launch::Launched> {
        let project = runs::load_session(&self.home, &req.session)?
            .project
            .ok_or_else(|| Failure::new("invalid_argument", "quick requires a project"))?;
        QuickLauncher {
            cfg: QuickConfig::from_env(&project)?,
        }
        .launch(req)
    }
}

struct AcceptPin;
impl ConclusionSink for AcceptPin {
    fn accept(&self, _: &pins::Pin) -> Res<()> {
        Ok(())
    }
}

#[derive(Clone)]
struct QuickMeta {
    session: String,
    run: String,
}
#[derive(Default)]
struct Tasks {
    tasks: HashMap<String, QuickTask>,
    meta: HashMap<String, QuickMeta>,
}

pub struct ModesEngine {
    pub home: PathBuf,
    pub provider: Arc<dyn Provider + Send + Sync>,
    pub router: Arc<dyn Router + Send + Sync>,
    pub wish: Arc<ServiceLauncher>,
    pub quick: Arc<dyn Launcher + Send + Sync>,
    state: Arc<Mutex<Tasks>>,
    alias_lock: Mutex<()>,
}

/// Quick text carries the brief: `SCOPE <paths>` and `VERIFY <command>` lines; the rest is the goal.
pub fn parse_brief(text: &str, review: bool) -> Brief {
    let mut goal = Vec::new();
    let mut scope = Vec::new();
    let mut verify = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SCOPE ") {
            scope.extend(rest.split_whitespace().map(str::to_owned));
        } else if let Some(rest) = line.strip_prefix("VERIFY ") {
            verify = rest.trim().to_owned();
        } else {
            goal.push(line);
        }
    }
    Brief {
        goal: goal.join("\n").trim().to_owned(),
        scope,
        verify,
        review,
    }
}

fn set_step(state: &Mutex<Tasks>, id: &str, step: QuickStep) {
    if let Some(t) = state.lock().unwrap().tasks.get_mut(id) {
        t.step = step;
    }
}

impl ModesEngine {
    pub fn new(
        home: &Path,
        provider: Arc<dyn Provider + Send + Sync>,
        router: Arc<dyn Router + Send + Sync>,
        wish: Arc<ServiceLauncher>,
        quick: Arc<dyn Launcher + Send + Sync>,
    ) -> Self {
        Self {
            home: home.into(),
            provider,
            router,
            wish,
            quick,
            state: Arc::default(),
            alias_lock: Mutex::new(()),
        }
    }
    fn aliases_path(&self) -> PathBuf {
        self.home.join("desktop").join("modes").join("aliases.json")
    }
    fn read_aliases(&self) -> BTreeMap<String, String> {
        std::fs::read(self.aliases_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn lookup(&self, key: &str, id: &str) -> Option<String> {
        let _g = self.alias_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = self.read_aliases().get(key) {
            return Some(s.clone());
        }
        runs::load_session(&self.home, id).ok().map(|s| s.id)
    }
    fn session_for(&self, kind: &str, id: &str, create: impl FnOnce() -> Res<String>) -> Res<String> {
        let key = format!("{kind}:{id}");
        if let Some(s) = self.lookup(&key, id) {
            return Ok(s);
        }
        let _g = self.alias_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.read_aliases();
        if let Some(s) = all.get(&key) {
            return Ok(s.clone());
        }
        let session = create()?;
        all.insert(key, session.clone());
        if let Some(dir) = self.aliases_path().parent() {
            std::fs::create_dir_all(dir).map_err(|e| Failure::new("io", e.to_string()))?;
        }
        base::atomic_write(
            &self.aliases_path(),
            &serde_json::to_vec(&all).map_err(|e| Failure::new("io", e.to_string()))?,
        )
        .map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(session)
    }
    fn chat_session(&self, id: &str) -> Res<String> {
        self.session_for("chat", id, || {
            Ok(Chat::open(&self.home, None, &[], ChatOptions::default())?
                .session
                .id)
        })
    }
    fn plaza_room(&self, id: &str) -> Res<plaza::Room> {
        let sid = self.session_for("plaza", id, || {
            let budgets = access::Budgets(
                Member::ALL
                    .into_iter()
                    .map(|m| {
                        (
                            m,
                            access::Budget {
                                max_calls: 20,
                                max_usd: Some(5.0),
                            },
                        )
                    })
                    .collect(),
            );
            Ok(plaza::open(&self.home, None, &[], budgets)?.session.id)
        })?;
        plaza::resume(&self.home, &sid)
    }
    fn tools(&self, session: &str) -> ReadOnly {
        let project = runs::load_session(&self.home, session)
            .ok()
            .and_then(|s| s.project);
        ReadOnly::new(project.as_deref(), &self.home)
    }
    fn texts(&self, session: &str, kinds: &[&str]) -> Vec<Message> {
        let Ok(t) = Transcript::open(&self.home, session) else {
            return vec![];
        };
        t.read(0, 100_000)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| kinds.contains(&m.kind.as_str()))
            .filter_map(|m| {
                Some(Message {
                    id: m.seq,
                    from: author(&m.author)?,
                    text: m.text,
                    at_ms: 0,
                })
            })
            .collect()
    }
    fn snapshot_text(&self, session: &str) -> Res<String> {
        let snap = Transcript::open(&self.home, session)?.snapshot(None)?;
        let msgs: Vec<_> = snap
            .messages
            .into_iter()
            .filter(|m| matches!(m.kind.as_str(), "message" | "reply" | "text" | "vote"))
            .collect();
        Ok(context::bounded(&msgs, context::DEFAULT_CONTEXT_BYTES))
    }
    /// Promote a mode to a wish through the 2a service; returns the service handle.
    pub fn promote_wish(&self, from: &ModeRef, goal: &str) -> Res<String> {
        let (session, run) = match from {
            ModeRef::Chat(id) => {
                let sid = self.chat_session(id)?;
                let run = Chat::resume(&self.home, &sid, ChatOptions::default())?.run;
                (sid, run)
            }
            ModeRef::Plaza(id) => {
                let room = self.plaza_room(id)?;
                (room.session.id.clone(), room.run)
            }
            ModeRef::Quick(id) => {
                let meta = self
                    .state
                    .lock()
                    .unwrap()
                    .meta
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Failure::new("not_found", "Mode not found"))?;
                (meta.session.clone(), runs::load_run(&self.home, &meta.run)?)
            }
        };
        let session = runs::load_session(&self.home, &session)?;
        let snapshot: Snapshot = Transcript::open(&self.home, &session.id)?.snapshot(None)?;
        let p = promotion::promote(
            &self.home,
            &session,
            &run,
            LaunchKind::Wish,
            goal,
            None,
            snapshot,
            &*self.wish,
        )?;
        Ok(p.child.detail["handle"].as_str().unwrap_or_default().into())
    }
    fn picks(picks: &[Pick]) -> (Vec<Speaker>, String) {
        (
            picks.iter().map(|p| speaker(p.member)).collect(),
            picks
                .iter()
                .map(|p| format!("{}: {}", p.member.name(), p.reason))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn decision(picked: Vec<Speaker>, reason: &str, skipped: usize) -> RouteDecision {
    RouteDecision {
        picked,
        reason: if skipped > 0 {
            format!("{reason}; over-budget member skipped")
        } else {
            reason.into()
        },
    }
}

impl ModeEngine for ModesEngine {
    fn chat_send(&self, thread: &str, text: &str) -> Res<()> {
        let sid = self.chat_session(thread)?;
        let chat = Chat::resume(&self.home, &sid, ChatOptions::default())?;
        chat.say(&*self.provider, &self.tools(&sid), text)
            .map(|_| ())
    }
    fn chat(&self, thread: &str) -> ChatThread {
        let messages = self
            .lookup(&format!("chat:{thread}"), thread)
            .map(|sid| self.texts(&sid, &["message", "reply"]))
            .unwrap_or_default();
        ChatThread {
            id: thread.into(),
            messages,
            busy: false,
        }
    }
    fn quick_start(&self, r: &QuickRequest) -> Res<String> {
        let brief = parse_brief(&r.text, r.review);
        brief.validate()?;
        let session = runs::create_session(&self.home, r.cwd.as_deref(), &brief.scope)?;
        let origin = runs::start_run(&self.home, &session.id, Mode::Chat, None, None)?;
        let snapshot = r.context.as_ref().map(|c| Snapshot {
            session: session.id.clone(),
            upto_seq: 0,
            thread: None,
            messages: vec![modes::Message {
                seq: 0,
                ts: base::now_iso(),
                author: "person".into(),
                kind: "message".into(),
                text: c.clone(),
                thread: None,
                data: serde_json::Value::Null,
            }],
        });
        let id = format!("quick-{}", base::unique());
        {
            let mut s = self.state.lock().unwrap();
            s.tasks.insert(
                id.clone(),
                QuickTask {
                    id: id.clone(),
                    request: r.clone(),
                    step: QuickStep::Queued,
                    unit: None,
                },
            );
            s.meta.insert(
                id.clone(),
                QuickMeta {
                    session: session.id.clone(),
                    run: origin.id.clone(),
                },
            );
        }
        let (state, launcher, tid) = (self.state.clone(), self.quick.clone(), id.clone());
        let request = LaunchRequest {
            kind: LaunchKind::Quick,
            session: session.id,
            parent_run: origin.id,
            goal: brief.goal.clone(),
            scope: brief.scope.clone(),
            verify: Some(brief.verify.clone()),
            review: brief.review,
            context: snapshot,
            sources: vec![],
        };
        std::thread::spawn(move || {
            set_step(&state, &tid, QuickStep::Building);
            let step = match launcher.launch(&request) {
                Ok(l) if l.detail["passed"].as_bool().unwrap_or(false) => QuickStep::Done {
                    summary: if request.review {
                        "Changes verified and reviewed.".into()
                    } else {
                        "Changes verified.".into()
                    },
                },
                Ok(l) => QuickStep::Failed {
                    reason: format!(
                        "Quick task ended in state {}.",
                        l.detail["state"].as_str().unwrap_or("failed")
                    ),
                },
                Err(e) => QuickStep::Failed { reason: e.message },
            };
            set_step(&state, &tid, step);
        });
        Ok(id)
    }
    fn quick(&self, id: &str) -> Option<QuickTask> {
        self.state.lock().unwrap().tasks.get(id).cloned()
    }
    fn plaza_send(&self, room: &str, ask: &PlazaAsk) -> Res<RouteDecision> {
        let r = self.plaza_room(room)?;
        let tools = self.tools(&r.session.id);
        let (provider, router) = (&*self.provider, &*self.router);
        let person = |text: &str| -> Res<()> {
            Transcript::open(&self.home, &r.session.id)?.append(
                "person",
                "text",
                text,
                Some(MAIN_THREAD),
                serde_json::Value::Null,
            )?;
            Ok(())
        };
        match ask {
            PlazaAsk::Say { text, mentions } => {
                let mut text = text.clone();
                for m in mentions.iter().filter_map(|s| member(*s)) {
                    if !plaza::protocol::mentions(&text).contains(&m) {
                        text.push_str(&format!(" @{}", m.name()));
                    }
                }
                let out = r.post(&text, Some(MAIN_THREAD), router, provider, &tools)?;
                Ok(match &out.routing {
                    Some(routing) => {
                        let (picked, why) = Self::picks(&routing.picks);
                        let why = if routing.note.is_empty() {
                            why
                        } else {
                            routing.note.clone()
                        };
                        decision(picked, &why, out.skipped.len())
                    }
                    None => decision(
                        plaza::protocol::mentions(&text)
                            .into_iter()
                            .map(speaker)
                            .collect(),
                        "Selected mentioned members",
                        out.skipped.len(),
                    ),
                })
            }
            PlazaAsk::Round { text } => {
                person(text)?;
                let replies = r.round(MAIN_THREAD, text, &[], provider, &tools)?;
                let skipped = Member::ALL.len() - replies.len();
                Ok(decision(
                    replies.iter().filter_map(|m| author(&m.author)).collect(),
                    "Asked every member",
                    skipped,
                ))
            }
            PlazaAsk::Debate { text, a, b, judge } => {
                let (Some(a), Some(b), Some(j)) = (member(*a), member(*b), member(*judge)) else {
                    return Err(Failure::new("invalid_argument", "debate needs three members"));
                };
                person(text)?;
                let replies = r.debate(MAIN_THREAD, text, a, b, j, provider, &tools)?;
                Ok(decision(
                    replies.iter().filter_map(|m| author(&m.author)).collect(),
                    "Selected debate participants",
                    3 - replies.len().min(3),
                ))
            }
            PlazaAsk::Vote { question } => {
                let result = r.vote(
                    MAIN_THREAD,
                    question,
                    &["yes".to_owned(), "no".to_owned()],
                    &Member::ALL,
                    provider,
                )?;
                Ok(decision(
                    result.ballots.iter().map(|(m, _)| speaker(*m)).collect(),
                    "Selected council members for vote",
                    Member::ALL.len() - result.ballots.len(),
                ))
            }
        }
    }
    fn plaza(&self, room: &str) -> PlazaRoom {
        let Some(sid) = self.lookup(&format!("plaza:{room}"), room) else {
            return PlazaRoom {
                id: room.into(),
                ..Default::default()
            };
        };
        let budgets = access::load(&self.home, &sid).ok();
        let spend = access::spend(&self.home, &sid).unwrap_or_default();
        let seats = Member::ALL
            .into_iter()
            .map(|m| PlazaSeat {
                member: speaker(m),
                budget_usd: budgets
                    .as_ref()
                    .and_then(|b| b.0.get(&m))
                    .and_then(|b| b.max_usd),
                spent_usd: Some(spend.get(&m).map_or(0.0, |s| s.usd)),
            })
            .collect();
        let last_route = Transcript::open(&self.home, &sid)
            .and_then(|t| t.read(0, 100_000))
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|m| m.kind == "router")
            .and_then(|m| {
                serde_json::from_value::<Vec<Pick>>(m.data["picks"].clone())
                    .ok()
                    .map(|p| (p, m.text))
            })
            .map(|(picks, note)| {
                let (picked, why) = Self::picks(&picks);
                RouteDecision {
                    picked,
                    reason: if note.is_empty() { why } else { note },
                }
            });
        PlazaRoom {
            id: room.into(),
            seats,
            messages: self.texts(&sid, &["text", "vote"]),
            speaker: None,
            last_route,
            pins: pins::pins(&self.home, &sid)
                .unwrap_or_default()
                .into_iter()
                .map(|p| Pin {
                    text: p.text,
                    sources: p.sources.iter().map(|s| s.seq).collect(),
                })
                .collect(),
        }
    }
    fn pin(&self, room: &str, message: u64) -> Res<()> {
        let r = self.plaza_room(room)?;
        pins::pin(
            &self.home,
            &r.session.id,
            message,
            PinKind::Decision,
            "person",
            &AcceptPin,
        )
        .map(|_| ())
    }
    fn handoff(&self, from: &ModeRef) -> Res<String> {
        let not_found = || Failure::new("not_found", "Mode not found");
        match from {
            ModeRef::Chat(id) => self.snapshot_text(
                &self
                    .lookup(&format!("chat:{id}"), id)
                    .ok_or_else(not_found)?,
            ),
            ModeRef::Plaza(id) => self.snapshot_text(
                &self
                    .lookup(&format!("plaza:{id}"), id)
                    .ok_or_else(not_found)?,
            ),
            ModeRef::Quick(id) => self
                .state
                .lock()
                .unwrap()
                .tasks
                .get(id)
                .map(|t| t.request.text.clone())
                .ok_or_else(not_found),
        }
    }
}
