use super::{both, columns_for, focus_ring, font, lift_shadow, selection_glow, space, row_height,
    Density, Hsla, Rgba, Step, TypeSize, RADIUS, RADIUS_SMALL, READ_WIDTH, TARGET_MIN, WIDE_BREAKPOINT};
use gpui_kit::px;

fn luminance(c: Hsla) -> f32 {
    let c = Rgba::from(c);
    let line = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * line(c.r) + 0.7152 * line(c.g) + 0.0722 * line(c.b)
}

fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn scales() {
    let steps = [Step::Xs, Step::Sm, Step::Md, Step::Lg, Step::Xl, Step::Xxl];
    let sizes = [TypeSize::Caption, TypeSize::Body, TypeSize::Label, TypeSize::Title, TypeSize::Heading, TypeSize::Display];
    for (density, spaces, fonts, row) in [
        (Density::Comfortable, [4., 8., 12., 16., 24., 32.], [12., 13., 12., 15., 20., 28.], 36.),
        (Density::Dense, [2., 4., 8., 12., 16., 24.], [12., 12., 12., 14., 18., 24.], 28.),
    ] {
        for (step, expected) in steps.into_iter().zip(spaces) {
            assert_eq!(space(density, step), px(expected));
        }
        for (size, expected) in sizes.into_iter().zip(fonts) {
            assert_eq!(font(density, size), px(expected));
            assert!(font(density, size) >= px(12.));
        }
        assert_eq!(row_height(density), px(row));
        assert!(row_height(density) >= TARGET_MIN);
    }
    assert_eq!(Density::default(), Density::Comfortable);
    assert_eq!(columns_for(px(1399.)), 1);
    assert_eq!(columns_for(px(1400.)), 2);
    assert!(RADIUS <= px(4.));
    assert_eq!(RADIUS, px(4.));
    assert_eq!(RADIUS_SMALL, px(2.));
    assert_eq!(TARGET_MIN, px(28.));
    assert_eq!(READ_WIDTH, px(720.));
    assert_eq!(WIDE_BREAKPOINT, px(1400.));
}

#[test]
fn shadows() {
    for t in both() {
        for level in [0., -1., f32::NAN] {
            assert!(lift_shadow(&t, level).is_empty());
            assert!(selection_glow(&t, level).is_empty());
        }
        let mut previous = (0., px(0.), px(0.));
        let mut glow_alpha = 0.;
        for level in [0.25, 0.5, 0.75, 1.] {
            let lift = lift_shadow(&t, level);
            assert_eq!(lift.len(), 1);
            let shadow = &lift[0];
            assert!(shadow.color.a >= previous.0 && shadow.blur_radius >= previous.1 && shadow.offset.y >= previous.2);
            assert_eq!(shadow.color.l, 0.);
            assert!(shadow.color.a <= if t.surface.l < 0.5 { 0.35 } else { 0.18 });
            previous = (shadow.color.a, shadow.blur_radius, shadow.offset.y);
            let glow = selection_glow(&t, level);
            assert!(!glow.is_empty());
            assert_eq!(glow[0].color.h, t.gold.h);
            assert!(glow[0].color.a > glow_alpha);
            glow_alpha = glow[0].color.a;
        }
        let low_lift = lift_shadow(&t, 0.25);
        let high_lift = lift_shadow(&t, 1.0);
        assert!(high_lift[0].color.a > low_lift[0].color.a);
        assert!(high_lift[0].blur_radius > low_lift[0].blur_radius);
        assert!(selection_glow(&t, 1.0)[0].color.a > selection_glow(&t, 0.25)[0].color.a);
        assert_eq!(lift_shadow(&t, 2.), lift_shadow(&t, 1.));
        assert_eq!(selection_glow(&t, 2.), selection_glow(&t, 1.));
        let ring = focus_ring(&t);
        assert_eq!(ring.len(), 1);
        assert_eq!(ring[0].spread_radius, px(2.));
        assert_eq!(ring[0].blur_radius, px(0.));
        assert_eq!(ring[0].offset, gpui_kit::Point::default());
        assert_eq!(ring[0].color, t.gold);
    }
}

