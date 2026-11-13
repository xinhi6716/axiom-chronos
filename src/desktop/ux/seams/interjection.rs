use crate::base::{Failure, Res};
use crate::desktop::data::outbox::{Outbox, Outcome};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub session: String,
    pub client_id: String,
    pub text: String,
    pub pause_first: bool,
    pub replan: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptStage {
    Sent,
    Received,
    Applied { first_action: String },
    Rejected { reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    pub client_id: String,
    pub session: String,
    pub stage: ReceiptStage,
    pub at_ms: i64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Amendment {
    pub client_id: Option<String>,
    pub text: String,
    pub at_ms: Option<i64>,
    pub stage: Option<ReceiptStage>,
    pub affected: Vec<String>,
    pub replan: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunningWork {
    pub stage: Option<String>,
    pub units: Vec<String>,
    pub batch: Option<String>,
}
pub trait Interjections: Send + Sync + 'static {
    fn send(&self, d: &Draft) -> Res<Receipt>;
    fn receipt(&self, client_id: &str) -> Option<Receipt>;
    fn history(&self, session: &str) -> Vec<Amendment>;
    fn running_work(&self, session: &str) -> RunningWork;
    fn pause_batch(&self, session: &str) -> Res<()>;
}
pub fn interjection_outbox_for(service: Arc<dyn Interjections>) -> Outbox<Draft> {
    Outbox::new(move |draft: &Draft, key| {
        if draft.client_id != key {
            return Outcome::Refused("idempotency key does not match client id".into());
        }
        match service.send(draft) {
            Ok(_) => Outcome::Applied,
            Err(error) => super::approvals::outbox_failure(error),
        }
    })
}
#[derive(Default)]
struct State {
    receipts: HashMap<String, Receipt>,
    history: HashMap<String, Vec<Amendment>>,
    sent: Vec<Draft>,
    pauses: usize,
    fail: bool,
    work: HashMap<String, RunningWork>,
}
#[derive(Default)]
pub struct FakeInterjections {
    state: Mutex<State>,
    path: Option<PathBuf>,
}
fn stage_json(s: &ReceiptStage) -> serde_json::Value {
    match s {
        ReceiptStage::Sent => serde_json::json!({"type":"sent"}),
        ReceiptStage::Received => serde_json::json!({"type":"received"}),
        ReceiptStage::Applied { first_action } => {
            serde_json::json!({"type":"applied","first_action":first_action})
        }
        ReceiptStage::Rejected { reason } => serde_json::json!({"type":"rejected","reason":reason}),
    }
}
fn stage_parse(v: &serde_json::Value) -> Option<ReceiptStage> {
    Some(match v.get("type")?.as_str()? {
        "sent" => ReceiptStage::Sent,
        "received" => ReceiptStage::Received,
        "applied" => ReceiptStage::Applied {
            first_action: v.get("first_action")?.as_str()?.into(),
        },
        "rejected" => ReceiptStage::Rejected {
            reason: v.get("reason")?.as_str()?.into(),
        },
        _ => return None,
    })
}
impl FakeInterjections {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn at(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let mut s = State::default();
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(rows) = v.as_array() {
                    for row in rows {
                        let parsed = (|| {
                            let session = row.get("session")?.as_str()?.to_owned();
                            let id = row.get("client_id")?.as_str()?.to_owned();
                            let text = row.get("text")?.as_str()?.to_owned();
                            let at = row.get("at_ms")?.as_i64()?;
                            let stage = stage_parse(row.get("stage")?)?;
                            let replan = row.get("replan")?.as_bool()?;
                            Some((session, id, text, at, stage, replan))
                        })();
                        if let Some((session, id, text, at, stage, replan)) = parsed {
                            s.receipts.insert(
                                id.clone(),
                                Receipt {
                                    client_id: id.clone(),
                                    session: session.clone(),
                                    stage: stage.clone(),
                                    at_ms: at,
                                },
                            );
                            s.history.entry(session).or_default().push(Amendment {
                                client_id: Some(id),
                                text,
                                at_ms: Some(at),
                                stage: Some(stage),
                                affected: vec![],
                                replan,
                            });
                        }
                    }
                }
            }
        }
        Self {
            state: Mutex::new(s),
            path: Some(path),
        }
    }
    fn save(&self, s: &State) -> Res<()> {
        if let Some(path) = &self.path {
            let rows:Vec<_>=s.history.iter().flat_map(|(session,items)| items.iter().filter_map(move |a| Some(serde_json::json!({"session":session,"client_id":a.client_id.as_ref()?,"text":a.text,"at_ms":a.at_ms?,"stage":stage_json(a.stage.as_ref()?),"replan":a.replan})))).collect();
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| Failure::new("io", e.to_string()))?;
            }
            std::fs::write(path, serde_json::to_vec(&rows).unwrap())
                .map_err(|e| Failure::new("io", e.to_string()))?;
        }
        Ok(())
    }
    pub fn advance(&self, id: &str, stage: ReceiptStage) {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.receipts.get_mut(id) {
            r.stage = stage.clone();
        }
        for a in s
            .history
            .values_mut()
            .flatten()
            .filter(|a| a.client_id.as_deref() == Some(id))
        {
            a.stage = Some(stage.clone());
        }
        let _ = self.save(&s);
    }
    pub fn set_pause_failure(&self, v: bool) {
        self.state.lock().unwrap().fail = v;
    }
    pub fn sent(&self) -> Vec<Draft> {
        self.state.lock().unwrap().sent.clone()
    }
    pub fn pauses(&self) -> usize {
        self.state.lock().unwrap().pauses
    }
    pub fn set_work(&self, session: &str, work: RunningWork) {
        self.state.lock().unwrap().work.insert(session.into(), work);
    }
}
impl Interjections for FakeInterjections {
    fn send(&self, d: &Draft) -> Res<Receipt> {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.receipts.get(&d.client_id) {
            return Ok(r.clone());
        }
        let at = super::super::wall_ms();
        let r = Receipt {
            client_id: d.client_id.clone(),
            session: d.session.clone(),
            stage: ReceiptStage::Sent,
            at_ms: at,
        };
        s.receipts.insert(d.client_id.clone(), r.clone());
        s.history
            .entry(d.session.clone())
            .or_default()
            .push(Amendment {
                client_id: Some(d.client_id.clone()),
                text: d.text.clone(),
                at_ms: Some(at),
                stage: Some(r.stage.clone()),
                affected: vec![],
                replan: d.replan,
            });
        s.sent.push(d.clone());
        self.save(&s)?;
        Ok(r)
    }
    fn receipt(&self, id: &str) -> Option<Receipt> {
        self.state.lock().unwrap().receipts.get(id).cloned()
    }
    fn history(&self, session: &str) -> Vec<Amendment> {
        self.state
            .lock()
            .unwrap()
            .history
            .get(session)
            .cloned()
            .unwrap_or_default()
    }
    fn running_work(&self, session: &str) -> RunningWork {
        self.state
            .lock()
            .unwrap()
            .work
            .get(session)
            .cloned()
            .unwrap_or_default()
    }
    fn pause_batch(&self, _: &str) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        s.pauses += 1;
        if s.fail {
            Err(Failure::new("pause_failed", "Batch could not be paused"))
        } else {
            Ok(())
        }
    }
}
