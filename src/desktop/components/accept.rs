#[test]
fn wishes_tokens_preserve_values() {
    use super::tokens::*;
    use gpui_kit::px;
    use std::time::Duration;
    for (actual, expected) in [
        (PROGRESS_RING_STROKE, 4.), (UNREAD_DOT_SIZE, 7.), (SPLIT_HANDLE_WIDTH, 12.),
        (ICON_STATUS, 13.), (ICON_ROW, 14.), (REM_BASE, 16.), (ICON_EMPTY, 18.),
        (EMPTY_STATE_ICON_SIZE, 22.), (ENTRY_HEIGHT, 34.), (LOG_TIME_WIDTH, 56.),
        (LOG_LEVEL_WIDTH, 112.), (NOTE_MIN_WIDTH, 120.), (LOG_OVERSCAN, 240.),
        (LOG_HEIGHT, 320.), (TOOLTIP_MAX_WIDTH, 340.), (LOG_SCROLL_BACK, 400.),
        (DIALOG_WIDTH, 420.), (WINDOW_HEIGHT, 700.), (WINDOW_WIDTH, 1100.),
    ] {
        assert_eq!(actual, px(expected));
    }
    assert_eq!(RESUME_SETTLE, Duration::from_secs(8));
    assert_eq!(PENDING_TIMEOUT, Duration::from_secs(20));
    assert_eq!(POLL_INTERVAL, Duration::from_millis(500));
    assert_eq!(LOG_SETTLE, Duration::from_millis(800));
    assert_eq!([FRAME_TINT_OPACITY, REVIEW_BACKGROUND_OPACITY, TAG_BACKGROUND_OPACITY,
        TAG_BORDER_OPACITY, FRAME_BORDER_OPACITY, RAW_STATE_OPACITY],
        [0.06, 0.1, 0.12, 0.35, 0.6, 0.7]);
}

#[test]
fn radii_are_ordered() {
    use super::tokens::{radius, Radius::*};
    assert!(radius(Sm) < radius(Md) && radius(Md) < radius(Lg) && radius(Lg) < radius(Pill));
}

#[test]
fn elevations_increase_in_blur_and_alpha() {
    use super::tokens::{elevation, Elevation::*};
    for t in crate::desktop::theme::both() {
        assert!(elevation(Flat, &t).is_empty());
        let shadows: Vec<_> = [Raised, Overlay, Modal]
            .into_iter()
            .map(|e| elevation(e, &t)[0].clone())
            .collect();
        assert!(shadows
            .windows(2)
            .all(|s| s[0].blur_radius < s[1].blur_radius && s[0].color.a < s[1].color.a));
    }
}

#[test]
fn glass_varies_by_level_and_theme() {
    use super::tokens::{glass, Glass::*};
    let [dark, light] = crate::desktop::theme::both();
    for t in [dark, light] {
        let fills: Vec<_> = [Subtle, Card, Hero]
            .into_iter()
            .map(|g| glass(g, &t).fill)
            .collect();
        assert_ne!(fills[0], fills[1]);
        assert_ne!(fills[1], fills[2]);
        assert_ne!(fills[0], fills[2]);
    }
    for g in [Subtle, Card, Hero] {
        assert_ne!(glass(g, &dark).fill, glass(g, &light).fill);
    }
}

#[test]
fn glow_is_bounded_and_scales() {
    use super::tokens::{glow, Glow::*};
    for t in crate::desktop::theme::both() {
        for g in [Rest, Active, LongRunning] {
            for n in [-1.0, 0.0, f32::NAN] {
                assert!(glow(g, n, &t).is_empty());
            }
            let low = glow(g, 0.25, &t)[0].clone();
            let high = glow(g, 0.75, &t)[0].clone();
            assert!(low.blur_radius < high.blur_radius && low.color.a < high.color.a);
        }
    }
}

#[test]
fn theme_and_motion_reexports_resolve() {
    use super::tokens::{
        font, space, Curve, Density, Dur, Ease, Pace, Spring, Step, Tokened, Tokens, TypeSize,
    };
    fn tokened<T: Tokened>() {}
    tokened::<gpui_kit::App>();
    let _: Tokens = crate::desktop::theme::night();
    assert!(space(Density::Comfortable, Step::Sm) > space(Density::Comfortable, Step::Xs));
    assert!(
        font(Density::Comfortable, TypeSize::Title) > font(Density::Comfortable, TypeSize::Body)
    );
    assert!(Pace::Glow.duration().as_millis() > 0);
    assert_eq!(Curve::Spring.at(1.0), 1.0);
    assert!(Dur::Quick.get().as_millis() > 0);
    assert_eq!(Ease::Out.at(1.0), 1.0);
    assert!(Spring::SNAPPY.stiffness > 0.0);
}
