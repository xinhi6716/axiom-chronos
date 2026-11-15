use super::super::ux::seams::{ApprovalCard, ApprovalReceipt, Book, Decision, Verdict};
use crate::base::Res;

impl Book {
    pub fn key_for(&mut self, card: &ApprovalCard) -> String {
        self.keys
            .entry((card.action_id.clone(), card.version))
            .or_insert_with(|| {
                self.next += 1;
                format!("{}:{}:{}", card.action_id, card.version, self.next)
            })
            .clone()
    }

    pub fn begin(&mut self, card: &ApprovalCard, verdict: Verdict) -> Option<Decision> {
        if matches!(&verdict, Verdict::Reject { reason } if reason.trim().is_empty()) {
            return None;
        }
        let key = self.key_for(card);
        if self.in_flight.contains(&key) || self.receipts.contains_key(&key) {
            return None;
        }
        self.errors.retain(|old_key, error| {
            !(error == "stale" && old_key.starts_with(&format!("{}:", card.action_id)))
        });
        self.errors.remove(&key);
        self.in_flight.insert(key.clone());
        Some(Decision {
            action_id: card.action_id.clone(),
            payload_version: card.version,
            key,
            verdict,
        })
    }

    pub fn finish(&mut self, key: &str, result: Res<ApprovalReceipt>) {
        self.in_flight.remove(key);
        match result {
            Ok(receipt) => {
                self.errors.remove(key);
                self.receipts.insert(key.to_owned(), receipt);
            }
            Err(error) if error.kind == "stale" => {
                self.keys.retain(|_, value| value != key);
                self.errors.insert(key.to_owned(), "stale".to_owned());
            }
            Err(error) => {
                self.errors.insert(key.to_owned(), error.message);
            }
        }
    }
}
use super::super::{
    i18n::{self, Lang, Table},
    theme::{self, Step},
    ux::seams::ApprovalOutcome,
};
use super::{AppState, Detail};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        button::Button,
        h_flex,
        input::{Input, InputState},
        v_flex, Disableable as _, Sizable as _,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::collections::HashMap;

// Snapshots live alongside the app's approval book, not the short-lived panel.
#[derive(Default)]
struct CardSnapshots(
    HashMap<EntityId, (WeakEntity<AppState>, HashMap<(String, u64), ApprovalCard>)>,
);
impl Global for CardSnapshots {}

