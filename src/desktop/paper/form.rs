use super::*;
use crate::desktop::theme::Tokens;
use crate::desktop::ux::seams::{
    AccessWanted, Money, PassModels, Quartile, QuartileScheme, Requirements, ShortlistInput,
};
use gpui_kit::component::input::InputState;
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
    ("Pass 1 · Astra model", "第 1 輪 · Astra 模型"),
    ("Pass 2 · Fable model", "第 2 輪 · Fable 模型"),
    ("Pass 3 · Gemini model", "第 3 輪 · Gemini 模型"),
    ("Pass 4 · Fable model", "第 4 輪 · Fable 模型"),
    ("Save models", "儲存模型"),
];
fn tr(l: Lang, k: &'static str) -> &'static str {
    i18n::translate(l, TABLE, k)
}
pub struct Form {
    inputs: Vec<Entity<InputState>>,
    models: Vec<Entity<InputState>>,
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
            models: (0..4)
                .map(|_| cx.new(|cx| InputState::new(window, cx)))
                .collect(),
            tier: None,
            scheme: QuartileScheme::Sjr,
            access: AccessWanted::Either,
            no_fee: false,
        }
    }
    pub fn initialize_models(
        &self,
        models: &PassModels,
        window: &mut Window,
        cx: &mut Context<PaperPage>,
    ) {
        for (input, value) in self.models.iter().zip([
            &models.astra_draft,
            &models.fable_revise,
            &models.gemini_revise,
            &models.fable_style,
        ]) {
            input.update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
        }
    }
    fn pass_models(&self, cx: &App) -> PassModels {
        let value = |i: usize| self.models[i].read(cx).value().to_string();
        PassModels {
            astra_draft: value(0),
            fable_revise: value(1),
            gemini_revise: value(2),
            fable_style: value(3),
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
    pub fn render(
        &self,
        lang: Lang,
        models_loaded: bool,
        pending: bool,
        cx: &mut Context<PaperPage>,
    ) -> AnyElement {
        let t = cx.tokens();
        let caption = crate::desktop::theme::font(
            crate::desktop::theme::density(cx),
            crate::desktop::theme::TypeSize::Caption,
        );
        let fields = self.render_fields(lang, caption, &t, cx);
        let tiers = self.render_tiers(lang, &t, cx);
        let access = self.render_access(lang, &t, cx);
        let actions = self.render_actions(lang, models_loaded, pending, &t, cx);
        v_flex()
            .child(actions)
            .child(fields)
            .child(tiers)
            .child(access)
            .child(
                div()
                    .text_size(caption)
                    .line_height(caption * (4. / 3.))
                    .child(tr(lang, "JCR file path (optional)")),
            )
            .child(TextField::new("paper-jcr", &self.inputs[6], &t))
            .into_any_element()
    }
    fn render_fields(
        &self,
        lang: Lang,
        caption: Pixels,
        t: &Tokens,
        cx: &mut Context<PaperPage>,
    ) -> AnyElement {
        let mut columns = [v_flex().flex_1().min_w_0(), v_flex().flex_1().min_w_0()];
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
            let column = i / 5;
            columns[column] = std::mem::replace(&mut columns[column], v_flex()).child(
                v_flex()
                    .child(
                        div()
                            .text_size(caption)
                            .line_height(caption * (4. / 3.))
                            .child(tr(lang, label)),
                    )
                    .child(div().min_w_0().child(TextField::new(
                        ("paper-input", i),
                        &self.inputs[i],
                        t,
                    ))),
            );
        }
        for (i, label) in [
            "Pass 1 · Astra model",
            "Pass 2 · Fable model",
            "Pass 3 · Gemini model",
            "Pass 4 · Fable model",
        ]
        .into_iter()
        .enumerate()
        {
            let column = (i + 6) / 5;
            columns[column] = std::mem::replace(&mut columns[column], v_flex()).child(
                v_flex()
                    .child(
                        div()
                            .text_size(caption)
                            .line_height(caption * (4. / 3.))
                            .child(tr(lang, label)),
                    )
                    .child(div().min_w_0().child(TextField::new(
                        ("paper-model", i),
                        &self.models[i],
                        t,
                    ))),
            );
        }
        let [left, right] = columns;
        let fields = gpui_kit::component::h_flex()
            .items_start()
            .id("paper-form")
            .test_support()
            .gap(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Xs,
            ))
            .child(left)
            .child(right);
        fields.into_any_element()
    }
    fn render_tiers(&self, lang: Lang, t: &Tokens, cx: &mut Context<PaperPage>) -> AnyElement {
        let mut tiers = gpui_kit::component::h_flex()
            .flex_wrap()
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
                Button::new(("paper-tier", i), label, t)
                    .variant(if selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
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
                Button::new(("paper-scheme", i), if i == 0 { "SJR" } else { "JCR" }, t)
                    .variant(if self.scheme == scheme {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.scheme = scheme.clone();
                        cx.notify();
                    })),
            );
        }
        tiers.into_any_element()
    }
    fn render_access(&self, lang: Lang, t: &Tokens, cx: &mut Context<PaperPage>) -> AnyElement {
        let mut access =
            gpui_kit::component::h_flex()
                .flex_wrap()
                .gap(crate::desktop::theme::space(
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
                Button::new(("paper-access", i), label, t)
                    .variant(if self.access == a {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.access = a.clone();
                        cx.notify();
                    })),
            );
        }
        access = access.child(
            Button::new("paper-no-fee", tr(lang, "No fee"), t)
                .variant(if self.no_fee {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.form.no_fee = !this.form.no_fee;
                    cx.notify();
                })),
        );
        access.into_any_element()
    }
    fn render_actions(
        &self,
        lang: Lang,
        models_loaded: bool,
        pending: bool,
        t: &Tokens,
        cx: &mut Context<PaperPage>,
    ) -> AnyElement {
        gpui_kit::component::h_flex()
            .flex_wrap()
            .child(
                div()
                    .id("action:paper.form.save-models")
                    .test_support()
                    .child(
                        Button::new("paper-save-models", tr(lang, "Save models"), t)
                            .variant(if models_loaded && !pending {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.models_loaded || this.pending {
                                    return;
                                }
                                let models = this.form.pass_models(cx);
                                this.estimate = None;
                                this.ack = false;
                                this.estimate_epoch
                                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                this.ignore_status_estimate = true;
                                this.action(cx, move |s, id| {
                                    s.set_models(&id, models).map(|_| None)
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .id("action:paper.form.shortlist")
                    .test_support()
                    .child(
                        Button::new("paper-shortlist", tr(lang, "Find journals"), t).on_click(
                            cx.listener(move |this, _, _, cx| match this.form.request(cx) {
                                Ok((req, jcr)) => {
                                    this.estimate = None;
                                    this.estimate_stale = false;
                                    this.ack = false;
                                    this.estimate_epoch
                                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                    this.ignore_status_estimate = true;
                                    this.action(cx, move |s, id| {
                                        s.request_shortlist(
                                            &id,
                                            ShortlistInput {
                                                requirements: req,
                                                jcr,
                                            },
                                        )
                                        .map(|_| None)
                                    });
                                }
                                Err(()) => {
                                    this.error =
                                        Some(tr(lang, "Invalid APC amount or currency").into());
                                    cx.notify();
                                }
                            }),
                        ),
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
