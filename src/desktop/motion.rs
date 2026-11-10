//! Motion for the whole app: the easing curves, the named durations, values that glide to a new
//! figure, things that appear and leave, and the driver that asks for frames.
//!
//! Every animation reads its time from here and asks for its next frame through a [`Driver`], so
//! three rules hold everywhere: a frame is requested only while something is still moving, a
//! settled screen asks for nothing and so uses no CPU, and `reduce_motion` shows every animation's
//! end state at once.
use gpui_kit::*;
use std::f32::consts::PI;
use std::time::{Duration, Instant};

/// Named durations. Everything in ordinary use stays at or under 300 ms; only the launch, a
/// ceremony that plays once, is longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// A dialog, overlay or toast leaving: out of the way fast.
    Close,
    /// Hover and selection glows fading in or out.
    Glow,
    /// A row arriving in a list.
    Row,
    /// A screen switch: the crossfade and its drift.
    Screen,
    /// A value, a count or a state colour moving to its new figure.
    Change,
    /// A dialog or the command palette opening, with its soft spring.
    Open,
    /// The launch ritual.
    Launch,
}

impl Pace {
    #[cfg(test)]
    pub const ALL: [Pace; 7] = [Pace::Close, Pace::Glow, Pace::Row, Pace::Screen, Pace::Change, Pace::Open, Pace::Launch];

    pub const fn duration(self) -> Duration {
        Duration::from_millis(match self {
            Pace::Close => 120,
            Pace::Glow => 160,
            Pace::Row => 220,
            Pace::Screen => 240,
            Pace::Change => 260,
            Pace::Open => 300,
            Pace::Launch => 2400,
        })
    }

    pub fn seconds(self) -> f32 {
        self.duration().as_secs_f32()
    }
}

/// The part of `[from, to]` that `s` has covered, 0 before it and 1 after it.
pub fn seg(s: f32, from: f32, to: f32) -> f32 {
    if to <= from {
        return if s >= to { 1.0 } else { 0.0 };
    }
    ((s - from) / (to - from)).clamp(0.0, 1.0)
}

pub fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

/// Blends two colours through RGB, so a blend never swings through an unrelated hue.
pub fn mix(a: Hsla, b: Hsla, x: f32) -> Hsla {
    let x = x.clamp(0.0, 1.0);
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    Rgba { r: lerp(a.r, b.r, x), g: lerp(a.g, b.g, x), b: lerp(a.b, b.b, x), a: lerp(a.a, b.a, x) }.into()
}

/// The gentle ease-out everything uses by default: quick to answer, slow to arrive.
pub fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

/// Accelerating away: for things that leave.
pub fn ease_in(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x
}

/// Slow at both ends.
pub fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A soft spring: starts as briskly as the ease-out, passes its rest by about two percent, and
/// settles back exactly at 1. It never bounces.
pub fn spring(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3) * (PI * x).cos()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    EaseOut,
    Smooth,
    Spring,
}

impl Curve {
    pub fn at(self, x: f32) -> f32 {
        match self {
            Curve::EaseOut => ease_out(x),
            Curve::Smooth => smooth(x),
            Curve::Spring => spring(x),
        }
    }
}

/// How every dialog enters and leaves: a short fade with a little travel, over [`Pace::Close`],
/// which `theme::apply` sets as the kit's dialog duration. Its content then settles with the
/// spring, through [`settle_in`].
pub const DIALOG_ENTRANCE: gpui_kit::component::dialog::DialogEntrance = gpui_kit::component::dialog::DialogEntrance::FadeSlide;

/// How far a dialog's content rises as it settles.
const SETTLE_RISE: f32 = 10.0;

/// Content that settles into place with the soft spring as its dialog opens. gpui's animation
/// element drives it: it asks for frames only until it lands, and under reduce motion (which the
/// app keeps in step with its own setting) it is simply there.
pub fn settle_in(id: impl Into<ElementId>, content: impl IntoElement) -> impl IntoElement {
    div()
        .relative()
        .child(content)
        .with_animation(id, Animation::new(Pace::Open.duration()).with_easing(|x| Curve::Spring.at(x)), |el, k| {
            el.top(px(SETTLE_RISE * (1.0 - k))).opacity(lerp(0.3, 1.0, k).min(1.0))
        })
}

/// The motion clock. Demo scenes freeze it, so one moment of an animation can be captured; a
/// frozen clock never asks for frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    frozen: Option<Instant>,
}

impl Global for Clock {}

pub fn now(cx: &App) -> Instant {
    cx.try_global::<Clock>().and_then(|clock| clock.frozen).unwrap_or_else(Instant::now)
}

pub fn is_frozen(cx: &App) -> bool {
    cx.try_global::<Clock>().is_some_and(|clock| clock.frozen.is_some())
}

