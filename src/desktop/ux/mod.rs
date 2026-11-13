pub mod activity;
pub mod celestial;
pub mod composer;
pub mod events;
pub mod prefs;
pub mod real;
pub mod seams;
pub mod usage;
use super::{
    model::{AppState, Run, RunStatus, UnitState},
    settings::Store,
};
pub use activity::*;
pub use celestial::*;
pub use events::*;
use crate::desktop::data::outbox::Outbox;
use gpui_kit::{Context, SharedString};
use seams::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
pub use usage::*;
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    // --- research workspace (phase 9) ---
    Research,
    // --- end research workspace (phase 9) ---
    Storage,
    Chat,
    QuickTask,
    Plaza,
    Session(SharedString),
    // p8: begin stub
    Paper(SharedString),
    Routine,
    // p8: end stub
}
impl Page { pub fn kind(&self) -> &'static str { match self { Self::Research => "research", Self::Storage => "storage", Self::Chat => "chat", Self::QuickTask => "quick", Self::Plaza => "plaza", Self::Session(_) => "session", Self::Paper(_) => "paper", Self::Routine => "routine" } } }
impl Page {
    pub const KINDS: &'static [&'static str] = &[
        "research", "storage", "chat", "quick", "plaza", "session", "paper", "routine",
    ];

