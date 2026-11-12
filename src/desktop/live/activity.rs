#![allow(dead_code)]
use crate::{
    base::Res,
    desktop::{
        i18n::Lang,
        model::{Phase, Run, RunStatus, UnitState},
        ux::{
            activity::{OpenActivity, Projection, Pulse, LOST_AFTER_MS},
            celestial::{CouncilView, Member, Seat},
        },
    },
    orch::events::{self, Event},
};
use gpui_kit::SharedString;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub trait ActivitySource: Send + Sync {
    fn events(&self, store: &Path, after_seq: u64) -> Res<Vec<Event>>;
}
pub struct OrchActivitySource;
impl ActivitySource for OrchActivitySource {
    fn events(&self, store: &Path, after_seq: u64) -> Res<Vec<Event>> {
        events::read_events(store, after_seq)
    }
}
#[derive(Default)]
pub struct FakeActivity(Mutex<HashMap<PathBuf, Vec<Event>>>);
impl FakeActivity {
    pub fn push(&self, store: &Path, event: Event) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(store.into())
            .or_default()
            .push(event);
    }
}
impl ActivitySource for FakeActivity {
    fn events(&self, store: &Path, after_seq: u64) -> Res<Vec<Event>> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(store)
            .into_iter()
            .flatten()
            .filter(|e| e.seq > after_seq)
            .cloned()
            .collect())
    }
}

