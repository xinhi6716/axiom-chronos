use super::background::{close_copy, close_plan, ClosePlan};
use crate::desktop::{
    i18n::Lang,
    ux::seams::{BackgroundService, FakeBackground},
};
use std::prelude::v1::test;
#[test]
fn background_detach_and_reattach_preserve_the_selected_sessions() {
    let service = FakeBackground::new(true);
    let ids = vec!["session-a".to_owned(), "session-b".to_owned()];
    let plan = close_plan(&ids, Some(&service));
    assert_eq!(
        plan,
        ClosePlan::Continue {
            sessions: ids.clone()
        }
    );
    assert!(
        close_copy(&plan, &["First".into(), "Second".into()], Lang::ZhTw)
            .unwrap()
            .description
            .contains("2 個願望")
    );
    service.detach(&ids).unwrap();
    assert_eq!(service.detached(), ids);
    assert_eq!(service.reattach().unwrap(), ids);
    assert!(service.detached().is_empty());
}
#[test]
fn unavailable_service_never_offers_background_close() {
    let ids = vec!["session".to_owned()];
    let unavailable = FakeBackground::new(false);
    let plan = close_plan(&ids, Some(&unavailable));
    assert!(matches!(plan, ClosePlan::AskStop { .. }));
    assert!(close_copy(&plan, &["Wish".into()], Lang::En).is_none());
    assert!(unavailable.detach(&ids).is_err());
    assert!(unavailable.detached().is_empty());
}

#[test]
fn service_backed_close_plan_asks_when_nothing_is_running_or_unavailable() {
    use crate::{
        base::Res,
        desktop::{
            live,
            model::{Backend, WishRequest},
        },
    };
    use std::sync::Arc;
    struct Recording;
    impl Backend for Recording {
        fn start_wish(&self, _: WishRequest) -> Res<()> {
            Ok(())
        }
        fn interject(&self, _: &str, _: &str) -> Res<()> {
            Ok(())
        }
        fn stop(&self, _: &str) -> Res<()> {
            Ok(())
        }
        fn resume(&self, _: &str, _: Option<&str>) -> Res<()> {
            Ok(())
        }
        fn delete_memory(&self, _: i64) -> Res<()> {
            Ok(())
        }
        fn correct_memory(&self, _: i64, _: &str) -> Res<()> {
            Ok(())
        }
        fn refresh(&self) -> Res<()> {
            Ok(())
        }
        fn recheck(&self) -> Res<()> {
            Ok(())
        }
        fn follow(&self, _: &str) -> Res<()> {
            Ok(())
        }
    }
    let home = tempfile::tempdir().unwrap();
    let real = live::test_service_background(home.path().to_path_buf());
    let mut state = crate::desktop::demo::state();
    state.backend = Some(Box::new(Recording));
    state.service_background = Some(Arc::downgrade(&real));
    state.ux.seams.background = Some(real);
    assert!(super::service_backed(&state));
    assert_eq!(super::close_plan_for(&state, &[]), ClosePlan::CloseNow);
    let sessions = vec!["session".to_owned()];
    let unavailable: Arc<dyn BackgroundService> = Arc::new(FakeBackground::new(false));
    state.service_background = Some(Arc::downgrade(&unavailable));
    state.ux.seams.background = Some(unavailable);
    assert!(super::service_backed(&state));
    assert_eq!(
        super::close_plan_for(&state, &sessions),
        ClosePlan::AskStop {
            sessions: sessions.clone()
        }
    );
    state.ux.seams.background = None;
    assert_eq!(
        super::close_plan_for(&state, &sessions),
        ClosePlan::AskStop { sessions }
    );
}

