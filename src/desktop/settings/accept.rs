use super::*;
use std::any::TypeId;

#[test]
fn toggle_ids_are_stable() {
    assert_eq!(LOGIN_START_ID, "login-start");
    assert_eq!(ARGON_EVERY_ROUND_ID, "argon-every-round");
}

#[test]
fn facade_types_and_login_setter_are_reexported() {
    use crate::desktop::model::facade::settings as facade;
    assert_eq!(TypeId::of::<Store>(), TypeId::of::<facade::Store>());
    assert_eq!(TypeId::of::<Settings>(), TypeId::of::<facade::Settings>());
    assert_eq!(TypeId::of::<SettingPrior>(), TypeId::of::<facade::SettingPrior>());
    assert_eq!(TypeId::of::<WindowMode>(), TypeId::of::<facade::WindowMode>());
    assert_eq!(TypeId::of::<ThemeChoice>(), TypeId::of::<facade::ThemeChoice>());
    assert_eq!(TypeId::of::<LanguageChoice>(), TypeId::of::<facade::LanguageChoice>());
    let setter: fn(&mut AppState, bool) = set_login_start;
    assert!(std::ptr::fn_addr_eq(setter, facade::set_login_start as fn(&mut AppState, bool)));
}

#[test]
fn hook_installation_is_idempotent_and_preserves_existing_hook() {
    std::thread::spawn(|| {
        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
        let outcome = result.clone();
        gpui_kit::platform::headless().run(move |cx| {
            let check = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert!(!cx.has_global::<PlainWordsApply>());
                install_hooks(cx);
                let expected: fn(bool, &mut App) = facade::work::apply_plain_words;
                assert!(std::ptr::fn_addr_eq(cx.global::<PlainWordsApply>().0, expected));
                install_hooks(cx);
                assert!(std::ptr::fn_addr_eq(cx.global::<PlainWordsApply>().0, expected));
                fn custom(_: bool, _: &mut App) {}
                cx.set_global(PlainWordsApply(custom));
                install_hooks(cx);
                assert!(std::ptr::fn_addr_eq(cx.global::<PlainWordsApply>().0, custom as fn(bool, &mut App)));
            }));
            *outcome.lock().unwrap() = Some(check);
            cx.quit();
        });
        if let Err(error) = result.lock().unwrap().take().expect("app callback did not run") {
            std::panic::resume_unwind(error);
        };
    }).join().unwrap();
}

#[test]
fn view_has_no_duplicate_store_or_council_dependency() {
    let source = include_str!("../settings.rs");
    assert!(!source.contains("struct Store"));
    assert!(!source.contains("crate::council"));
}
