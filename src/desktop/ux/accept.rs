use super::*;
use std::prelude::v1::test;

#[test]
fn prefs_round_trip_and_independent_damage() {
    let prefs = prefs::UxPrefs {
        extreme: true,
        read: [(
            "one".into(),
            prefs::ReadMark {
                key: 7,
                archived: true,
            },
        )]
        .into(),
        read_filter: ReadFilter::Unread,
        usage_wish: Some("wish".into()),
        usage_compare: Some("other".into()),
        drafts: [("s".into(), "draft".into())].into(),
        draft_ids: [("s".into(), "client-id".into())].into(),
        last_mode: ModeKind::Plaza,
    };
    assert_eq!(prefs::UxPrefs::parse(&prefs.to_json()), prefs);
    let damaged = serde_json::json!({"extreme":true,"read":{"good":{"key":2,"archived":true},"bad":{"key":"bad","archived":true}},"drafts":{"good":"text","bad":3},"read_filter":"broken","last_mode":"chat"});
    let p = prefs::UxPrefs::parse(&damaged);
    assert!(p.extreme);
    assert_eq!(p.read.len(), 1);
    assert_eq!(p.drafts.len(), 1);
    assert_eq!(p.read_filter, ReadFilter::All);
    assert_eq!(p.last_mode, ModeKind::Chat);
}

#[test]
fn draft_client_id_persists_rotates_and_clears() {
    let mut prefs = prefs::UxPrefs::default();
    prefs.drafts.insert("s".into(), "text".into());
    prefs.drafts.insert("other".into(), "other text".into());
    assert_eq!(prefs.draft_client_id("s", || "first".into()), "first");
    assert_eq!(
        prefs.draft_client_id("s", || panic!("id already exists")),
        "first"
    );
    assert_eq!(
        prefs.draft_client_id("other", || "other id".into()),
        "other id"
    );
    let mut prefs = prefs::UxPrefs::parse(&prefs.to_json());
    assert_eq!(
        prefs.draft_client_id("s", || panic!("id must survive")),
        "first"
    );
    prefs.rotate_draft_client_id("s", "second".into());
    assert_eq!(
        prefs.draft_client_id("s", || panic!("id was rotated")),
        "second"
    );
    prefs.clear_draft("s");
    assert!(!prefs.drafts.contains_key("s"));
    assert_eq!(prefs.draft_client_id("s", || "third".into()), "third");
    assert_eq!(
        prefs.drafts.get("other").map(String::as_str),
        Some("other text")
    );
    assert_eq!(
        prefs.draft_client_id("other", || panic!("other id must remain")),
        "other id"
    );
}

#[test]
fn damaged_draft_client_ids_are_ignored() {
    let mut prefs = prefs::UxPrefs::parse(&serde_json::json!({
        "draft_ids": {"good": "saved", "number": 3, "null": null, "object": {}}
    }));
    assert_eq!(prefs.draft_ids.len(), 1);
    assert_eq!(
        prefs.draft_client_id("good", || panic!("valid id must remain")),
        "saved"
    );
    assert_eq!(prefs.draft_client_id("number", || "new".into()), "new");
    assert!(prefs::UxPrefs::parse(&serde_json::json!({"draft_ids": 5}))
        .draft_ids
        .is_empty());
    assert!(prefs::UxPrefs::parse(&serde_json::json!({}))
        .draft_ids
        .is_empty());
}

#[test]
fn settings_ux_is_optional_and_persisted() {
    let old = super::super::settings::Settings::parse("{}");
    assert_eq!(old.ux, prefs::UxPrefs::default());
    let mut settings = old;
    settings.ux.last_mode = ModeKind::QuickTask;
    assert_eq!(
        super::super::settings::Settings::parse(&settings.to_json()).ux,
        settings.ux
    );
}