/// Demo only: holds every animation at `at`.
pub fn freeze(at: Instant, cx: &mut App) {
    cx.set_global(Clock { frozen: Some(at) });
    cx.refresh_windows();
}

/// Anything that moves for a while and then rests.
pub trait Settling {
    fn settled(&self, now: Instant, reduce: bool) -> bool;
}

fn progress(since: Instant, pace: Pace, now: Instant) -> f32 {
    now.saturating_duration_since(since).as_secs_f32() / pace.seconds()
}

/// A figure that glides to each new value instead of jumping: a bar's width, a count, a blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Option<Instant>,
    pace: Pace,
    curve: Curve,
}

impl Tween {
    /// At rest on `value`; the first [`Tween::set`] to another value starts it moving.
    pub fn rest(value: f32, pace: Pace, curve: Curve) -> Self {
        Self { from: value, to: value, start: None, pace, curve }
    }

    /// How far through its move it is, 0..1; 1 at rest.
    pub fn progress(&self, now: Instant, reduce: bool) -> f32 {
        match self.start {
            Some(start) if !reduce => progress(start, self.pace, now).min(1.0),
            _ => 1.0,
        }
    }

    pub fn value(&self, now: Instant, reduce: bool) -> f32 {
        let p = self.progress(now, reduce);
        if p >= 1.0 {
            return self.to;
        }
        lerp(self.from, self.to, self.curve.at(p))
    }

    /// Heads for `to` from wherever it is now, so a change mid-move never jumps.
    pub fn set(&mut self, to: f32, now: Instant, reduce: bool) {
        if to == self.to {
            return;
        }
        if reduce {
            *self = Self::rest(to, self.pace, self.curve);
            return;
        }
        self.from = self.value(now, reduce);
        self.to = to;
        self.start = Some(now);
    }

    /// Jumps to `value` with no motion, as when the subject itself changes.
    pub fn jump(&mut self, value: f32) {
        *self = Self::rest(value, self.pace, self.curve);
    }
}

impl Settling for Tween {
    fn settled(&self, now: Instant, reduce: bool) -> bool {
        self.progress(now, reduce) >= 1.0
    }
}

/// Something that appears and leaves, such as an overlay: it opens with the soft spring over
/// [`Pace::Open`] and closes quickly over [`Pace::Close`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Presence {
    shown: bool,
    since: Option<Instant>,
}

impl Presence {
    pub fn show(&mut self, now: Instant) {
        if !self.shown {
            self.shown = true;
            self.since = Some(now);
        }
    }

    pub fn hide(&mut self, now: Instant) {
        if self.shown {
            self.shown = false;
            self.since = Some(now);
        }
    }

    fn pace(&self) -> Pace {
        if self.shown {
            Pace::Open
        } else {
            Pace::Close
        }
    }

    /// 0 gone, 1 fully present. While opening the spring passes 1 by a hair before settling.
    pub fn level(&self, now: Instant, reduce: bool) -> f32 {
        let end = if self.shown { 1.0 } else { 0.0 };
        let Some(since) = self.since.filter(|_| !reduce) else { return end };
        let p = progress(since, self.pace(), now);
        if p >= 1.0 {
            return end;
        }
        if self.shown {
            spring(p)
        } else {
            1.0 - ease_in(p)
        }
    }

    /// Whether anything is drawn: while shown, and while it is still leaving.
    pub fn visible(&self, now: Instant, reduce: bool) -> bool {
        self.shown || !self.settled(now, reduce)
    }
}

impl Settling for Presence {
    fn settled(&self, now: Instant, reduce: bool) -> bool {
        reduce || self.since.is_none_or(|since| progress(since, self.pace(), now) >= 1.0)
    }
}

/// Asks for frames for one view. A render makes one, reads every moving value through it, and
/// ends with [`Driver::finish`], which requests the next frame only if something is still moving.
/// So an animation that settles stops costing anything on the very next frame.
#[derive(Clone, Copy, Debug)]
pub struct Driver {
    now: Instant,
    reduce: bool,
    held: bool,
    moving: bool,
}

impl Driver {
    pub fn new(reduce: bool, cx: &App) -> Self {
        Self { now: now(cx), reduce, held: is_frozen(cx), moving: false }
    }