fn set(items: &mut Vec<SharedString>, value: &str, present: bool) {
    if present {
        if !items.iter().any(|v| v.as_ref() == value) {
            items.push(value.to_owned().into());
        }
    } else {
        items.retain(|v| v.as_ref() != value);
    }
}
impl Projection {
    pub fn apply(&mut self, events: &[Event]) -> bool {
        let mut changed = false;
        for e in events {
            if e.seq <= self.last_seq {
                continue;
            }
            changed = true;
            self.last_seq = e.seq;
            self.last_seen_ms = Some(self.last_seen_ms.map_or(e.ts_ms, |v| v.max(e.ts_ms)));
            if e.kind != "heartbeat" {
                self.last_activity_ms =
                    Some(self.last_activity_ms.map_or(e.ts_ms, |v| v.max(e.ts_ms)));
            }
            match e.kind.as_str() {
                "activity_start" => {
                    if let (Some(id), Some(module), Some(actor), Some(phase)) = (
                        e.data["activity"].as_str(),
                        e.module.as_deref(),
                        e.data["actor"].as_str(),
                        e.data["phase"].as_str(),
                    ) {
                        self.open.retain(|a| a.id.as_ref() != id);
                        self.open.push(OpenActivity {
                            id: id.to_owned().into(),
                            module: module.to_owned().into(),
                            actor: actor.to_owned().into(),
                            phase: phase.to_owned().into(),
                            since_ms: e.ts_ms,
                        });
                    }
                }
                "activity_finish" => {
                    if let Some(id) = e.data["activity"].as_str() {
                        self.open.retain(|a| a.id.as_ref() != id);
                    }
                }
                "module_claimed" | "module_released" | "module_verified" => {
                    if let Some(m) = e.module.as_deref() {
                        set(&mut self.claimed, m, e.kind == "module_claimed");
                    }
                }
                "unit_state" => {
                    if let Some(u) = e.unit.as_deref() {
                        set(&mut self.running_units, u, e.data["state"] == "running");
                    }
                }
                _ => {}
            }
        }
        changed
    }
    pub fn pulse(&self, now_ms: i64) -> Pulse {
        match self.last_seen_ms {
            None => Pulse::Unknown,
            Some(t) if now_ms.saturating_sub(t) > LOST_AFTER_MS => Pulse::Lost { last_seen_ms: t },
            Some(_) => Pulse::Live {
                last_activity_ms: self.last_activity_ms,
            },
        }
    }
    pub fn claude_side(&self) -> Option<&OpenActivity> {
        self.open
            .iter()
            .rev()
            .find(|a| matches!(a.actor.as_ref(), "core" | "dispatcher"))
    }
}
pub fn heartbeat_text(p: Pulse, now_ms: i64, lang: Lang) -> String {
    let age = |t: i64| now_ms.saturating_sub(t).max(0) / 1000;
    match (p, lang) {
        (Pulse::Unknown, Lang::En) => "Last activity: unknown".into(),
        (Pulse::Unknown, Lang::ZhTw) => "最後活動：未知".into(),
        (
            Pulse::Live {
                last_activity_ms: None,
            },
            Lang::En,
        ) => "Last activity: none yet".into(),
        (
            Pulse::Live {
                last_activity_ms: None,
            },
            Lang::ZhTw,
        ) => "最後活動：尚無".into(),
        (
            Pulse::Live {
                last_activity_ms: Some(t),
            },
            l,
        ) => {
            let s = age(t);
            match (s >= 120, l) {
                (false, Lang::En) => format!("Last activity: {s} s ago"),
                (false, Lang::ZhTw) => format!("最後活動：{s} 秒前"),
                (true, Lang::En) => format!("Last activity: {} min ago", s / 60),
                (true, Lang::ZhTw) => format!("最後活動：{} 分鐘前", s / 60),
            }
        }
        (Pulse::Lost { last_seen_ms: t }, Lang::En) => format!("No signal for {} min", age(t) / 60),
        (Pulse::Lost { last_seen_ms: t }, Lang::ZhTw) => format!("已 {} 分鐘沒有訊號", age(t) / 60),
    }
}
pub struct LiveStage {
    pub name: SharedString,
    pub actor: SharedString,
    pub since_ms: Option<i64>,
}
pub fn live_stage(run: &Run, p: &Projection, now_ms: i64, lang: Lang) -> Option<LiveStage> {
    let running = run.stages.iter().find(|s| s.phase == Phase::Running);
    let stage = running.or_else(|| {
        if run.status != RunStatus::Active || !matches!(p.pulse(now_ms), Pulse::Live { .. }) {
            return None;
        }
        let start = run
            .stages
            .iter()
            .rposition(|s| s.phase == Phase::Done)
            .map_or(0, |i| i + 1);
        run.stages[start..]
            .iter()
            .find(|s| s.phase == Phase::Waiting)
    })?;
    let name: SharedString = if running.is_none() {
        match lang {
            Lang::En => "Astra planning",
            Lang::ZhTw => "Astra 規劃中",
        }
        .into()
    } else if lang == Lang::ZhTw {
        crate::desktop::i18n::COMMON
            .iter()
            .find(|(en, _)| *en == stage.name.as_ref())
            .map_or_else(|| stage.name.clone(), |(_, zh)| (*zh).into())
    } else {
        stage.name.clone()
    };
    let actor = if running.is_none() {
        "Astra"
    } else {
        match stage.name.as_ref() {
            "Critic" => "Critic",
            "Architect" => "Architect",
            "Build" => "Codex",
            _ => "Astra",
        }
    };
    Some(LiveStage {
        name,
        actor: crate::desktop::i18n::translate(lang, crate::desktop::i18n::COMMON, actor)
            .to_owned()
            .into(),
        since_ms: p.claude_side().map(|a| a.since_ms),
    })
}
pub fn activity_text(a: &OpenActivity, lang: Lang) -> String {
    let actor = match (a.actor.as_ref(), lang) {
        ("core", Lang::En) => "Core",
        ("core", Lang::ZhTw) => "主控",
        ("dispatcher", Lang::En) => "Dispatcher",
        ("dispatcher", Lang::ZhTw) => "調度員",
        (v, _) => v,
    };
    let phase = match (a.phase.as_ref(), lang) {
        ("brief", Lang::En) => "writing the brief",
        ("brief", Lang::ZhTw) => "撰寫任務說明中",
        ("review", Lang::En) => "reviewing",
        ("review", Lang::ZhTw) => "審查中",
        ("decide", Lang::En) => "deciding",
        ("decide", Lang::ZhTw) => "決策中",
        (v, _) => v,
    };
    format!("{actor} · {phase} · {}", a.module)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerKind {
    Claude,
    Codex,
    Council,
}
pub struct Worker {
    pub kind: WorkerKind,
    pub who: SharedString,
    pub what: SharedString,
    pub since_ms: Option<i64>,
    pub unit: Option<SharedString>,
}
pub fn workers(run: &Run, p: &Projection, council: Option<&CouncilView>) -> Vec<Worker> {
    let lang = crate::desktop::i18n::data_lang();
    let mut out = Vec::new();
    if let Some(a) = p.claude_side() {
        out.push(Worker {
            kind: WorkerKind::Claude,
            who: "Claude".into(),
            what: activity_text(a, lang).into(),
            since_ms: Some(a.since_ms),
            unit: None,
        });
    }
    for id in &p.running_units {
        let what = run
            .units
            .iter()
            .find(|u| u.id == *id)
            .map_or_else(|| id.clone(), |u| u.title.clone());
        out.push(Worker {
            kind: WorkerKind::Codex,
            who: "Codex".into(),
            what,
            since_ms: None,
            unit: Some(id.clone()),
        });
    }
    for unit in run
        .units
        .iter()
        .filter(|u| u.state == UnitState::Running && !p.running_units.contains(&u.id))
    {
        out.push(Worker {
            kind: WorkerKind::Codex,
            who: "Codex".into(),
            what: unit.title.clone(),
            since_ms: None,
            unit: Some(unit.id.clone()),
        });
    }
    if let Some(c) = council.filter(|c| c.active) {
        for (member, seat) in &c.seats {
            if *seat == Seat::Speaking {
                let who = match member {
                    Member::Astra => "Astra",
                    Member::Fable => "Fable",
                    Member::Argon => "Argon",
                };
                out.push(Worker {
                    kind: WorkerKind::Council,
                    who: who.into(),
                    what: c.phase.clone(),
                    since_ms: None,
                    unit: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(seq: u64, ts: i64, kind: &str, data: serde_json::Value) -> Event {
        Event {
            seq,
            ts: String::new(),
            ts_ms: ts,
            kind: kind.into(),
            wish: "w".into(),
            module: Some("m".into()),
            unit: Some("u".into()),
            data,
        }
    }
    #[test]
    fn replay_and_poll() {
        let dir = tempfile::tempdir().unwrap();
        let source = FakeActivity::default();
        let mut p = Projection::default();
        assert_eq!(p.pulse(0), Pulse::Unknown);
        let rows = vec![
            event(1, 1000, "module_claimed", json!({})),
            event(
                2,
                2000,
                "activity_start",
                json!({"activity":"a","actor":"core","phase":"brief"}),
            ),
            event(3, 3000, "activity_finish", json!({"activity":"a"})),
            event(4, 4000, "unit_state", json!({"state":"running"})),
            event(5, 5000, "unit_state", json!({"state":"built"})),
            event(
                6,
                6000,
                "activity_start",
                json!({"activity":"b","actor":"dispatcher","phase":"review"}),
            ),
            event(7, 7000, "heartbeat", json!({})),
        ];
        for e in &rows {
            source.push(dir.path(), e.clone());
        }
        assert!(p.apply(&source.events(dir.path(), 0).unwrap()));
        assert_eq!(p.last_activity_ms, Some(6000));
        assert_eq!(p.claude_side().unwrap().id.as_ref(), "b");
        assert!(p.running_units.is_empty());
        assert!(!p.apply(&rows));
        assert!(source.events(dir.path(), p.last_seq).unwrap().is_empty());
        assert!(!p.apply(&[event(2, 999999, "heartbeat", json!({}))]));
        assert_eq!(p.last_activity_ms, Some(6000));
        assert!(matches!(p.pulse(7000), Pulse::Live { .. }));
        assert!(matches!(p.pulse(127001), Pulse::Lost { .. }));
        assert_eq!(
            heartbeat_text(p.pulse(7000), 7000, Lang::ZhTw),
            "最後活動：1 秒前"
        );
        assert!(p.apply(&[event(8, 100, "module_released", json!({}))]));
        assert!(p.claimed.is_empty());
        assert_eq!(
            (p.last_seen_ms, p.last_activity_ms),
            (Some(7000), Some(6000))
        );
    }
    #[test]
    fn stage_workers_and_heartbeat() {
        let mut run = crate::desktop::demo::sample_run();
        run.status = RunStatus::Active;
        for stage in &mut run.stages {
            stage.phase = Phase::Waiting;
        }
        run.stages[0].phase = Phase::Done;
        let mut p = Projection::default();
        p.apply(&[event(1, 1000, "heartbeat", json!({}))]);
        assert_eq!(
            heartbeat_text(p.pulse(1000), 1000, Lang::En),
            "Last activity: none yet"
        );
        assert_eq!(
            live_stage(&run, &p, 1000, Lang::ZhTw)
                .unwrap()
                .name
                .as_ref(),
            "Astra 規劃中"
        );
        assert!(live_stage(&run, &p, 200000, Lang::En).is_none());
        run.stages[3].phase = Phase::Running;
        assert_eq!(
            live_stage(&run, &p, 1000, Lang::ZhTw)
                .unwrap()
                .name
                .as_ref(),
            "施工"
        );
        p.apply(&[
            event(
                2,
                2000,
                "activity_start",
                json!({"activity":"a","actor":"core","phase":"brief"}),
            ),
            event(3, 3000, "unit_state", json!({"state":"running"})),
        ]);
        assert_eq!(
            activity_text(p.claude_side().unwrap(), Lang::ZhTw),
            "主控 · 撰寫任務說明中 · m"
        );
        let workers = workers(&run, &p, None);
        assert!(workers.iter().any(|w| w.kind == WorkerKind::Claude));
        assert!(workers
            .iter()
            .any(|w| w.kind == WorkerKind::Codex && w.unit.as_deref() == Some("u")));
        assert_eq!(
            heartbeat_text(
                Pulse::Live {
                    last_activity_ms: Some(0)
                },
                120000,
                Lang::En
            ),
            "Last activity: 2 min ago"
        );
    }
}
