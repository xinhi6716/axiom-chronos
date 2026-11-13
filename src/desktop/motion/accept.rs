use super::*;
use std::prelude::v1::test;

const TICK: Duration = Duration::from_micros(8_300);

#[test]
fn display_refresh_is_not_throttled() {
    let start = Instant::now();
    let mut tween = Tween::rest(0.0, Pace::Open, Curve::Smooth);
    tween.set(1.0, start, false);
    for i in 0..36 {
        let mut driver = Driver::at(start + TICK * i, false);
        driver.value(&tween);
        assert_eq!(driver.ask(true), FrameAsk::Next);
    }
    let sources = [
        include_str!("mod.rs"),
        include_str!("tokens.rs"),
        include_str!("spring.rs"),
    ];
    for source in sources {
        assert!(!source.contains(concat!("FRAME", "_GAP")));
        assert!(!source.contains(concat!("from_millis(", "33", ")")));
    }
}

#[test]
fn frame_decision_negative_control() {
    assert_eq!(frame_ask(true, true, true), FrameAsk::Next);
    assert_eq!(frame_ask(false, true, true), FrameAsk::Idle);
    assert_eq!(frame_ask(true, false, true), FrameAsk::Idle);
    assert_eq!(frame_ask(true, true, false), FrameAsk::Idle);
}

#[test]
fn drivers_stop_when_hidden_or_idle() {
    let start = Instant::now();
    let mut driver = Driver::at(start, false);
    assert_eq!(driver.ask(true), FrameAsk::Idle);
    assert!(!driver.wants_frame());
    driver.keep(true);
    assert_eq!(driver.ask(true), FrameAsk::Next);
    assert_eq!(driver.ask(false), FrameAsk::Idle);
    driver.view_visible(false);
    assert_eq!(driver.ask(true), FrameAsk::Idle);
    assert!(!driver.wants_frame());
    driver.view_visible(true);
    let mut tween = Tween::rest(0.0, Pace::Open, Curve::Smooth);
    tween.set(1.0, start, false);
    let mut settled = Driver::at(start + Pace::Open.duration(), false);
    settled.track(&tween);
    assert_eq!(settled.ask(true), FrameAsk::Idle);
    assert!(!settled.wants_frame());
}

fn bounded(values: &[f32]) {
    for pair in values.windows(2) {
        assert!((pair[1] - pair[0]).abs() <= 0.2, "{pair:?}");
    }
}

#[test]
fn interruptions_and_reversals_are_continuous() {
    let start = Instant::now();
    let mut presence = Presence::default();
    let mut tween = Tween::rest(0.0, Pace::Open, Curve::EaseOut);
    let mut spring_value = SpringValue::new(0.0, Spring::GENTLE);
    presence.show(start);
    tween.set(1.0, start, false);
    spring_value.set(1.0, start);
    let (mut p, mut t, mut s) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..100 {
        let now = start + TICK * i;
        if i == 12 || i == 19 {
            let before = (
                presence.level(now, false),
                tween.value(now, false),
                spring_value.value(now),
            );
            if i == 12 {
                presence.hide(now);
            } else {
                presence.show(now);
            }
            let target = if i == 12 { 0.0 } else { 1.0 };
            tween.set(target, now, false);
            spring_value.set(target, now);
            assert!((presence.level(now, false) - before.0).abs() < 1e-6);
            assert!((tween.value(now, false) - before.1).abs() < 1e-6);
            assert!((spring_value.value(now) - before.2).abs() < 1e-6);
        }
        p.push(presence.level(now, false));
        t.push(tween.value(now, false));
        s.push(spring_value.value(now));
    }
    bounded(&p);
    bounded(&t);
    bounded(&s);
}

#[test]
fn reduce_motion_settles_all_values() {
    let now = Instant::now();
    let mut tween = Tween::rest(0.0, Pace::Open, Curve::Smooth);
    tween.set(1.0, now, true);
    assert_eq!(tween.value(now, true), 1.0);
    let mut presence = Presence::default();
    presence.show(now);
    assert_eq!(presence.level(now, true), 1.0);
    presence.hide(now);
    assert_eq!(presence.level(now, true), 0.0);
    let mut spring_value = SpringValue::new(0.0, Spring::GENTLE);
    spring_value.set_reduced(1.0, now, true);
    assert_eq!(spring_value.value(now), 1.0);
    assert!(Settling::settled(&spring_value, now, true));
    let mut driver = Driver::at(now, true);
    driver.track(&tween);
    driver.track(&presence);
    driver.track(&spring_value);
    driver.keep(true);
    assert_eq!(driver.ask(true), FrameAsk::Idle);
    assert!(!driver.wants_frame());
}

