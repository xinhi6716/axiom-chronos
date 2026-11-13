use super::{install_policy, TurnPolicy};
use crate::{
    codex::extreme,
    desktop::{
        demo,
        i18n::{self, Lang},
        settings::Store,
        ux::{real::extreme::CodexTurnPolicy, seams::FakeTurnPolicy},
    },
};
use gpui_kit::{App, AppContext};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
    time::Instant,
};

// Windows' message loop belongs to the thread that creates the platform. Always quit it,
// including on an assertion failure, and propagate that failure after the thread joins.
fn with_app(test: impl FnOnce(&mut App) + Send + 'static) {
    static APP_LOCK: Mutex<()> = Mutex::new(());
    let _lock = APP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::thread::spawn(move || {
        let result = Arc::new(Mutex::new(None));
        let outcome = result.clone();
        gpui_kit::platform::headless().run(move |cx| {
            let result = catch_unwind(AssertUnwindSafe(|| {
                cx.set_global(Store::open(true));
                i18n::set(Lang::En, cx);
                test(cx);
            }));
            *outcome.lock().unwrap() = Some(result);
            cx.quit();
        });
        if let Err(panic) = result
            .lock()
            .unwrap()
            .take()
            .expect("app callback did not run")
        {
            std::panic::resume_unwind(panic);
        };
    })
    .join()
    .unwrap();
}

#[test]
fn installation_toggling_and_turn_snapshots() {
    with_app(|cx| {
        let dir = tempfile::tempdir().unwrap();
        // A first installation seeds the fallback policy from the stored preference.
        Store::edit(cx, |prefs| prefs.ux.extreme = true);
        let mut state = demo::state();
        state.ux.seams.turn_policy = None;
        install_policy(&mut state, cx);
        let fallback = state.ux.seams.turn_policy.as_ref().unwrap().clone();
        assert!(Store::current(cx).ux.extreme);
        assert!(fallback.current().extreme);
        let entity = cx.new(|_| state);
        entity.update(cx, |s, cx| s.set_extreme(false, cx));
        assert!(!Store::current(cx).ux.extreme);
        assert!(!fallback.current().extreme);
        assert!(entity.read(cx).ux.extreme_flash.is_none());

        // A real existing policy wins over the opposite preference, and toggles persist.
        let home = dir.path().join("slots");
        extreme::set(&home, true).unwrap();
        let policy = Arc::new(CodexTurnPolicy::new(home.clone()));
        let mut state = demo::state();
        state.ux.seams.turn_policy = Some(policy.clone());
        install_policy(&mut state, cx);
        assert!(Store::current(cx).ux.extreme);
        assert!(policy.current().extreme);
        assert!(extreme::load(&home).on);
        let entity = cx.new(|_| state);
        entity.update(cx, |s, cx| s.set_extreme(false, cx));
        assert!(!Store::current(cx).ux.extreme);
        assert!(!policy.current().extreme);
        assert!(!extreme::load(&home).on);
        assert!(entity.read(cx).ux.extreme_flash.is_none());
        entity.update(cx, |s, cx| s.set_extreme(true, cx));
        assert!(Store::current(cx).ux.extreme);
        assert!(policy.current().extreme);
        assert!(extreme::load(&home).on);
        let flash = entity.read(cx).ux.extreme_flash;
        assert!(flash.is_some());
        let revision = policy.current().revision;
        let bytes = std::fs::read(extreme::path(&home)).unwrap();
        // Defect: repeating the current value used to clear the flash and rewrite the policy.
        entity.update(cx, |s, cx| s.set_extreme(true, cx));
        assert!(Store::current(cx).ux.extreme);
        assert_eq!(policy.current().revision, revision);
        assert_eq!(entity.read(cx).ux.extreme_flash, flash);
        assert_eq!(std::fs::read(extreme::path(&home)).unwrap(), bytes);
        assert!(extreme::load(&home).on);

        // A turn owns its original snapshot; only the next turn sees the change.
        Store::edit(cx, |prefs| prefs.ux.extreme = false);
        let fake = Arc::new(FakeTurnPolicy::new(None));
        let mut state = demo::state();
        state.reduce_motion = true;
        state.ux.seams.turn_policy = Some(fake.clone());
        install_policy(&mut state, cx);
        assert!(!Store::current(cx).ux.extreme);
        assert!(!fake.current().extreme);
        let entity = cx.new(|_| state);
        let running = fake.begin_turn();
        entity.update(cx, |s, cx| s.set_extreme(true, cx));
        assert!(Store::current(cx).ux.extreme);
        assert!(fake.current().extreme);
        assert!(entity.read(cx).ux.extreme_flash.is_none());
        assert!(!running.extreme);
        let next = fake.begin_turn();
        assert!(next.extreme);
        assert_eq!(fake.turns(), vec![running, next]);
    });
}

#[test]
fn failed_save_does_not_flip_preference_and_reports_error() {
    with_app(|cx| {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let home = blocker.join("slots");
        let policy = Arc::new(CodexTurnPolicy::new(home.clone()));
        let err = policy.try_set_extreme(true).unwrap_err();
        assert_eq!(err.kind, "io");
        let mut state = demo::state();
        state.ux.seams.turn_policy = Some(policy.clone());
        state.ux.extreme_flash = Some(Instant::now());
        install_policy(&mut state, cx);
        assert!(!Store::current(cx).ux.extreme);
        assert!(!policy.current().extreme);
        let entity = cx.new(|_| state);
        entity.update(cx, |s, cx| s.set_extreme(true, cx));
        assert!(!Store::current(cx).ux.extreme);
        assert!(!policy.current().extreme);
        assert!(!extreme::load(&home).on);
        assert!(entity.read(cx).ux.extreme_flash.is_none());
        assert_eq!(
            entity.read(cx).errors.last().unwrap().message.as_ref(),
            format!("Could not save extreme mode: {}", err.message)
        );
    });
}
