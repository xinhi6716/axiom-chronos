use super::*;
use crate::desktop::theme::Tokened as _;
use crate::desktop::ux::seams::{ModeKind, ModeRef, PlazaAsk, PlazaSeat};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
fn mentioned(text: &str, seats: &[PlazaSeat]) -> Vec<Speaker> {
    seats
        .iter()
        .filter(|s| {
            text.split_whitespace().any(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric() && c != '@')
                    .eq_ignore_ascii_case(&format!("@{}", name(s.member)))
            })
        })
        .map(|s| s.member)
        .collect()
}
use gpui_kit::component::{
    input::{Input, InputEvent, InputState},
    Disableable as _,
};
use std::time::Duration;

pub struct PlazaPage {
    state: Entity<AppState>,
    input: Entity<InputState>,
    room: String,
    pending: bool,
    error: Option<String>,
}
impl PlazaPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(0, window, cx);
            }
            cx.notify();
        })
        .detach();
        Self {
            state,
            input,
            room: "plaza".into(),
            pending: false,
            error: None,
        }
    }
    fn send(&mut self, kind: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let original = text.clone();
        let Some(m) = service(&self.state, cx) else {
            return;
        };
        let seats = m.plaza(&self.room).seats;
        let ask = match kind {
            1 => PlazaAsk::Round { text },
            2 if seats.len() >= 3 => PlazaAsk::Debate {
                text,
                a: seats[0].member,
                b: seats[1].member,
                judge: seats[2].member,
            },
            2 => return,
            3 => PlazaAsk::Vote { question: text },
            _ => {
                let mentions = mentioned(&text, &seats);
                PlazaAsk::Say { text, mentions }
            }
        };
        self.pending = true;
        self.error = None;
        let room = self.room.clone();
        let task = cx.background_spawn(async move { m.plaza_send(&room, &ask) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(_) => this.input.update(cx, |i, cx| i.set_value("", window, cx)),
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.input
                            .update(cx, |i, cx| i.set_value(original, window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let pending = this
                .update(cx, |this, cx| {
                    if this.pending {
                        cx.notify();
                    }
                    this.pending
                })
                .unwrap_or(false);
            if !pending {
                break;
            }
        })
        .detach();
        cx.notify();
    }
    fn promote(&mut self, cx: &mut Context<Self>) {
        if let Some(context) = service(&self.state, cx)
            .and_then(|m| m.handoff(&ModeRef::Plaza(self.room.clone())).ok())
        {
            self.state
                .update(cx, |s, cx| s.open_mode(ModeKind::Wish, Some(context), cx));
        }
    }
}
impl Render for PlazaPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let Some(m) = service(&self.state, cx) else {
            return unavailable(self.state.clone(), lang, "plaza-page");
        };
        let room = m.plaza(&self.room);
        let input = self.input.read(cx).value().to_string();
        let complete = input.ends_with('@');
        let gold = cx.tokens().gold;
        let animate = self.pending && !self.state.read(cx).reduce_motion;
        let mut seats = room
            .seats
            .iter()
            .map(|seat| {
                let active = room.speaker == Some(seat.member);
                let budget = match (seat.spent_usd, seat.budget_usd) {
                    (Some(s), Some(b)) => format!("${s:.2} / ${b:.2}"),
                    _ => "—".into(),
                };
                let label = seat_label(seat.member);
                div().id(("plaza-seat-slot", seat.member as usize)).child(
                    div()
                        .id("plaza-seat")
                        .test_support()
                        .p(px(8.))
                        .child(if active {
                            let ring = div()
                                .id("plaza-speaker")
                                .test_support()
                                .p(px(4.))
                                .border_1()
                                .border_color(gold)
                                .rounded_full()
                                .child(label.clone().test_text(label.clone()));
                            if animate {
                                ring.with_animation(
                                    "plaza-speaking",
                                    Animation::new(Duration::from_millis(900)).repeat(),
                                    |el, k| {
                                        el.opacity(0.65 + 0.35 * (std::f32::consts::PI * k).sin())
                                    },
                                )
                                .into_any_element()
                            } else {
                                ring.into_any_element()
                            }
                        } else {
                            div()
                                .p(px(4.))
                                .child(label.clone().test_text(label))
                                .into_any_element()
                        })
                        .child(marked("plaza-budget", budget)),
                )
            })
            .collect::<Vec<_>>();
        let lower_seats = seats.split_off((seats.len() + 1) / 2);
        let messages = room
            .messages
            .into_iter()
            .map(|msg| {
                let m = m.clone();
                let room = self.room.clone();
                let id = msg.id;
                gpui_kit::component::h_flex()
                    .id(("plaza-message-slot", id as usize))
                    .gap(px(8.))
                    .child(marked(
                        "plaza-message",
                        format!(
                            "{} {}: {}",
                            glyph(msg.from),
                            display_name(msg.from, lang),
                            msg.text
                        ),
                    ))
                    .child(
                        Button::new(("pin", id as usize))
                            .label(tr(lang, "Pin"))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let _ = m.pin(&room, id);
                                cx.notify();
                            })),
                    )
            })
            .collect::<Vec<_>>();
        let completions = if complete {
            room.seats
                .iter()
                .map(|s| {
                    let member = s.member;
                    Button::new(("mention", member as usize))
                        .label(name(member))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let text = this.input.read(cx).value().to_string();
                            this.input.update(cx, |i, cx| {
                                i.set_value(format!("{}{} ", text, name(member)), window, cx)
                            });
                        }))
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        v_flex()
            .id("plaza-page")
            .test_support()
            .size_full()
            .p(px(20.))
            .gap(px(8.))
            .child(back(self.state.clone(), lang))
            .child(tr(lang, "Plaza"))
            .child(
                gpui_kit::component::h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .children(seats),
            )
            .child(v_flex().flex_1().overflow_y_scrollbar().children(messages))
            .child(
                gpui_kit::component::h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .children(lower_seats),
            )
            .when_some(room.last_route, |p, r| {
                p.child(marked("plaza-route-reason", r.reason))
            })
            .child(marked(
                "plaza-pins",
                room.pins
                    .into_iter()
                    .map(|p| p.text)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
            .when_some(self.error.clone(), |p, e| p.child(marked("plaza-error", e)))
            .children(completions)
            .child(
                Input::new(&self.input)
                    .id("plaza-input")
                    .disabled(self.pending),
            )
            .child(
                gpui_kit::component::h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("plaza-send")
                            .disabled(self.pending)
                            .label(tr(lang, "Send"))
                            .on_click(cx.listener(|this, _, window, cx| this.send(0, window, cx))),
                    )
                    .child(
                        Button::new("plaza-round")
                            .disabled(self.pending)
                            .label(tr(lang, "Round"))
                            .on_click(cx.listener(|this, _, window, cx| this.send(1, window, cx))),
                    )
                    .child(
                        Button::new("plaza-debate")
                            .disabled(self.pending)
                            .label(tr(lang, "Debate"))
                            .on_click(cx.listener(|this, _, window, cx| this.send(2, window, cx))),
                    )
                    .child(
                        Button::new("plaza-vote")
                            .disabled(self.pending)
                            .label(tr(lang, "Vote"))
                            .on_click(cx.listener(|this, _, window, cx| this.send(3, window, cx))),
                    ),
            )
            .child(
                Button::new("mode-promote")
                    .disabled(self.pending)
                    .label(tr(lang, "Make it a wish"))
                    .on_click(cx.listener(|this, _, _, cx| this.promote(cx))),
            )
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::ux::seams::{FakeModes, ModeEngine};
    use std::prelude::v1::test;
    #[test]
    fn mentions_use_only_seated_members_and_whole_names() {
        let modes = FakeModes::new();
        let seats = modes.plaza("plaza").seats;
        assert_eq!(
            mentioned("@astra, @ARGON hello @Astral", &seats),
            vec![Speaker::Astra, Speaker::Argon]
        );
        assert!(mentioned("mail@Astra or @Astral", &seats).is_empty());
    }
}