    pub fn needs_subject(kind: &str) -> bool {
        matches!(kind, "session" | "paper")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadFilter {
    #[default]
    All,
    Unread,
    Archived,
}
#[derive(Clone, Default)]
pub struct Seams {
    pub dual_core: Option<Arc<dyn DualCoreSource>>,
    pub turn_policy: Option<Arc<dyn TurnPolicy>>,
    pub interjections: Option<Arc<dyn Interjections>>,
    pub approvals: Option<Arc<dyn ApprovalService>>,
    pub approval_outbox: Option<Outbox<Decision>>,
    pub interjection_outbox: Option<Outbox<Draft>>,
    pub unit_action_outbox: Option<Outbox<UnitAction>>,
    pub storage: Option<Arc<dyn StorageService>>,
    pub background: Option<Arc<dyn BackgroundService>>,
    pub modes: Option<Arc<dyn ModeEngine>>,
    pub sessions: Option<Arc<dyn SessionStream>>,
    pub unit_actions: Option<Arc<dyn UnitActions>>,
    pub optimizer: Option<Arc<dyn OptimizerRunner>>,
    pub github: Option<Arc<dyn GhRunner>>,
    pub routines: Option<Arc<dyn RoutineSeam>>,
    pub paper: Option<Arc<dyn PaperService>>, // p8:
}
impl Seams {
    pub fn demo() -> Self {
        Self::for_runs(&[super::demo::sample_run()])
    }
    fn for_runs(runs: &[Run]) -> Self {
        let dual = FakeDualCore::new();
        let interjections = FakeInterjections::new();
        let units = FakeUnitActions::new();
        let mut cards = vec![];
        let mut sessions = vec![];
        if let Some(run) = runs
            .iter()
            .find(|r| r.status == RunStatus::Active)
            .or_else(|| runs.first())
        {
            dual.set(
                &run.session,
                DualCore::Agreed {
                    topic: "Review the reader pipeline".into(),
                },
            );
            let _ = interjections.send(&Draft {
                session: run.session.to_string(),
                client_id: "demo-amendment".into(),
                text: "Keep source PDFs local during analysis.".into(),
                pause_first: false,
                replan: true,
            });
            if let Some(unit) = run.units.iter().find(|u| u.state == UnitState::Running) {
                sessions.push(WorkerSession {
                    id: "worker-terminal".into(),
                    run: run.session.to_string(),
                    unit: Some(unit.id.to_string()),
                    title: "Reader worker terminal".into(),
                    kind: SessionKind::Terminal,
                });
                interjections.set_work(
                    &run.session,
                    RunningWork {
                        stage: Some("Building".into()),
                        units: vec![unit.id.to_string()],
                        batch: None,
                    },
                );
            }
            cards.push(ApprovalCard {
                action_id: "publish-report".into(),
                version: 1,
                session: run.session.to_string(),
                action: "Publish".into(),
                target: "Research report".into(),
                scope: "Report files only".into(),
                cost_usd: None,
            });
        }
        for run in runs {
            for unit in &run.units {
                units.set_evidence(
                    &run.session,
                    &unit.id,
                    UnitEvidence {
                        failure: (unit.state == UnitState::Failed).then(|| {
                            "Verification failed; inspect the causal model report.".into()
                        }),
                        ..Default::default()
                    },
                );
            }
        }
        let items = [
            ("cache", "cache", "Page image cache"),
            ("logs", "logs", "Worker logs"),
            ("artifacts", "artifacts", "Build artifacts"),
            ("sessions", "sessions", "Session transcripts"),
        ]
        .into_iter()
        .map(|(id, category, path)| StorageItem {
            id: id.into(),
            category: category.into(),
            project: runs.first().map(|r| r.project.to_string()),
            wish: runs.first().map(|r| r.session.to_string()),
            path: path.into(),
            bytes: 1024,
            measured: true,
            reclaimable: true,
        })
        .collect();
        let interjections = Arc::new(interjections);
        let approvals = Arc::new(FakeApprovals::new(cards));
        let units = Arc::new(units);
        Self {
            dual_core: Some(Arc::new(dual)),
            turn_policy: Some(Arc::new(FakeTurnPolicy::new(Some(FastCost {
                multiplier: 2.0,
            })))),
            interjections: Some(interjections.clone()),
            approvals: Some(approvals.clone()),
            approval_outbox: Some(approval_outbox_for(approvals)),
            interjection_outbox: Some(interjection_outbox_for(interjections)),
            unit_action_outbox: Some(unit_action_outbox_for(units.clone())),
            storage: Some(Arc::new(FakeStorage::new(
                items,
                Some(DiskSpace {
                    free_bytes: 100_000_000_000,
                    total_bytes: 500_000_000_000,
                }),
            ))),
            background: Some(Arc::new(FakeBackground::new(true))),
            modes: Some(Arc::new(FakeModes::new())),
            sessions: Some(Arc::new(FakeSessions::new(sessions))),
            unit_actions: Some(units),
            optimizer: Some(Arc::new(FakeOptimizer::new(vec![Ok(Proposal {
                goal: "Produce a verified research report".into(),
                scope: "Reader and analysis pipeline".into(),
                limits: "Keep source papers local".into(),
                acceptance: "All verification checks pass".into(),
                ambiguities: vec![],
            })]))),
            github: Some(Arc::new(FakeGh::new(vec![GhOutcome::Exit {
                code: 0,
                stdout: "Repository is ready".into(),
                stderr: String::new(),
            }]))),
            routines: None,
            paper: None, // p8:
        }
    }
}
#[derive(Default)]
pub struct UxState {
    // --- research workspace (phase 9) ---
    pub research: crate::desktop::research::adapter::ResearchSeams,
    // --- end research workspace (phase 9) ---
    pub seams: Seams,
    pub page: Option<Page>,
    pub activity: HashMap<SharedString, Projection>,
    pub notices: HashMap<SharedString, Vec<Notice>>,
    pub council: HashMap<SharedString, CouncilView>,
    pub usage_sessions: Vec<UsageSession>,
    pub wish_ids: HashMap<SharedString, SharedString>,
    pub amendments: HashMap<SharedString, Vec<SharedString>>,
    pub approvals: Book,
    pub disk: Option<DiskSpace>,
    pub extreme_flash: Option<Instant>,
    pub composer_prefill: Option<String>,
    pub clock_ms: Option<i64>,
}
pub(crate) fn wall_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
impl UxState {
    pub fn now_ms(&self) -> i64 {
        self.clock_ms.unwrap_or_else(wall_ms)
    }
    pub fn demo(runs: &[Run], now_ms: i64) -> Self {
        let seams = Seams::for_runs(runs);
        let mut state = Self {
            disk: seams.storage.as_ref().and_then(|s| s.space()),
            seams,
            clock_ms: Some(now_ms),
            ..Default::default()
        };
        if let Some(run) = runs
            .iter()
            .find(|r| r.status == RunStatus::Active)
            .or_else(|| runs.first())
        {
            if let Some(module) = run.modules.first() {
                state.activity.insert(
                    run.session.clone(),
                    Projection {
                        last_seq: 1,
                        last_activity_ms: Some(now_ms - 12_000),
                        last_seen_ms: Some(now_ms - 5_000),
                        open: vec![OpenActivity {
                            id: "review-1".into(),
                            module: module.id.clone(),
                            actor: "core".into(),
                            phase: "review".into(),
                            since_ms: now_ms - 12_000,
                        }],
                        running_units: run
                            .units
                            .iter()
                            .filter(|u| u.state == UnitState::Running)
                            .map(|u| u.id.clone())
                            .collect(),
                        ..Default::default()
                    },
                );
            }
            state.council.insert(
                run.session.clone(),
                CouncilView {
                    phase: "review".into(),
                    agreed: 2,
                    majority: 2,
                    disputed: 1,
                    ..Default::default()
                },
            );
            state.amendments.insert(
                run.session.clone(),
                vec!["Keep source PDFs local during analysis.".into()],
            );
        }
        if let Some(run) = runs.iter().find(|r| r.status == RunStatus::Blocked) {
            state
                .notices
                .entry(run.session.clone())
                .or_default()
                .extend([
                    Notice {
                        key: "max-turns".into(),
                        kind: NoticeKind::MaxTurns {
                            turn_limit: Some(48),
                        },
                        at_ms: Some(now_ms - 60_000),
                    },
                    Notice {
                        key: "scope-failure".into(),
                        kind: NoticeKind::ScopeFailure {
                            unit: run
                                .units
                                .first()
                                .map(|u| u.id.to_string())
                                .unwrap_or_default(),
                            allowed: vec!["src".into()],
                            outside: vec!["docs/report.md".into()],
                            outside_count: 1,
                            ignored: vec![],
                        },
                        at_ms: Some(now_ms - 50_000),
                    },
                ]);
        }
        if let Some(run) = runs.iter().find(|r| r.status == RunStatus::Done) {
            state
                .notices
                .entry(run.session.clone())
                .or_default()
                .push(Notice {
                    key: "council-degraded".into(),
                    kind: NoticeKind::DegradedCouncil {
                        member: "Fable".into(),
                        phase: "review".into(),
                        round: 1,
                        status: "unavailable".into(),
                        reason: "Review timed out".into(),
                    },
                    at_ms: Some(now_ms - 100_000),
                });
        }
        for (i, run) in runs.iter().take(3).enumerate() {
            let wish = if i == 1 {
                runs[0].session.clone()
            } else {
                run.session.clone()
            };
            state.wish_ids.insert(run.session.clone(), wish.clone());
            state.usage_sessions.push(UsageSession {
                wish,
                session: run.session.clone(),
                label: run.wish.clone(),
                usage: run.usage.clone(),
                complete: run.status == RunStatus::Done,
                started_at: run.started_at,
            });
        }
        state
    }
}
impl AppState {
    fn select_page(&mut self, p: Option<Page>) {
        self.ux.page = p;
    }
    fn select_mode(&mut self, m: ModeKind, carry: Option<String>, prefs: &mut prefs::UxPrefs) {
        prefs.last_mode = m;
        match m {
            ModeKind::Wish => {
                self.select_page(None);
                self.ux.composer_prefill = carry;
                self.composing = true;
            }
            ModeKind::Chat => self.select_page(Some(Page::Chat)),
            ModeKind::QuickTask => self.select_page(Some(Page::QuickTask)),
            ModeKind::Plaza => self.select_page(Some(Page::Plaza)),
        }
    }
    pub fn open_page(&mut self, p: Page, cx: &mut Context<Self>) {
        self.select_page(Some(p));
        cx.notify();
    }
    pub fn close_page(&mut self, cx: &mut Context<Self>) {
        self.select_page(None);
        cx.notify();
    }
    pub fn open_mode(&mut self, m: ModeKind, carry: Option<String>, cx: &mut Context<Self>) {
        Store::edit(cx, |s| self.select_mode(m, carry, &mut s.ux));
        cx.notify();
    }
}
#[cfg(test)]
mod accept;

#[cfg(test)]
mod page_kind_tests {
    use super::Page;
    use std::prelude::v1::test;

    fn expected_kind(page: &Page) -> &'static str {
        match page {
            Page::Research => "research",
            Page::Storage => "storage",
            Page::Chat => "chat",
            Page::QuickTask => "quick",
            Page::Plaza => "plaza",
            Page::Session(_) => "session",
            Page::Paper(_) => "paper",
            Page::Routine => "routine",
        }
    }

    fn missing_kinds<'a>(pages: &[Page], kinds: &[&'a str]) -> Vec<&'static str> {
        pages.iter().map(expected_kind).filter(|kind| !kinds.contains(kind)).collect()
    }

    fn pages() -> Vec<Page> {
        vec![Page::Research, Page::Storage, Page::Chat, Page::QuickTask, Page::Plaza,
             Page::Session("subject".into()), Page::Paper("subject".into()), Page::Routine]
    }

    #[test]
    fn kinds_cover_exactly_all_page_variants() {
        let pages = pages();
        for page in &pages {
            assert_eq!(page.kind(), expected_kind(page));
            assert!(Page::KINDS.contains(&page.kind()));
        }
        assert!(missing_kinds(&pages, Page::KINDS).is_empty());
        assert_eq!(Page::KINDS.len(), pages.len());
        assert!(Page::KINDS.iter().all(|kind| pages.iter().any(|page| page.kind() == *kind)));

        let without_paper: Vec<_> = Page::KINDS.iter().copied().filter(|kind| *kind != "paper").collect();
        assert_eq!(missing_kinds(&pages, &without_paper), vec!["paper"]);
    }

    #[test]
    fn only_session_and_paper_need_subjects() {
        for kind in Page::KINDS {
            assert_eq!(Page::needs_subject(kind), *kind == "session" || *kind == "paper", "{kind}");
        }
        assert!(!Page::needs_subject("unknown"));
    }
}