#[test]
fn depth_and_raised_contrast() {
    for t in both() {
        assert!(luminance(t.canvas) < luminance(t.surface));
        assert!(luminance(t.surface) < luminance(t.raised));
        for fg in [t.text, t.muted, t.gold, t.ok, t.run, t.review, t.fail] {
            let ratio = contrast(fg, t.raised);
            assert!(ratio >= 4.5, "{fg:?} on {:?}: {ratio}", t.raised);
        }
    }
}

#[test]
fn ornament_sand_height_is_clamped_and_monotonic() {
    use super::super::ornament::sand_height;
    for (input, expected) in [(0.0, 0.0), (1.0, 1.0), (-1.0, 0.0), (2.0, 1.0), (f32::NAN, 0.0)] {
        assert_eq!(sand_height(input), expected);
    }
    let mut previous = sand_height(0.0);
    for i in 1..=20 {
        let current = sand_height(i as f32 * 0.05);
        assert!(current >= previous);
        previous = current;
    }
}

#[test]
fn ornament_hourglass_geometry_stays_in_bounds_and_sand_rises() {
    use super::super::ornament::hourglass_geometry;
    for side in [24.0, 60.0] {
        let g = hourglass_geometry(side);
        for &(x, y) in g.upper.iter().chain(&g.lower).chain(std::iter::once(&g.neck)) {
            assert!((0.0..=side).contains(&x) && (0.0..=side).contains(&y));
        }
        for rect in [g.upper_sand, g.lower_sand, g.sand_rects(1.0).0, g.sand_rects(0.25).0] {
            assert!(rect.x >= 0.0 && rect.y >= 0.0 && rect.x + rect.w <= side && rect.y + rect.h <= side);
        }
        assert!(g.sand_rects(1.0).0.h > g.sand_rects(0.25).0.h);
        assert_eq!(g.sand_rects(0.0).0.h, 0.0);
        assert_eq!(g.sand_rects(1.0).1.h, 0.0);
    }
}

#[test]
fn ornament_raised_shadows_combine_in_order() {
    use super::super::ornament::raised_card_shadows;
    for t in both() {
        assert!(raised_card_shadows(&t, 0.0, 0.0).is_empty());
        let lift = lift_shadow(&t, 1.0);
        let glow = selection_glow(&t, 1.0);
        assert_eq!(raised_card_shadows(&t, 1.0, 0.0), lift);
        assert_eq!(raised_card_shadows(&t, 0.0, 1.0), glow);
        let combined = raised_card_shadows(&t, 1.0, 1.0);
        assert_eq!(combined.len(), lift.len() + glow.len());
        assert_eq!(&combined[..lift.len()], &lift);
        assert_eq!(&combined[lift.len()..], &glow);
        assert_eq!(combined[lift.len()].color.h, t.gold.h);
    }
}

#[test]
fn ornament_elements_construct_in_both_themes() {
    use super::super::ornament::{empty_state, hourglass, raised_card, stale_note};
    for t in both() {
        drop(raised_card(&t, 1.0, 1.0));
        drop(empty_state(&t, "Title".into(), "Line".into()));
        drop(stale_note(&t, "Stale".into()));
        drop(hourglass(&t, Some(0.5), t.text, 24.0));
        drop(hourglass(&t, None, t.text, 24.0));
    }
}

#[test]
fn ornament_negative_control_rejects_stubs() {
    use super::super::ornament::{raised_card_shadows, sand_height};
    assert!(sand_height(0.3) < sand_height(0.7));
    for t in both() {
        assert!(!raised_card_shadows(&t, 1.0, 0.0).is_empty());
    }
}