#[test]
fn demo_has_all_seams_and_matching_keys() {
    let s = super::super::demo::state_at(1_700_000_000);
    let x = &s.ux;
    let seams = &x.seams;
    assert!(
        seams.dual_core.is_some()
            && seams.turn_policy.is_some()
            && seams.interjections.is_some()
            && seams.approvals.is_some()
            && seams.storage.is_some()
            && seams.background.is_some()
            && seams.modes.is_some()
            && seams.sessions.is_some()
            && seams.unit_actions.is_some()
            && seams.optimizer.is_some()
            && seams.github.is_some()
    );
    for key in x
        .activity
        .keys()
        .chain(x.notices.keys())
        .chain(x.council.keys())
        .chain(x.amendments.keys())
        .chain(x.wish_ids.keys())
    {
        assert!(s.runs.iter().any(|r| &r.session == key));
    }
    let sessions = seams.sessions.as_ref().unwrap().sessions();
    assert!(!sessions.is_empty());
    assert!(sessions
        .iter()
        .any(|session| session.kind == SessionKind::Terminal && session.unit.is_some()));
    for session in sessions {
        let run = s
            .runs
            .iter()
            .find(|r| r.session.as_ref() == session.run)
            .unwrap();
        if let Some(unit) = session.unit {
            assert!(run.units.iter().any(|u| u.id.as_ref() == unit));
        }
    }
    for usage in &x.usage_sessions {
        assert!(s.runs.iter().any(|r| r.session == usage.session));
    }
}

