use crate::base::{Failure, Res};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModeKind {
    #[default]
    Wish,
    Chat,
    QuickTask,
    Plaza,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    You,
    Core,
    Astra,
    Fable,
    Argon,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: u64,
    pub from: Speaker,
    pub text: String,
    pub at_ms: i64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatThread {
    pub id: String,
    pub messages: Vec<Message>,
    pub busy: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuickRequest {
    pub cwd: Option<PathBuf>,
    pub text: String,
    pub review: bool,
    pub context: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum QuickStep {
    Queued,
    Building,
    Verifying,
    Reviewing,
    Done { summary: String },
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuickTask {
    pub id: String,
    pub request: QuickRequest,
    pub step: QuickStep,
    pub unit: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PlazaAsk {
    Say {
        text: String,
        mentions: Vec<Speaker>,
    },
    Round {
        text: String,
    },
    Debate {
        text: String,
        a: Speaker,
        b: Speaker,
        judge: Speaker,
    },
    Vote {
        question: String,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct RouteDecision {
    pub picked: Vec<Speaker>,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PlazaSeat {
    pub member: Speaker,
    pub budget_usd: Option<f64>,
    pub spent_usd: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub text: String,
    pub sources: Vec<u64>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlazaRoom {
    pub id: String,
    pub seats: Vec<PlazaSeat>,
    pub messages: Vec<Message>,
    pub speaker: Option<Speaker>,
    pub last_route: Option<RouteDecision>,
    pub pins: Vec<Pin>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ModeRef {
    Chat(String),
    Quick(String),
    Plaza(String),
}
pub trait ModeEngine: Send + Sync + 'static {
    fn chat_send(&self, thread: &str, text: &str) -> Res<()>;
    fn chat(&self, thread: &str) -> ChatThread;
    fn quick_start(&self, r: &QuickRequest) -> Res<String>;
    fn quick(&self, id: &str) -> Option<QuickTask>;
    fn plaza_send(&self, room: &str, ask: &PlazaAsk) -> Res<RouteDecision>;
    fn plaza(&self, room: &str) -> PlazaRoom;
    fn pin(&self, room: &str, message: u64) -> Res<()>;
    fn handoff(&self, from: &ModeRef) -> Res<String>;
}
#[derive(Default)]
struct State {
    calls: usize,
    next: u64,
    chats: HashMap<String, ChatThread>,
    tasks: HashMap<String, QuickTask>,
    rooms: HashMap<String, PlazaRoom>,
}
#[derive(Default)]
pub struct FakeModes {
    state: Mutex<State>,
}
fn room(id: &str) -> PlazaRoom {
    PlazaRoom {
        id: id.into(),
        seats: [
            Speaker::Core,
            Speaker::Astra,
            Speaker::Fable,
            Speaker::Argon,
        ]
        .into_iter()
        .map(|member| PlazaSeat {
            member,
            budget_usd: Some(5.0),
            spent_usd: Some(0.0),
        })
        .collect(),
        ..Default::default()
    }
}
impl FakeModes {
    pub fn new() -> Self {
        let mut s = State::default();
        s.rooms.insert("plaza".into(), room("plaza"));
        Self {
            state: Mutex::new(s),
        }
    }
    pub fn calls(&self) -> usize {
        self.state.lock().unwrap().calls
    }
    pub fn set_seats(&self, id: &str, seats: Vec<PlazaSeat>) {
        self.state
            .lock()
            .unwrap()
            .rooms
            .entry(id.into())
            .or_insert_with(|| room(id))
            .seats = seats;
    }
    pub fn step_quick(&self, id: &str) {
        if let Some(t) = self.state.lock().unwrap().tasks.get_mut(id) {
            t.step = match t.step {
                QuickStep::Queued => QuickStep::Building,
                QuickStep::Building => QuickStep::Verifying,
                QuickStep::Verifying if t.request.review => QuickStep::Reviewing,
                QuickStep::Verifying | QuickStep::Reviewing => QuickStep::Done {
                    summary: "Changes verified.".into(),
                },
                ref x => x.clone(),
            };
        }
    }
}
impl ModeEngine for FakeModes {
    fn chat_send(&self, id: &str, text: &str) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        s.next += 1;
        let n = s.next;
        let thread = s.chats.entry(id.into()).or_insert_with(|| ChatThread {
            id: id.into(),
            ..Default::default()
        });
        thread.messages.push(Message {
            id: n * 2,
            from: Speaker::You,
            text: text.into(),
            at_ms: 0,
        });
        thread.messages.push(Message {
            id: n * 2 + 1,
            from: Speaker::Core,
            text: "I can help examine the request and identify the next step.".into(),
            at_ms: 0,
        });
        Ok(())
    }
    fn chat(&self, id: &str) -> ChatThread {
        self.state
            .lock()
            .unwrap()
            .chats
            .get(id)
            .cloned()
            .unwrap_or_else(|| ChatThread {
                id: id.into(),
                ..Default::default()
            })
    }
    fn quick_start(&self, r: &QuickRequest) -> Res<String> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        s.next += 1;
        let id = format!("quick-{}", s.next);
        s.tasks.insert(
            id.clone(),
            QuickTask {
                id: id.clone(),
                request: r.clone(),
                step: QuickStep::Queued,
                unit: None,
            },
        );
        Ok(id)
    }
    fn quick(&self, id: &str) -> Option<QuickTask> {
        self.state.lock().unwrap().tasks.get(id).cloned()
    }
    fn plaza_send(&self, id: &str, ask: &PlazaAsk) -> Res<RouteDecision> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        s.next += 1;
        let n = s.next;
        let r = s.rooms.entry(id.into()).or_insert_with(|| room(id));
        let (text, requested, why) = match ask {
            PlazaAsk::Say { text, mentions } if !mentions.is_empty() => {
                (text.clone(), mentions.clone(), "Selected mentioned members")
            }
            PlazaAsk::Say { text, .. } | PlazaAsk::Round { text } => {
                let t = text.to_lowercase();
                let picked = if t.contains("review") || t.contains("risk") {
                    vec![Speaker::Astra, Speaker::Fable]
                } else if t.contains("code") || t.contains("build") {
                    vec![Speaker::Argon, Speaker::Core]
                } else {
                    vec![Speaker::Core]
                };
                (text.clone(), picked, "Selected members by topic")
            }
            PlazaAsk::Debate { text, a, b, judge } => (
                text.clone(),
                vec![*a, *b, *judge],
                "Selected debate participants",
            ),
            PlazaAsk::Vote { question } => (
                question.clone(),
                vec![Speaker::Astra, Speaker::Fable, Speaker::Argon],
                "Selected council members for vote",
            ),
        };
        let mut picked = vec![];
        let mut skipped = false;
        for member in requested {
            if picked.len() == 3 {
                break;
            }
            if picked.contains(&member) || member == Speaker::You {
                continue;
            }
            if let Some(seat) = r.seats.iter().find(|x| x.member == member) {
                if matches!((seat.budget_usd,seat.spent_usd),(Some(b),Some(sp)) if sp>=b) {
                    skipped = true;
                    continue;
                }
                picked.push(member);
            }
        }
        let decision = RouteDecision {
            picked,
            reason: if skipped {
                format!("{why}; over-budget member skipped")
            } else {
                why.into()
            },
        };
        r.messages.push(Message {
            id: n * 10,
            from: Speaker::You,
            text,
            at_ms: 0,
        });
        for (index, member) in decision.picked.iter().enumerate() {
            let reply = match member {
                Speaker::Core => "I will outline the next step and its dependencies.",
                Speaker::Astra => "I will check the assumptions and identify risks.",
                Speaker::Fable => "I will review the evidence and acceptance criteria.",
                Speaker::Argon => "I will examine the implementation and verification plan.",
                Speaker::You => unreachable!(),
            };
            r.messages.push(Message {
                id: n * 10 + index as u64 + 1,
                from: *member,
                text: reply.into(),
                at_ms: 0,
            });
        }
        r.last_route = Some(decision.clone());
        Ok(decision)
    }
    fn plaza(&self, id: &str) -> PlazaRoom {
        self.state
            .lock()
            .unwrap()
            .rooms
            .get(id)
            .cloned()
            .unwrap_or_else(|| room(id))
    }
    fn pin(&self, id: &str, message: u64) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        let r = s
            .rooms
            .get_mut(id)
            .ok_or_else(|| Failure::new("not_found", "Room not found"))?;
        let m = r
            .messages
            .iter()
            .find(|m| m.id == message)
            .ok_or_else(|| Failure::new("not_found", "Message not found"))?;
        r.pins.push(Pin {
            text: m.text.clone(),
            sources: vec![message],
        });
        Ok(())
    }
    fn handoff(&self, from: &ModeRef) -> Res<String> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        match from {
            ModeRef::Chat(id) => s.chats.get(id).map(|x| {
                x.messages
                    .iter()
                    .map(|m| m.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            ModeRef::Quick(id) => s.tasks.get(id).map(|x| x.request.text.clone()),
            ModeRef::Plaza(id) => s.rooms.get(id).map(|x| {
                x.messages
                    .iter()
                    .map(|m| m.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        }
        .ok_or_else(|| Failure::new("not_found", "Mode not found"))
    }
}
