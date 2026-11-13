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