#[test]
fn dual_core_and_turn_snapshots() {
    let d = FakeDualCore::new();
    assert_eq!(d.state("missing"), DualCore::Off);
    d.set(
        "run",
        DualCore::Split {
            topic: "scope".into(),
        },
    );
    assert_eq!(
        d.state("run"),
        DualCore::Split {
            topic: "scope".into()
        }
    );
    let t = FakeTurnPolicy::new(Some(FastCost { multiplier: 2.0 }));
    let first = t.begin_turn();
    assert_eq!(first.revision, 0);
    assert_eq!(t.set_extreme(true).revision, 1);
    assert_eq!(t.set_extreme(true).revision, 1);
    let second = t.begin_turn();
    assert_eq!(t.turns(), vec![first, second]);
    assert_eq!(t.fast_cost(), Some(FastCost { multiplier: 2.0 }));
}
#[test]
fn scripted_optimizer_and_github() {
    use crate::base::Failure;
    let proposal = Proposal {
        goal: "Improve citations".into(),
        scope: "Reader".into(),
        limits: "Keep PDFs local".into(),
        acceptance: "Citations verified".into(),
        ambiguities: vec![],
    };
    let optimizer = FakeOptimizer::new(vec![
        Err(Failure::new("timeout", "Timed out")),
        Ok(proposal.clone()),
    ]);
    assert_eq!(
        optimizer
            .propose("x", crate::desktop::i18n::Lang::En)
            .unwrap_err()
            .kind,
        "timeout"
    );
    assert_eq!(
        optimizer
            .propose("x", crate::desktop::i18n::Lang::En)
            .unwrap(),
        proposal
    );
    assert_eq!(optimizer.calls(), 2);
    let gh = FakeGh::new(vec![GhOutcome::TimedOut, GhOutcome::Missing]);
    let args = vec!["status".into()];
    assert_eq!(
        gh.run(&args, std::time::Duration::ZERO),
        GhOutcome::TimedOut
    );
    assert_eq!(gh.run(&[], std::time::Duration::ZERO), GhOutcome::Missing);
    assert_eq!(gh.calls(), vec![args, vec![]]);
}
#[test]
fn interjection_idempotency_stages_pause_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    let f = FakeInterjections::at(&path);
    let d = Draft {
        session: "run".into(),
        client_id: "id".into(),
        text: "Keep source files local".into(),
        pause_first: false,
        replan: true,
    };
    let receipt = f.send(&d).unwrap();
    assert_eq!(f.send(&d).unwrap(), receipt);
    assert_eq!(f.sent().len(), 1);
    assert_eq!(f.history("run").len(), 1);
    f.advance(
        "id",
        ReceiptStage::Applied {
            first_action: "review".into(),
        },
    );
    f.advance("id", ReceiptStage::Received);
    assert_eq!(f.receipt("id").unwrap().stage, ReceiptStage::Received);
    f.set_pause_failure(true);
    assert!(f.pause_batch("run").is_err());
    assert_eq!(f.pauses(), 1);
    drop(f);
    let resumed = FakeInterjections::at(&path);
    assert_eq!(resumed.receipt("id").unwrap().stage, ReceiptStage::Received);
    assert_eq!(resumed.send(&d).unwrap().stage, ReceiptStage::Received);
    assert!(resumed.sent().is_empty());
    assert_eq!(resumed.history("run").len(), 1);
}
#[test]
fn approval_version_idempotency_and_rejection() {
    let card = |id: &str| ApprovalCard {
        action_id: id.into(),
        version: 1,
        session: "run".into(),
        action: "Publish".into(),
        target: "report".into(),
        scope: "report only".into(),
        cost_usd: None,
    };
    let f = FakeApprovals::new(vec![card("a"), card("b")]);
    let mut d = Decision {
        action_id: "a".into(),
        payload_version: 1,
        key: "k".into(),
        verdict: Verdict::Approve,
    };
    f.bump_version("a");
    assert_eq!(f.decide(&d).unwrap_err().kind, "stale");
    d.payload_version = 2;
    let receipt = f.decide(&d).unwrap();
    assert_eq!(f.decide(&d).unwrap(), receipt);
    d.key = "other".into();
    assert_eq!(f.decide(&d).unwrap(), receipt);
    assert_eq!(f.executions("a"), 1);
    let reject = Decision {
        action_id: "b".into(),
        payload_version: 1,
        key: "reject".into(),
        verdict: Verdict::Reject {
            reason: "Not ready".into(),
        },
    };
    assert_eq!(
        f.decide(&reject).unwrap().outcome,
        ApprovalOutcome::Rejected
    );
    assert_eq!(f.executions("b"), 0);
    assert!(f.pending().is_empty());
}
#[test]
fn storage_confirmation_staleness_and_progress() {
    let item = |id: &str| StorageItem {
        id: id.into(),
        category: "cache".into(),
        project: None,
        wish: None,
        path: id.into(),
        bytes: 10,
        measured: true,
        reclaimable: true,
    };
    let f = FakeStorage::new(vec![item("a"), item("b")], None);
    let ids = vec!["a".into(), "b".into()];
    let p = f.preview(&ids).unwrap();
    assert_eq!(
        f.clean(&p.id, p.version, false).unwrap_err().kind,
        "unconfirmed"
    );
    assert!(f.deleted().is_empty());
    f.touch();
    assert_eq!(f.clean(&p.id, p.version, true).unwrap_err().kind, "stale");
    let p = f.preview(&ids).unwrap();
    f.fail_item("b");
    let job = f.clean(&p.id, p.version, true).unwrap();
    f.step();
    assert_eq!(f.deleted(), vec!["a"]);
    assert_eq!(f.progress(&job).unwrap().remaining, 1);
    f.step();
    let progress = f.progress(&job).unwrap();
    assert!(progress.finished);
    assert_eq!(progress.failed[0].0, "b");
    f.set_space(DiskSpace {
        free_bytes: 3,
        total_bytes: 10,
    });
    assert_eq!(f.space().unwrap().free_bytes, 3);
}
#[test]
fn background_detach_and_reattach() {
    let unavailable = FakeBackground::new(false);
    assert!(matches!(
        unavailable.status(),
        KeepAlive::Unavailable { .. }
    ));
    assert!(unavailable.detach(&["run".into()]).is_err());
    let f = FakeBackground::new(true);
    f.detach(&["run".into()]).unwrap();
    assert_eq!(f.detached(), vec!["run"]);
    assert_eq!(f.reattach().unwrap(), vec!["run"]);
    assert!(f.detached().is_empty());
}
#[test]
fn worker_session_control_and_reconnection() {
    let f = FakeSessions::new(vec![WorkerSession {
        id: "terminal".into(),
        run: "run".into(),
        unit: Some("unit".into()),
        title: "Worker".into(),
        kind: SessionKind::Terminal,
    }]);
    f.push("terminal", "first");
    f.push("terminal", "second");
    assert_eq!(f.frames("terminal", 1).unwrap()[0].text, "second");
    let t = f.request_takeover("terminal").unwrap();
    assert_eq!(f.request_takeover("terminal").unwrap_err().kind, "busy");
    assert_eq!(
        f.send_input("terminal", &t, "x").unwrap_err().kind,
        "not_controller"
    );
    f.confirm("terminal");
    assert_eq!(
        f.send_input("terminal", "wrong", "x").unwrap_err().kind,
        "not_controller"
    );
    f.send_input("terminal", &t, "status").unwrap();
    assert_eq!(f.inputs("terminal"), vec!["status"]);
    f.disconnect("terminal");
    assert_eq!(f.connection("terminal"), Connection::Lost);
    f.reconnect("terminal").unwrap();
    assert_eq!(f.connection("terminal"), Connection::Live);
    f.hand_back("terminal", &t).unwrap();
    let _ = f.request_takeover("terminal").unwrap();
    f.refuse("terminal", "Busy");
    assert!(matches!(
        f.controller("terminal"),
        Controller::Refused { .. }
    ));
}
#[test]
fn unit_actions_require_reasons_and_deduplicate() {
    let f = FakeUnitActions::new();
    f.set_evidence(
        "run",
        "u",
        UnitEvidence {
            failure: Some("Build failed".into()),
            ..Default::default()
        },
    );
    assert_eq!(f.evidence("run", "missing").unwrap_err().kind, "not_found");
    assert_eq!(
        f.act("run", "missing", &UnitActionKind::Retry, "missing-action")
            .unwrap_err()
            .kind,
        "not_found"
    );
    assert_eq!(f.count("missing", "retry"), 0);
    assert_eq!(
        f.act(
            "run",
            "u",
            &UnitActionKind::Waive {
                reason: "  ".into(),
                finding: None
            },
            "x"
        )
        .unwrap_err()
        .kind,
        "reason_required"
    );
    let r = f.act("run", "u", &UnitActionKind::Retry, "x").unwrap();
    assert_eq!(f.act("run", "u", &UnitActionKind::Retry, "x").unwrap(), r);
    assert_eq!(f.count("u", "retry"), 1);
    f.act(
        "run",
        "u",
        &UnitActionKind::Waive {
            reason: "Reviewed".into(),
            finding: None,
        },
        "y",
    )
    .unwrap();
    assert_eq!(f.count("u", "waive"), 1);
}
#[test]
fn modes_route_budget_quick_and_handoff() {
    let f = FakeModes::new();
    assert_eq!(f.plaza("plaza").seats.len(), 4);
    f.chat_send("chat", "Explain the reader").unwrap();
    assert_eq!(f.chat("chat").messages.len(), 2);
    assert!(f
        .handoff(&ModeRef::Chat("chat".into()))
        .unwrap()
        .contains("reader"));
    let q = f
        .quick_start(&QuickRequest {
            cwd: None,
            text: "Fix reader".into(),
            review: true,
            context: None,
        })
        .unwrap();
    for _ in 0..4 {
        f.step_quick(&q);
    }
    assert!(matches!(f.quick(&q).unwrap().step, QuickStep::Done { .. }));
    let route = f
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "Review this".into(),
                mentions: vec![Speaker::Argon, Speaker::Astra],
            },
        )
        .unwrap();
    assert_eq!(route.picked, vec![Speaker::Argon, Speaker::Astra]);
    let keyword = f
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "code".into(),
                mentions: vec![],
            },
        )
        .unwrap();
    assert_eq!(keyword.picked, vec![Speaker::Argon, Speaker::Core]);
    assert_eq!(keyword.reason, "Selected members by topic");
    let capped = f
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "All members".into(),
                mentions: vec![
                    Speaker::Argon,
                    Speaker::Core,
                    Speaker::Astra,
                    Speaker::Fable,
                ],
            },
        )
        .unwrap();
    assert_eq!(
        capped.picked,
        vec![Speaker::Argon, Speaker::Core, Speaker::Astra]
    );
    let id = f.plaza("plaza").messages[0].id;
    f.pin("plaza", id).unwrap();
    assert_eq!(f.plaza("plaza").pins[0].sources, vec![id]);
    f.set_seats(
        "plaza",
        vec![PlazaSeat {
            member: Speaker::Astra,
            budget_usd: Some(1.0),
            spent_usd: Some(1.0),
        }],
    );
    let r = f
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "review".into(),
                mentions: vec![Speaker::Astra],
            },
        )
        .unwrap();
    assert!(r.picked.is_empty() && r.reason.contains("budget"));
    assert!(f.calls() >= 5);
}
#[test]
fn navigation_updates_page_composer_and_persisted_mode() {
    // Root lib tests do not enable gpui-kit test-support; these are the mutations used by the GPUI wrappers.
    let mut state = super::super::demo::state();
    let mut settings = super::super::settings::Settings::default();
    state.select_page(Some(Page::Storage));
    assert_eq!(state.ux.page, Some(Page::Storage));
    state.select_page(Some(Page::Session("terminal".into())));
    assert_eq!(state.ux.page, Some(Page::Session("terminal".into())));
    state.select_page(None);
    assert_eq!(state.ux.page, None);
    for (mode, page) in [
        (ModeKind::Chat, Page::Chat),
        (ModeKind::QuickTask, Page::QuickTask),
        (ModeKind::Plaza, Page::Plaza),
    ] {
        state.select_mode(mode, None, &mut settings.ux);
        assert_eq!(state.ux.page, Some(page));
        assert_eq!(
            super::super::settings::Settings::parse(&settings.to_json())
                .ux
                .last_mode,
            mode
        );
    }
    state.select_mode(
        ModeKind::Wish,
        Some("Carry context".into()),
        &mut settings.ux,
    );
    assert_eq!(state.ux.page, None);
    assert!(state.composing);
    assert_eq!(state.ux.composer_prefill.as_deref(), Some("Carry context"));
    assert_eq!(
        super::super::settings::Settings::parse(&settings.to_json())
            .ux
            .last_mode,
        ModeKind::Wish
    );
}
#[test]
fn demo_populates_activity_notices_usage_and_evidence() {
    let state = super::super::demo::state_at(1_700_000_000);
    let current = state.run().unwrap();
    let ux = &state.ux;
    let projection = &ux.activity[&current.session];
    assert_eq!(projection.last_activity_ms, Some(1_699_999_988_000));
    assert_eq!(projection.last_seen_ms, Some(1_699_999_995_000));
    assert!(current
        .modules
        .iter()
        .any(|m| m.id == projection.open[0].module));
    assert!(!ux.council[&current.session].active);
    assert!(!ux.notices.contains_key(&current.session));
    assert_eq!(ux.notices.values().map(Vec::len).sum::<usize>(), 3);
    assert!(ux.usage_sessions.iter().any(|u| !u.complete));
    assert!(ux.usage_sessions.iter().any(|u| ux
        .usage_sessions
        .iter()
        .filter(|v| v.wish == u.wish)
        .count()
        >= 2));
    assert!(
        ux.usage_sessions
            .iter()
            .map(|u| &u.wish)
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 2
    );
    let failed = current
        .units
        .iter()
        .find(|u| u.state == super::super::model::UnitState::Failed)
        .unwrap();
    assert!(ux
        .seams
        .unit_actions
        .as_ref()
        .unwrap()
        .evidence(&current.session, &failed.id)
        .unwrap()
        .failure
        .is_some());
    assert_eq!(
        ux.seams
            .interjections
            .as_ref()
            .unwrap()
            .history(&current.session)
            .len(),
        1
    );
    assert_eq!(
        ux.seams.approvals.as_ref().unwrap().pending()[0].session,
        current.session.as_ref()
    );
    let report = ux.seams.storage.as_ref().unwrap().report().unwrap();
    assert_eq!(report.items.len(), 4);
    assert!(report.space.unwrap().free_bytes > 0);
    for notices in ux.notices.values() {
        for notice in notices {
            if let NoticeKind::ScopeFailure { unit, .. } = &notice.kind {
                assert!(state
                    .runs
                    .iter()
                    .any(|r| r.units.iter().any(|u| u.id.as_ref() == unit)));
            }
        }
    }
}
