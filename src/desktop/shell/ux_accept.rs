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
fn page_escape_is_scoped_and_yields_to_overlays() {
    use super::pages::{can_close_page, page_escape_binding};
    let binding = page_escape_binding();
    assert_eq!(binding.keystrokes()[0].inner().key, "escape");
    assert_eq!(binding.predicate().map(|p| p.to_string()).as_deref(), Some("ChronosShell"));
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