#[test]
fn service_backed_stop_and_close_targets_hosted_running_sessions_in_order() {
    use crate::{
        base::Res,
        desktop::{
            demo,
            model::{Backend, RunStatus, WishRequest},
        },
    };
    use std::sync::{Arc, Mutex};
    struct Recording(Arc<Mutex<Vec<String>>>);
    impl Backend for Recording {
        fn start_wish(&self, _: WishRequest) -> Res<()> {
            Ok(())
        }
        fn interject(&self, _: &str, _: &str) -> Res<()> {
            Ok(())
        }
        fn stop(&self, session: &str) -> Res<()> {
            self.0.lock().unwrap().push(session.into());
            Ok(())
        }
        fn resume(&self, _: &str, _: Option<&str>) -> Res<()> {
            Ok(())
        }
        fn delete_memory(&self, _: i64) -> Res<()> {
            Ok(())
        }
        fn correct_memory(&self, _: i64, _: &str) -> Res<()> {
            Ok(())
        }
        fn refresh(&self) -> Res<()> {
            Ok(())
        }
        fn recheck(&self) -> Res<()> {
            Ok(())
        }
        fn follow(&self, _: &str) -> Res<()> {
            Ok(())
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut state = demo::state();
    state.backend = Some(Box::new(Recording(calls.clone())));
    let run = demo::sample_run();
    state.runs = [
        ("first", true, RunStatus::Active),
        ("remote", false, RunStatus::Active),
        ("second", true, RunStatus::LimitWait),
        ("done", true, RunStatus::Done),
    ]
    .into_iter()
    .map(|(session, hosted, status)| {
        let mut run = run.clone();
        run.session = session.into();
        run.hosted = hosted;
        run.status = status;
        run
    })
    .collect();
    let super::StopStep::StopThenWait { sessions } = super::stop_step(&state) else {
        panic!("expected stop targets")
    };
    assert_eq!(sessions, vec!["first", "second"]);
    for session in &sessions {
        state.stop(session);
    }
    assert_eq!(*calls.lock().unwrap(), vec!["first", "second"]);
    assert!(!super::stop_complete(
        &state,
        &sessions,
        std::time::Duration::from_secs(9)
    ));
    state.runs[0].status = RunStatus::Done;
    assert!(!super::stop_complete(
        &state,
        &sessions,
        std::time::Duration::from_secs(9)
    ));
    assert!(super::stop_complete(
        &state,
        &sessions,
        std::time::Duration::from_secs(10)
    ));
    state.runs[2].status = RunStatus::Done;
    assert!(super::stop_complete(
        &state,
        &sessions,
        std::time::Duration::from_secs(9)
    ));
    state.runs.clear();
    assert!(matches!(
        super::stop_step(&state),
        super::StopStep::CloseNow
    ));
}

#[test]
fn service_backed_stop_and_close_closes_now_without_backend() {
    let mut state = crate::desktop::demo::state();
    state.runs.clear();
    let mut run = crate::desktop::demo::sample_run();
    run.hosted = true;
    run.status = crate::desktop::model::RunStatus::Active;
    state.runs.push(run);
    state.backend = None;
    assert!(matches!(
        super::stop_step(&state),
        super::StopStep::CloseNow
    ));
}

#[test]
fn page_escape_is_scoped_and_yields_to_overlays() {
    use super::pages::{can_close_page, page_escape_binding};
    let binding = page_escape_binding();
    assert_eq!(binding.keystrokes()[0].inner().key, "escape");
    assert_eq!(
        binding.predicate().map(|p| p.to_string()).as_deref(),
        Some("ChronosShell")
    );
    assert!(can_close_page(true, false, false));
    assert!(!can_close_page(false, false, false));
    assert!(!can_close_page(true, true, false));
    assert!(!can_close_page(true, false, true));
}

#[test]
fn plaza_seat_labels_include_visible_glyph_and_name() {
    use super::modes::seat_label;
    use crate::desktop::ux::seams::Speaker;
    for (speaker, expected) in [
        (Speaker::Core, "⌛ Core"),
        (Speaker::Astra, "✦ Astra"),
        (Speaker::Fable, "♃ Fable"),
        (Speaker::Argon, "♊ Argon"),
    ] {
        assert_eq!(seat_label(speaker), expected);
    }
}