pub fn decided_cards(state: &Entity<AppState>, cx: &App) -> Vec<ApprovalCard> {
    cx.try_global::<CardSnapshots>()
        .and_then(|snapshots| snapshots.0.get(&state.entity_id()))
        .map(|(_, cards)| cards.values().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn remove_failed_snapshot(
    cards: &mut HashMap<(String, u64), ApprovalCard>,
    card_id: &(String, u64),
    result: &Res<ApprovalReceipt>,
) {
    if result.is_err() {
        cards.remove(card_id);
    }
}

pub fn merge_cards(pending: Vec<ApprovalCard>, decided: Vec<ApprovalCard>) -> Vec<ApprovalCard> {
    let mut cards = Vec::new();
    for card in decided.into_iter().chain(pending) {
        if !cards.iter().any(|old: &ApprovalCard| {
            old.action_id == card.action_id && old.version == card.version
        }) {
            cards.push(card);
        }
    }
    cards
}

const TABLE: Table = &[
    ("Approve", "核准"),
    ("Reject", "拒絕"),
    ("Reason", "原因"),
    (
        "Changed since you looked. Review it again.",
        "內容已變更，請重新檢視。",
    ),
    ("Done: {}", "已執行：{}"),
    ("Rejected", "已拒絕"),
    ("Failed: {}", "失敗：{}"),
    ("Action", "動作"),
    ("Target", "目標"),
    ("Scope", "範圍"),
    ("Cost", "費用"),
];

pub fn submit(
    state: &mut AppState,
    card: &ApprovalCard,
    verdict: Verdict,
    cx: &mut Context<AppState>,
) {
    let Some(service) = state.ux.seams.approvals.clone() else {
        return;
    };
    let Some(decision) = state.ux.approvals.begin(card, verdict) else {
        return;
    };
    let key = decision.key.clone();
    if cx.try_global::<CardSnapshots>().is_none() {
        cx.set_global(CardSnapshots::default());
    }
    let state_id = cx.entity_id();
    let weak = cx.weak_entity();
    let card_id = (card.action_id.clone(), card.version);
    let snapshots = cx.global_mut::<CardSnapshots>();
    snapshots
        .0
        .retain(|_, (state, _)| state.upgrade().is_some());
    snapshots
        .0
        .entry(state_id)
        .or_insert_with(|| (weak, HashMap::new()))
        .1
        .insert(card_id.clone(), card.clone());
    let task = cx.background_spawn(async move { service.decide(&decision) });
    cx.notify();
    cx.spawn(async move |state, cx| {
        let result = task.await;
        state
            .update(cx, |state, cx| {
                if let Some((_, cards)) = cx.global_mut::<CardSnapshots>().0.get_mut(&state_id) {
                    remove_failed_snapshot(cards, &card_id, &result);
                }
                state.ux.approvals.finish(&key, result);
                cx.notify();
            })
            .ok();
    })
    .detach();
}

fn marked(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
        .into_any_element()
}

pub fn approval_card(
    state: &Entity<AppState>,
    card: &ApprovalCard,
    reason: &Entity<InputState>,
    cx: &Context<Detail>,
) -> AnyElement {
    let lang = *cx.global::<Lang>();
    let tr = |key| i18n::translate(lang, TABLE, key);
    let book = &state.read(cx).ux.approvals;
    let key = book.keys.get(&(card.action_id.clone(), card.version));
    let locked =
        key.is_some_and(|key| book.in_flight.contains(key) || book.receipts.contains_key(key));
    let receipt = key.and_then(|key| book.receipts.get(key));
    let error = key.and_then(|key| book.errors.get(key)).or_else(|| {
        book.errors
            .iter()
            .find(|(key, error)| {
                error.as_str() == "stale" && key.starts_with(&format!("{}:", card.action_id))
            })
            .map(|(_, error)| error)
    });
    let receipt_text = receipt
        .map(|r| match &r.outcome {
            ApprovalOutcome::Executed { summary } => i18n::fill(tr("Done: {}"), &[summary]),
            ApprovalOutcome::Rejected => tr("Rejected").to_owned(),
            ApprovalOutcome::Failed { reason } => i18n::fill(tr("Failed: {}"), &[reason]),
        })
        .or_else(|| {
            error.map(|e| {
                if e == "stale" {
                    tr("Changed since you looked. Review it again.").to_owned()
                } else {
                    i18n::fill(tr("Failed: {}"), &[e])
                }
            })
        });
    let approve_state = state.clone();
    let approve_card = card.clone();
    let reject_state = state.clone();
    let reject_card = card.clone();
    let reason_input = reason.clone();
    let reason_empty = reason.read(cx).value().trim().is_empty();
    let cost = card
        .cost_usd
        .map_or_else(|| "—".to_owned(), |n| format!("${n:.2}"));
    let d = theme::density(cx);
    v_flex()
        .id("approval-card")
        .test_support()
        .gap(theme::space(d, Step::Sm))
        .child(marked(
            "approval-action",
            format!("{}: {}", tr("Action"), card.action),
        ))
        .child(marked(
            "approval-target",
            format!("{}: {}", tr("Target"), card.target),
        ))
        .child(marked(
            "approval-scope",
            format!("{}: {}", tr("Scope"), card.scope),
        ))
        .child(marked("approval-cost", format!("{}: {}", tr("Cost"), cost)))
        .child(marked("approval-reason-label", tr("Reason").to_owned()))
        .child(
            Input::new(reason)
                .id("approval-reason")
                .small()
                .disabled(locked),
        )
        .child(
            h_flex()
                .gap(theme::space(d, Step::Sm))
                .child(
                    Button::new("approval-approve")
                        .small()
                        .tooltip(tr("Approve"))
                        .child(tr("Approve").test_text(tr("Approve")))
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            approve_state.update(cx, |state, cx| {
                                submit(state, &approve_card, Verdict::Approve, cx)
                            })
                        }),
                )
                .child(
                    Button::new("approval-reject")
                        .small()
                        .tooltip(tr("Reject"))
                        .child(tr("Reject").test_text(tr("Reject")))
                        .disabled(locked || reason_empty)
                        .on_click(move |_, _, cx| {
                            let reason = reason_input.read(cx).value().to_string();
                            reject_state.update(cx, |state, cx| {
                                submit(state, &reject_card, Verdict::Reject { reason }, cx)
                            });
                        }),
                ),
        )
        .when_some(receipt_text, |this, text| {
            this.child(marked("approval-receipt", text))
        })
        .into_any_element()
}
