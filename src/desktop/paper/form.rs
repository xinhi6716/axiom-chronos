use super::*;
use crate::paper::{AccessWanted, Money, Quartile, QuartileScheme, Requirements};
use gpui_kit::component::input::{Input, InputState};
use std::path::PathBuf;
const TABLE: Table = &[
    ("Abstract", "摘要"),
    ("Keywords (comma separated)", "關鍵字（逗號分隔）"),
    ("Quartile", "期刊分區"),
    ("Any", "不限"),
    ("Open access", "開放取用"),
    ("Subscription", "訂閱制"),
    ("Either", "皆可"),
    ("APC limit (minor units)", "文章處理費上限（最小貨幣單位）"),
    ("Currency", "幣別"),
    ("No fee", "不收費"),
    ("Categories (comma separated)", "類別（逗號分隔）"),
    (
        "Other requirements (comma separated)",
        "其他需求（逗號分隔）",
    ),
    ("JCR file path (optional)", "JCR 檔案路徑（選填）"),
    ("Find journals", "尋找期刊"),
    ("Invalid APC amount or currency", "文章處理費金額或幣別無效"),
];
fn tr(l: Lang, k: &'static str) -> &'static str {
    i18n::translate(l, TABLE, k)
}
pub struct Form {
    inputs: Vec<Entity<InputState>>,
    tier: Option<Quartile>,
    scheme: QuartileScheme,
    access: AccessWanted,
    no_fee: bool,
}
impl Form {
    pub fn new(window: &mut Window, cx: &mut Context<PaperPage>) -> Self {
        let inputs = (0..7)
            .map(|_| cx.new(|cx| InputState::new(window, cx)))
            .collect();
        Self {
            inputs,
            tier: None,
            scheme: QuartileScheme::Sjr,
            access: AccessWanted::Either,
            no_fee: false,
        }
    }
    pub fn initialize(
        &mut self,
        req: Option<&Requirements>,
        question: Option<&str>,
        window: &mut Window,
        cx: &mut Context<PaperPage>,
    ) {
        if let Some(r) = req {
            self.tier = r.max_quartile.clone();
            self.scheme = r.tier_scheme.clone();
            self.access = r.access.clone();
            self.no_fee = r.require_no_fee;
            let vals = [
                r.abstract_text.clone(),
                r.keywords.join(", "),
                r.apc_limit
                    .as_ref()
                    .map(|m| m.amount_minor.to_string())
                    .unwrap_or_default(),
                r.apc_limit
                    .as_ref()
                    .map(|m| m.currency.clone())
                    .unwrap_or_default(),
                r.categories.join(", "),
                r.free_text.join(", "),
            ];
            for (input, value) in self.inputs.iter().zip(vals) {
                input.update(cx, |i, cx| i.set_value(value, window, cx));
            }
        } else if let Some(q) = question {
            self.inputs[0].update(cx, |i, cx| i.set_value(q, window, cx));
        }
    }
    fn request(&self, cx: &App) -> Result<(Requirements, Option<PathBuf>), ()> {
        let v: Vec<String> = self
            .inputs
            .iter()
            .map(|i| i.read(cx).value().to_string())
            .collect();
        convert(
            &v,
            self.tier.clone(),
            self.scheme.clone(),
            self.access.clone(),
            self.no_fee,
        )
    }
    pub fn render(&self, lang: Lang, cx: &mut Context<PaperPage>) -> AnyElement {
        let mut fields =
            v_flex()
                .id("paper-form")
                .test_support()
                .gap(crate::desktop::theme::space(
                    crate::desktop::theme::density(cx),
                    crate::desktop::theme::Step::Xs,
                ));
        for (i, label) in [
            "Abstract",
            "Keywords (comma separated)",
            "APC limit (minor units)",
            "Currency",
            "Categories (comma separated)",
            "Other requirements (comma separated)",
        ]
        .iter()
        .enumerate()
        {
            fields = fields.child(
                gpui_kit::component::h_flex()
                    .gap(crate::desktop::theme::space(
                        crate::desktop::theme::density(cx),
                        crate::desktop::theme::Step::Xs,
                    ))
                    .child(tr(lang, label))
                    .child(Input::new(&self.inputs[i]).id(("paper-input", i))),
            );
        }
        let mut tiers = gpui_kit::component::h_flex()
            .gap(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Xs,
            ))
            .child(tr(lang, "Quartile"));
        for (i, q) in [
            None,
            Some(Quartile::Q1),
            Some(Quartile::Q2),
            Some(Quartile::Q3),
            Some(Quartile::Q4),
        ]
        .into_iter()
        .enumerate()
        {
            let label = [tr(lang, "Any"), "Q1", "Q2", "Q3", "Q4"][i];
            let selected = self.tier == q;
            tiers = tiers.child(
                Button::new(("paper-tier", i))
                    .label(label)
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.tier = q.clone();
                        cx.notify();
                    })),
            );
        }
        for (i, scheme) in [QuartileScheme::Sjr, QuartileScheme::Jcr]
            .into_iter()
            .enumerate()
        {
            tiers = tiers.child(
                Button::new(("paper-scheme", i))
                    .label(if i == 0 { "SJR" } else { "JCR" })
                    .selected(self.scheme == scheme)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.scheme = scheme.clone();
                        cx.notify();
                    })),
            );
        }
        let mut access = gpui_kit::component::h_flex().gap(crate::desktop::theme::space(
            crate::desktop::theme::density(cx),
            crate::desktop::theme::Step::Xs,
        ));
        for (i, a) in [
            AccessWanted::OpenAccess,
            AccessWanted::Subscription,
            AccessWanted::Either,
        ]
        .into_iter()
        .enumerate()
        {
            let label = tr(lang, ["Open access", "Subscription", "Either"][i]);
            access = access.child(
                Button::new(("paper-access", i))
                    .label(label)
                    .selected(self.access == a)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.access = a.clone();
                        cx.notify();
                    })),
            );
        }
        access = access.child(
            Button::new("paper-no-fee")
                .label(tr(lang, "No fee"))
                .selected(self.no_fee)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.form.no_fee = !this.form.no_fee;
                    cx.notify();
                })),
        );
        fields
            .child(tiers)
            .child(access)
            .child(tr(lang, "JCR file path (optional)"))
            .child(Input::new(&self.inputs[6]).id("paper-jcr"))
            .child(
                Button::new("paper-shortlist")
                    .label(tr(lang, "Find journals"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| match this.form.request(cx) {
                            Ok((req, jcr)) => {
                                this.estimate = None;
                                this.ack = false;
                                this.action(cx, move |s, id| {
                                    s.request_shortlist(&id, req, jcr).map(|_| None)
                                });
                            }
                            Err(()) => {
                                this.error =
                                    Some(tr(lang, "Invalid APC amount or currency").into());
                                cx.notify();
                            }
                        }),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn convert(
    v: &[String],
    tier: Option<Quartile>,
    scheme: QuartileScheme,
    access: AccessWanted,
    no_fee: bool,
) -> Result<(Requirements, Option<PathBuf>), ()> {
    let split = |s: &str| {
        s.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let apc_limit = if v[2].trim().is_empty() {
        None
    } else {
        let currency = v[3].trim();
        if currency.is_empty() {
            return Err(());
        }
        Some(Money {
            amount_minor: v[2].trim().parse().map_err(|_| ())?,
            currency: currency.to_owned(),
        })
    };
    let jcr = (!v[6].trim().is_empty()).then(|| PathBuf::from(v[6].trim()));
    Ok((
        Requirements {
            abstract_text: v[0].clone(),
            keywords: split(&v[1]),
            max_quartile: tier,
            tier_scheme: scheme,
            access,
            apc_limit,
            require_no_fee: no_fee,
            categories: split(&v[4]),
            free_text: split(&v[5]),
        },
        jcr,
    ))
}
