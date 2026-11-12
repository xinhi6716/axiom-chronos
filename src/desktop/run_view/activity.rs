use super::*;
use crate::desktop::{
    live::activity::{self, LiveStage, Worker},
    ux::activity::{OpenActivity, Pulse},
};
use gpui_kit::base::TestTextExt as _;
use std::rc::Rc;

pub(super) fn heartbeat(p: Pulse, now_ms: i64, lang: Lang, t: &Tokens, d: Density) -> AnyElement {
    let text = activity::heartbeat_text(p, now_ms, lang);
    div()
        .id("run-heartbeat")
        .test_support()
        .text_size(theme::font(d, TypeSize::Caption))
        .text_color(t.muted)
        .child(text.clone().test_text(text))
        .into_any_element()
}
pub(super) fn live_stage_label(
    stage: &LiveStage,
    _now_ms: i64,
    _lang: Lang,
    t: &Tokens,
    d: Density,
) -> AnyElement {
    let text = format!("{} · {}", stage.name, stage.actor);
    div()
        .id("run-live-stage")
        .test_support()
        .text_size(theme::font(d, TypeSize::Caption))
        .text_color(t.text)
        .child(text.clone().test_text(text))
        .into_any_element()
}
pub(super) fn claude_card(
    a: &OpenActivity,
    _now_ms: i64,
    lang: Lang,
    t: &Tokens,
    d: Density,
) -> AnyElement {
    let text = activity::activity_text(a, lang);
    div()
        .id("run-claude-activity")
        .test_support()
        .w_full()
        .p(theme::space(d, Step::Sm))
        .border_1()
        .border_color(t.border)
        .text_size(theme::font(d, TypeSize::Caption))
        .text_color(t.text)
        .child(text.clone().test_text(text))
        .into_any_element()
}
pub(super) fn who_running(
    workers: Vec<(Worker, Option<SharedString>)>,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_watch: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
) -> AnyElement {
    let header = if lang == Lang::En {
        "Working now"
    } else {
        "正在工作"
    };
    let watch_label = if lang == Lang::En { "Watch" } else { "觀看" };
    v_flex()
        .id("who-running")
        .test_support()
        .w_full()
        .p(theme::space(d, Step::Sm))
        .gap(theme::space(d, Step::Sm))
        .border_1()
        .border_color(t.border)
        .text_size(theme::font(d, TypeSize::Caption))
        .text_color(t.text)
        .child(header.test_text(header))
        .children(
            workers
                .into_iter()
                .enumerate()
                .map(|(i, (worker, session))| {
                    let on_watch = on_watch.clone();
                    let text = format!("{} · {}", worker.who, worker.what);
                    h_flex()
                        .id(("who-row", i))
                        .test_support()
                        .w_full()
                        .items_center()
                        .gap(theme::space(d, Step::Sm))
                        .child(div().flex_1().min_w_0().child(text.clone().test_text(text)))
                        .when_some(session, |row, session| {
                            row.child(
                                Button::new(("watch-session", i))
                                    .small()
                                    .accessibility_label(watch_label)
                                    .child(watch_label.test_text(watch_label))
                                    .on_click(move |_, window, cx| {
                                        on_watch(session.clone(), window, cx)
                                    }),
                            )
                        })
                }),
        )
        .into_any_element()
}