#[test]
fn tokens_and_spring_presets_settle() {
    assert_eq!(
        [
            Dur::Instant.get(),
            Dur::Quick.get(),
            Dur::Standard.get(),
            Dur::Gentle.get(),
            Dur::Slow.get()
        ],
        [
            Duration::ZERO,
            Duration::from_millis(120),
            Duration::from_millis(220),
            Duration::from_millis(360),
            Duration::from_millis(600)
        ]
    );
    assert_eq!(
        [
            Pace::Close.token(),
            Pace::Glow.token(),
            Pace::Row.token(),
            Pace::Screen.token(),
            Pace::Change.token(),
            Pace::Open.token(),
            Pace::Launch.token()
        ],
        [
            Dur::Quick,
            Dur::Quick,
            Dur::Standard,
            Dur::Standard,
            Dur::Standard,
            Dur::Gentle,
            Dur::Slow
        ]
    );
    for spring in [Spring::SNAPPY, Spring::GENTLE, Spring::SOFT] {
        assert!(spring.mass > 0.0 && spring.stiffness > 0.0 && spring.damping > 0.0);
        let start = Instant::now();
        let mut value = SpringValue::new(0.0, spring);
        value.set(1.0, start);
        assert!(!value.settled(start));
        assert!(value.settled(start + Duration::from_millis(999)));
        assert_eq!(value.value(start + Duration::from_millis(999)), 1.0);
    }
    for (ease, expected) in [
        (Ease::Out, ease_out(0.4)),
        (Ease::In, ease_in(0.4)),
        (Ease::InOut, ease_in_out(0.4)),
        (Ease::Smooth, smooth(0.4)),
    ] {
        assert_eq!(ease.at(0.4), expected);
    }
}

#[test]
fn short_tweens_retain_velocity_and_bound_steps() {
    let start = Instant::now();
    for curve in [Curve::EaseOut, Curve::Smooth, Curve::Spring] {
        let mut tween = Tween::rest(0.0, Pace::Close, curve);
        tween.set(1.0, start, false);
        let mut values = Vec::new();
        for i in 0..45 {
            let at = start + TICK * i;
            if i == 7 {
                let dt = Duration::from_micros(100);
                let before = tween.value(at, false);
                let previous = tween.value(at - dt, false);
                tween.set(0.0, at, false);
                let after = tween.value(at + dt, false);
                assert!(((before - previous) - (after - before)).abs() < 0.0001);
            }
            values.push(tween.value(at, false));
        }
        bounded(&values);
    }
}

#[test]
fn frozen_clock_and_reduced_spring_never_request_frames() {
    let now = Instant::now();
    let mut driver = Driver::at(now, false);
    driver.keep(true);
    driver.held = true;
    assert_eq!(driver.ask(true), FrameAsk::Idle);
    assert!(!driver.wants_frame());
    let mut spring = SpringValue::new(0.0, Spring::SNAPPY);
    spring.set(1.0, now);
    assert_eq!(spring.value_reduced(now, true), 1.0);
    assert!(Settling::settled(&spring, now, true));
    let mut reduced = Driver::at(now, true);
    reduced.track(&spring);
    assert_eq!(reduced.ask(true), FrameAsk::Idle);
}

#[test]
fn retargeting_preserves_incoming_velocity() {
    let start = Instant::now();
    let at = start + Duration::from_millis(90);
    let dt = Duration::from_micros(100);
    let mut presence = Presence::default();
    presence.show(start);
    let p0 = presence.level(at - dt, false);
    let p1 = presence.level(at, false);
    presence.hide(at);
    let p2 = presence.level(at + dt, false);
    assert!(((p1 - p0) - (p2 - p1)).abs() < 0.0001);

    let mut spring = SpringValue::new(0.0, Spring::GENTLE);
    spring.set(1.0, start);
    let s0 = spring.value(at - dt);
    let s1 = spring.value(at);
    spring.set(0.0, at);
    let s2 = spring.value(at + dt);
    assert!(((s1 - s0) - (s2 - s1)).abs() < 0.0001);
}

#[test]
fn spring_solver_supports_damping_regimes() {
    let start = Instant::now();
    for damping in [10.0, 40.0, 60.0] {
        let mut value = SpringValue::new(
            0.0,
            Spring {
                stiffness: 400.0,
                damping,
                mass: 1.0,
            },
        );
        value.set(1.0, start);
        assert_eq!(value.value(start), 0.0);
        assert!(value.value(start + Duration::from_millis(100)).is_finite());
        assert!(value.settled(start + Duration::from_secs(4)));
    }
}