    /// A driver at a given moment, for tests.
    #[cfg(test)]
    pub fn at(now: Instant, reduce: bool) -> Self {
        Self { now, reduce, held: false, moving: false }
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    pub fn reduce(&self) -> bool {
        self.reduce
    }

    pub fn track(&mut self, item: &impl Settling) {
        if !item.settled(self.now, self.reduce) {
            self.moving = true;
        }
    }

    pub fn value(&mut self, tween: &Tween) -> f32 {
        self.track(tween);
        tween.value(self.now, self.reduce)
    }

    pub fn level(&mut self, presence: &Presence) -> f32 {
        self.track(presence);
        presence.level(self.now, self.reduce)
    }

    /// For motion that lasts as long as its subject, such as running work breathing. The caller
    /// says whether it is moving; under `reduce_motion` it never is.
    pub fn keep(&mut self, moving: bool) {
        if moving && !self.reduce {
            self.moving = true;
        }
    }

    /// Whether the view needs another frame. Never while settled, and never on a frozen clock.
    pub fn wants_frame(&self) -> bool {
        self.moving && !self.held
    }

    pub fn finish(self, window: &mut Window) -> bool {
        let wants = self.wants_frame();
        if wants {
            window.request_animation_frame();
        }
        wants
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    fn steps() -> impl Iterator<Item = f32> {
        (0..=200).map(|i| i as f32 / 200.0)
    }

    #[test]
    fn every_curve_starts_at_rest_and_lands_exactly() {
        for curve in [Curve::EaseOut, Curve::Smooth, Curve::Spring] {
            assert!(curve.at(0.0).abs() < 1e-6, "{curve:?} at 0: {}", curve.at(0.0));
            assert!((curve.at(1.0) - 1.0).abs() < 1e-6, "{curve:?} at 1: {}", curve.at(1.0));
            // Outside the range they hold their ends rather than running away.
            assert!(curve.at(-0.5).abs() < 1e-6 && (curve.at(1.7) - 1.0).abs() < 1e-6, "{curve:?}");
        }
    }

    #[test]
    fn the_ease_out_is_gentle_monotone_and_answers_quickly() {
        let mut last = 0.0;
        for x in steps() {
            let y = ease_out(x);
            assert!(y >= last - 1e-6, "ease_out falls at {x}");
            assert!(y <= 1.0 + 1e-6, "ease_out overshoots at {x}");
            last = y;
        }
        // A third of the way in, most of the distance is covered: the click feels answered.
        assert!(ease_out(1.0 / 3.0) > 0.65);
        // And the last tenth moves very little: it arrives softly.
        assert!(1.0 - ease_out(0.9) < 0.002);
    }

    #[test]
    fn the_spring_overshoots_a_little_once_and_never_bounces() {
        let peak = steps().map(spring).fold(0.0f32, f32::max);
        assert!(peak > 1.005, "a spring with no overshoot is just an ease: {peak}");
        assert!(peak < 1.04, "overshoot must stay a soft settle: {peak}");
        // One pass beyond rest, then a monotone return: no second crossing below 1.
        let values: Vec<f32> = steps().map(spring).collect();
        let first_over = values.iter().position(|&y| y > 1.0).expect("it passes rest");
        assert!(values[first_over..].iter().all(|&y| y >= 1.0 - 1e-6), "the spring bounces back below rest");
        let crest = values.iter().cloned().enumerate().fold((0, 0.0), |a, (i, y)| if y > a.1 { (i, y) } else { a }).0;
        assert!(values[crest..].windows(2).all(|w| w[1] <= w[0] + 1e-6), "after its crest it only settles");
        // Before the crest it only rises.
        assert!(values[..crest].windows(2).all(|w| w[1] >= w[0] - 1e-6));
    }

    #[test]
    fn durations_are_named_and_stay_under_300ms_except_the_launch() {
        for pace in Pace::ALL {
            let ms = pace.duration().as_millis();
            if pace == Pace::Launch {
                assert!((2000..=2500).contains(&ms), "launch {ms} ms");
            } else {
                assert!((80..=300).contains(&ms), "{pace:?} {ms} ms");
            }
        }
        assert!(Pace::Close.duration() < Pace::Open.duration(), "closing is quicker than opening");
    }

    #[test]
    fn seg_and_mix_hold_their_ends() {
        assert_eq!(seg(0.2, 0.5, 1.0), 0.0);
        assert_eq!(seg(0.75, 0.5, 1.0), 0.5);
        assert_eq!(seg(4.0, 0.5, 1.0), 1.0);
        assert_eq!(seg(1.0, 1.0, 1.0), 1.0);
        let (a, b): (Hsla, Hsla) = (rgb(0x0c1826).into(), rgb(0xdcc28a).into());
        let half = Rgba::from(mix(a, b, 0.5));
        assert!((half.r - (Rgba::from(a).r + Rgba::from(b).r) / 2.0).abs() < 1e-3);
        assert_eq!(Rgba::from(mix(a, b, 7.0)), Rgba::from(b));
    }

    #[test]
    fn a_tween_glides_to_its_target_and_retargets_without_jumping() {
        let t0 = Instant::now();
        let mut tween = Tween::rest(2.0, Pace::Change, Curve::EaseOut);
        assert_eq!(tween.value(t0, false), 2.0);
        assert!(tween.settled(t0, false));
        tween.set(6.0, t0, false);
        assert!(!tween.settled(t0, false));
        assert_eq!(tween.value(t0, false), 2.0, "a move starts where the value was");
        let mid = t0 + Pace::Change.duration() / 2;
        let at_mid = tween.value(mid, false);
        assert!(at_mid > 2.0 && at_mid < 6.0);
        // Turning back halfway starts from the halfway value.
        tween.set(3.0, mid, false);
        assert!((tween.value(mid, false) - at_mid).abs() < 1e-5);
        let end = mid + Pace::Change.duration();
        assert_eq!(tween.value(end, false), 3.0);
        assert!(tween.settled(end, false));
        // Setting the same target again does not restart it.
        tween.set(3.0, end, false);
        assert!(tween.settled(end, false));
    }

    #[test]
    fn reduce_motion_shows_every_end_state_at_once() {
        let t0 = Instant::now();
        let mut tween = Tween::rest(0.0, Pace::Change, Curve::Spring);
        tween.set(10.0, t0, true);
        assert_eq!(tween.value(t0, true), 10.0);
        assert!(tween.settled(t0, true));
        // A move already under way also shows its end when reduce motion is turned on.
        let mut moving = Tween::rest(0.0, Pace::Change, Curve::EaseOut);
        moving.set(4.0, t0, false);
        assert_eq!(moving.value(t0, true), 4.0);

        let mut overlay = Presence::default();
        overlay.show(t0);
        assert_eq!(overlay.level(t0, true), 1.0);
        assert!(overlay.settled(t0, true));
        overlay.hide(t0);
        assert_eq!(overlay.level(t0, true), 0.0);
        assert!(!overlay.visible(t0, true), "a closed overlay is gone at once");

        let mut driver = Driver::at(t0, true);
        assert_eq!(driver.value(&moving), 4.0);
        driver.keep(true);
        assert!(!driver.wants_frame(), "nothing moves under reduce motion");
    }

    #[test]
    fn an_overlay_opens_with_the_spring_and_closes_quickly() {
        let t0 = Instant::now();
        let mut overlay = Presence::default();
        assert_eq!(overlay.level(t0, false), 0.0);
        assert!(!overlay.visible(t0, false));
        overlay.show(t0);
        assert_eq!(overlay.level(t0, false), 0.0);
        let peak = (1..30)
            .map(|i| overlay.level(t0 + Pace::Open.duration() * i / 30, false))
            .fold(0.0f32, f32::max);
        assert!(peak > 1.0 && peak < 1.04, "opening settles softly: {peak}");
        let open = t0 + Pace::Open.duration();
        assert_eq!(overlay.level(open, false), 1.0);
        overlay.hide(open);
        assert!(overlay.visible(open, false), "it stays drawn while it leaves");
        let half = overlay.level(open + Pace::Close.duration() / 2, false);
        assert!(half > 0.0 && half < 1.0);
        let gone = open + Pace::Close.duration();
        assert_eq!(overlay.level(gone, false), 0.0);
        assert!(!overlay.visible(gone, false));
    }

    #[test]
    fn the_driver_asks_for_frames_only_until_everything_settles() {
        let t0 = Instant::now();
        let mut bar = Tween::rest(0.0, Pace::Change, Curve::EaseOut);
        let mut overlay = Presence::default();
        // At rest nothing asks for a frame: an idle screen costs no CPU.
        let mut idle = Driver::at(t0, false);
        idle.value(&bar);
        idle.level(&overlay);
        assert!(!idle.wants_frame());

        bar.set(1.0, t0, false);
        overlay.show(t0);
        let mut frames = 0;
        let mut at = t0;
        loop {
            let mut driver = Driver::at(at, false);
            driver.value(&bar);
            driver.level(&overlay);
            if !driver.wants_frame() {
                break;
            }
            frames += 1;
            assert!(frames < 100, "the driver never stopped asking for frames");
            at += Duration::from_millis(16);
        }
        // About 300 ms at 60 frames a second, and then it stops by itself.
        assert!((17..=21).contains(&frames), "{frames} frames");
        for later in [Duration::from_secs(1), Duration::from_secs(600)] {
            let mut driver = Driver::at(t0 + later, false);
            driver.value(&bar);
            driver.level(&overlay);
            assert!(!driver.wants_frame(), "still asking {later:?} later");
        }
    }

    #[test]
    fn continuous_motion_keeps_the_driver_going_only_while_it_lasts() {
        let t0 = Instant::now();
        let mut driver = Driver::at(t0, false);
        driver.keep(false);
        assert!(!driver.wants_frame());
        driver.keep(true);
        assert!(driver.wants_frame());
    }
}
