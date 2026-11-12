use super::super::{
    i18n::{fill, translate, Lang, Table},
    model::{Action, AppState},
    motion,
    settings::Store,
    theme::{self, Density, Step, Tokens, TypeSize},
    ux::seams::{FastCost, TurnPolicy, TurnSettings},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{switch::Switch, tooltip::Tooltip, v_flex},
    *,
};
use std::{
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const T: Table = &[
    ("Extreme mode", "極限模式"),
    ("Extreme mode: on", "極限模式：開"),
    ("Extreme mode: off", "極限模式：關"),
    (
        "Applies from the next Codex turn; a turn already running keeps its setting.",
        "從下一個 Codex 回合起生效；進行中的回合維持原設定。",
    ),
    ("Fast usage: {}× Codex quota", "快速用量：Codex 額度 {} 倍"),
    ("Fast usage cost: —", "快速用量成本：—"),
    ("Could not save extreme mode: {}", "無法儲存極限模式：{}"),
];
#[derive(Default)]
pub struct PrefsTurnPolicy(Mutex<TurnSettings>);
impl TurnPolicy for PrefsTurnPolicy {
    fn current(&self) -> TurnSettings {
        *self.0.lock().unwrap()
    }
    fn set_extreme(&self, on: bool) -> TurnSettings {
        let mut s = self.0.lock().unwrap();
        if s.extreme != on {
            s.extreme = on;
            s.revision += 1;
        }
        *s
    }
    fn begin_turn(&self) -> TurnSettings {
        self.current()
    }
    fn fast_cost(&self) -> Option<FastCost> {
        None
    }
}
pub fn save_failure(lang: Lang, reason: &str) -> String {
    fill(translate(lang, T, "Could not save extreme mode: {}"), &[reason])
}
pub fn install_policy(state: &mut AppState, cx: &mut App) {
    match &state.ux.seams.turn_policy {
        Some(policy) => {
            let on = policy.current().extreme;
            Store::edit(cx, |s| s.ux.extreme = on);
        }
        None => {
            let policy = PrefsTurnPolicy::default();
            policy.set_extreme(Store::current(cx).ux.extreme);
            state.ux.seams.turn_policy = Some(Arc::new(policy));
        }
    }
}
impl AppState {
    pub fn set_extreme(&mut self, on: bool, cx: &mut Context<Self>) {
        let was_on = Store::current(cx).ux.extreme;
        if let Some(policy) = &self.ux.seams.turn_policy {
            if let Err(e) = policy.try_set_extreme(on) {
                let lang = cx.try_global::<Lang>().copied().unwrap_or(Lang::En);
                self.report(Action::Engine, None, save_failure(lang, &e.message));
                self.ux.extreme_flash = None;
                cx.notify();
                return;
            }
        }
        Store::edit(cx, |s| s.ux.extreme = on);
        self.ux.extreme_flash = if on && !was_on && !self.reduce_motion {
            Some(Instant::now())
        } else {
            None
        };
        cx.notify();
    }
}
pub fn extreme_on(cx: &App) -> bool {
    Store::current(cx).ux.extreme
}
pub fn cinnabar(t: &Tokens) -> Hsla {
    motion::mix(t.fail, t.gold, 0.2)
}
pub const FLASH: Duration = Duration::from_millis(600);
pub fn flash_level(flash: Option<Instant>, now: Instant, reduce: bool) -> f32 {
    if reduce {
        return 0.;
    }
    flash
        .map(|start| {
            1. - (now.saturating_duration_since(start).as_secs_f32() / FLASH.as_secs_f32())
                .clamp(0., 1.)
        })
        .unwrap_or(0.)
}
pub fn strip_toggle(
    on: bool,
    flash: f32,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_toggle: Rc<dyn Fn(bool, &mut Window, &mut App)>,
) -> AnyElement {
    let color = if on {
        motion::mix(cinnabar(t), t.gold, flash.clamp(0., 1.) * 0.3)
    } else {
        t.muted
    };
    let side = theme::font(d, TypeSize::Title);
    let tip = translate(
        lang,
        T,
        if on {
            "Extreme mode: on"
        } else {
            "Extreme mode: off"
        },
    );
    div()
        .id("extreme-toggle")
        .test_support()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .on_click(move |_, window, cx| on_toggle(!on, window, cx))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let s = f32::from(side);
                    let x = f32::from(bounds.origin.x);
                    let y = f32::from(bounds.origin.y);
                    let points: Vec<_> = [
                        (0.58, 0.06),
                        (0.19, 0.55),
                        (0.46, 0.55),
                        (0.34, 0.94),
                        (0.82, 0.39),
                        (0.54, 0.39),
                        (0.58, 0.06),
                    ]
                    .into_iter()
                    .map(|(a, b)| point(px(x + a * s), px(y + b * s)))
                    .collect();
                    let mut path = PathBuilder::stroke(px(1.4));
                    path.add_polygon(&points, false);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .size(side),
        )
        .into_any_element()
}
pub fn strip_current(on: bool, flash: f32, t: &Tokens) -> Option<AnyElement> {
    on.then(|| {
        div()
            .id("extreme-current")
            .test_support()
            .w_full()
            .h(px(2.))
            .bg(motion::mix(cinnabar(t), t.gold, flash.clamp(0., 1.) * 0.3))
            .into_any_element()
    })
}
pub fn settings_row(
    on: bool,
    fast: Option<FastCost>,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_toggle: Rc<dyn Fn(bool, &mut Window, &mut App)>,
) -> AnyElement {
    let description = translate(
        lang,
        T,
        "Applies from the next Codex turn; a turn already running keeps its setting.",
    );
    let cost = fast
        .map(|f| {
            fill(
                translate(lang, T, "Fast usage: {}× Codex quota"),
                &[&f.multiplier.to_string()],
            )
        })
        .unwrap_or_else(|| translate(lang, T, "Fast usage cost: —").to_owned());
    v_flex()
        .id("settings-extreme")
        .test_support()
        .gap(theme::space(d, Step::Xs))
        .py(theme::space(d, Step::Xs))
        .child(translate(lang, T, "Extreme mode").test_text(translate(lang, T, "Extreme mode")))
        .child(
            div()
                .text_size(theme::font(d, TypeSize::Caption))
                .text_color(t.muted)
                .child(description.test_text(description)),
        )
        .child(
            div()
                .text_size(theme::font(d, TypeSize::Caption))
                .text_color(t.muted)
                .child(cost.clone().test_text(cost)),
        )
        .child(
            Switch::new("extreme-switch")
                .checked(on)
                .accessibility_label(translate(lang, T, "Extreme mode"))
                .on_click(move |on, window, cx| on_toggle(*on, window, cx)),
        )
        .into_any_element()
}
