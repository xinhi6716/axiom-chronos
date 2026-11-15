use super::github::{self, ImportError};
use super::optimizer::{self, OptimizerState};
use super::*;
use crate::desktop::i18n::Lang;
use crate::desktop::ux::seams::{GhRunner, ModeKind, OptimizerRunner};

fn optimize_with_timeout(
    runner: Arc<dyn OptimizerRunner>,
    original: String,
    lang: Lang,
    timeout: Duration,
) -> Result<crate::desktop::ux::seams::Proposal, OptimizerState> {
    let original_for_run = original.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(runner.propose(&original_for_run, lang.into()));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(proposal)) => Ok(proposal),
        Ok(Err(e)) => Err(OptimizerState::Failed {
            original,
            timeout: e.kind == "timeout",
            message: e.message,
        }),
        Err(_) => Err(OptimizerState::Failed {
            original,
            timeout: true,
            message: String::new(),
        }),
    }
}

fn import_with_timeout(
    runner: Arc<dyn GhRunner>,
    parsed: github::IssueRef,
    timeout: Duration,
) -> Result<github::IssueDraft, ImportError> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(github::import(runner.as_ref(), &parsed, timeout));
    });
    rx.recv_timeout(timeout)
        .unwrap_or(Err(ImportError::TimedOut))
}

fn adoption(state: &OptimizerState, lang: Lang) -> Option<String> {
    match state {
        OptimizerState::Proposed {
            original,
            proposal,
            answers,
        } => Some(optimizer::compose(original, proposal, answers, lang)),
        _ => None,
    }
}
fn dismiss(state: &mut OptimizerState) {
    *state = OptimizerState::Idle;
}

fn text(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    if lang == Lang::ZhTw {
        zh
    } else {
        en
    }
}

pub(super) struct OptimizerPanel {
    state: OptimizerState,
    answers: Vec<Entity<InputState>>,
    answer_subs: Vec<Subscription>,
    reference: Entity<InputState>,
    templates_open: bool,
    github_open: bool,
    github_running: bool,
    github_error: Option<ImportError>,
    generation: u64,
    github_generation: u64,
    optimizer_timeout: Duration,
    github_timeout: Duration,
}
impl OptimizerPanel {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Composer>) -> Self {
        Self {
            state: OptimizerState::Idle,
            answers: vec![],
            answer_subs: vec![],
            reference: cx.new(|cx| InputState::new(window, cx)),
            templates_open: false,
            github_open: false,
            github_running: false,
            github_error: None,
            generation: 0,
            github_generation: 0,
            optimizer_timeout: optimizer::OPTIMIZE_TIMEOUT,
            github_timeout: Duration::from_secs(20),
        }
    }
    pub(super) fn wish_locked(&self) -> bool {
        matches!(
            self.state,
            OptimizerState::Running | OptimizerState::Proposed { .. }
        )
    }
    pub(super) fn reset(&mut self) {
        self.generation += 1;
        self.github_generation += 1;
        self.state = OptimizerState::Idle;
        self.answers.clear();
        self.answer_subs.clear();
        self.templates_open = false;
        self.github_open = false;
        self.github_running = false;
        self.github_error = None;
    }
}
impl Composer {
    pub(super) fn mode_ui(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.state.read(cx).ux.seams.modes.is_none() {
            return None;
        }
        let current = super::super::settings::Store::current(cx).ux.last_mode;
        let lang = *cx.global::<Lang>();
        let t = cx.tokens();
        let d = theme::density(cx);
        let view = cx.entity().downgrade();
        Some(
            super::modes::mode_picker(current, None, lang, &t, d, move |mode, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    let typed = this.wish.read(cx).value().to_string();
                    this.state.update(cx, |s, cx| {
                        if mode != ModeKind::Wish {
                            s.set_composing(false, cx);
                        }
                        s.open_mode(mode, Some(typed), cx);
                        if mode == ModeKind::Wish {
                            s.ux.composer_prefill = None;
                        }
                    });
                });
            })
            .into_any_element(),
        )
    }
    fn run_optimizer(&mut self, original: String, window: &mut Window, cx: &mut Context<Self>) {
        let runner: Arc<dyn OptimizerRunner> = self
            .state
            .read(cx)
            .ux
            .seams
            .optimizer
            .clone()
            .unwrap_or_else(|| Arc::new(optimizer::ClaudeOptimizer));
        let lang = *cx.global::<Lang>();
        self.panel.generation += 1;
        let generation = self.panel.generation;
        self.panel.state = OptimizerState::Running;
        self.close_mention(cx);
        let original_for_run = original.clone();
        let timeout = self.panel.optimizer_timeout;
        self.panel.templates_open = false;
        self.panel.github_open = false;
        let task = cx.background_spawn(async move {
            optimize_with_timeout(runner, original_for_run, lang, timeout)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.panel.generation != generation {
                    return;
                }
                this.panel.state = match result {
                    Ok(proposal) => {
                        this.panel.answers = proposal
                            .ambiguities
                            .iter()
                            .map(|_| cx.new(|cx| InputState::new(window, cx)))
                            .collect();
                        this.panel.answer_subs = this
                            .panel
                            .answers
                            .iter()
                            .enumerate()
                            .map(|(i, answer)| {
                                cx.observe(answer, move |this, input, cx| {
                                    if let OptimizerState::Proposed { answers, .. } =
                                        &mut this.panel.state
                                    {
                                        answers[i] = input.read(cx).value().to_string();
                                    }
                                    cx.notify();
                                })
                            })
                            .collect();
                        OptimizerState::Proposed {
                            original,
                            answers: vec![String::new(); proposal.ambiguities.len()],
                            proposal,
                        }
                    }
                    Err(failed) => failed,
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn import_issue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = *cx.global::<Lang>();
        let reference = self.panel.reference.read(cx).value().to_string();
        let parsed = match github::parse_reference(&reference) {
            Ok(r) => r,
            Err(e) => {
                self.panel.github_error = Some(e);
                cx.notify();
                return;
            }
        };
        let runner: Arc<dyn GhRunner> = self
            .state
            .read(cx)
            .ux
            .seams
            .github
            .clone()
            .unwrap_or_else(|| Arc::new(github::RealGh));
        self.panel.github_generation += 1;
        let generation = self.panel.github_generation;
        self.panel.github_error = None;
        self.panel.github_running = true;
        let timeout = self.panel.github_timeout;
        let task = cx.background_spawn(async move { import_with_timeout(runner, parsed, timeout) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.panel.github_generation != generation {
                    return;
                }
                this.panel.github_running = false;
                match result {
                    Ok(draft) => {
                        this.wish.update(cx, |s, cx| {
                            s.set_value(github::draft_text(&draft, lang), window, cx)
                        });
                        this.panel.github_open = false;
                    }
                    Err(e) => this.panel.github_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn panel_ui(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let locked = self.panel.wish_locked();
        let mut root = v_flex().gap_2();
        root = root.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("wish-templates")
                        .small()
                        .disabled(locked || self.panel.github_running)
                        .label(text(lang, "Templates", "範本"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panel.templates_open = !this.panel.templates_open;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("optimizer-run")
                        .small()
                        .label(text(lang, "Optimize", "最佳化"))
                        .disabled(
                            locked
                                || self.panel.github_running
                                || self.wish.read(cx).value().trim().is_empty(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            let original = this.wish.read(cx).value().to_string();
                            this.run_optimizer(original, window, cx);
                        })),
                )
                .child(
                    Button::new("github-import")
                        .small()
                        .disabled(locked || self.panel.github_running)
                        .label(text(lang, "From GitHub issue", "從 GitHub issue 匯入"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panel.github_open = !this.panel.github_open;
                            cx.notify();
                        })),
                ),
        );
        if self.panel.templates_open {
            let mut menu = h_flex().gap_1().flex_wrap();
            for template in super::templates::templates() {
                let body = text(lang, template.body.0, template.body.1);
                menu = menu.child(
                    Button::new(template.id)
                        .small()
                        .label(text(lang, template.title.0, template.title.1))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let target = this.wish.clone();
                            if asks_before_discarding(&target.read(cx).value()) {
                                ask(
                                    Ask {
                                        title: text(lang, "Replace the wish?", "要取代願望嗎？"),
                                        detail: text(
                                            lang,
                                            "The template will replace what you wrote.",
                                            "範本會取代你寫的內容。",
                                        ),
                                        confirm: text(lang, "Replace", "取代"),
                                        back: text(lang, "Keep writing", "繼續寫"),
                                        danger: false,
                                        focus_confirm: false,
                                    },
                                    window,
                                    cx,
                                    move |window, cx| {
                                        target.update(cx, |s, cx| s.set_value(body, window, cx))
                                    },
                                );
                            } else {
                                target.update(cx, |s, cx| s.set_value(body, window, cx));
                            }
                            this.panel.templates_open = false;
                            cx.notify();
                        })),
                );
            }
            root = root.child(menu);
        }
        if self.panel.github_open {
            let mut import = v_flex()
                .gap_1()
                .child(div().child(text(lang, "Issue reference", "議題參照")))
                .child(
                    Input::new(&self.panel.reference)
                        .id("github-import-field")
                        .small(),
                )
                .child(
                    Button::new("github-import-go")
                        .small()
                        .label(text(lang, "Import", "匯入"))
                        .disabled(self.panel.github_running)
                        .on_click(cx.listener(|this, _, window, cx| this.import_issue(window, cx))),
                );
            if let Some(error) = &self.panel.github_error {
                let message = error.message(lang);
                import = import.child(
                    div()
                        .id("github-import-error")
                        .child(message.clone().test_text(message))
                        .test_support(),
                );
            }
            root = root.child(import);
        }
        match &self.panel.state {
            OptimizerState::Idle => {}
            OptimizerState::Running => root = root.child(text(lang, "Optimizing…", "正在最佳化…")),
            OptimizerState::Failed {
                timeout,
                message,
                ..
            } => {
                let msg = if *timeout {
                    text(lang, "Optimization timed out.", "最佳化逾時。").to_owned()
                } else {
                    format!(
                        "{}: {}",
                        text(lang, "Optimization failed", "最佳化失敗"),
                        message
                    )
                };
                root = root.child(
                    v_flex().gap_1().child(msg).child(
                        Button::new("optimizer-retry")
                            .small()
                            .label(text(lang, "Try again", "重試"))
                            .disabled(
                                self.panel.github_running
                                    || self.wish.read(cx).value().trim().is_empty(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                let original = this.wish.read(cx).value().to_string();
                                this.run_optimizer(original, window, cx)
                            })),
                    ),
                );
            }
            OptimizerState::Proposed {
                original,
                proposal,
                answers,
            } => {
                let mut panel = v_flex().gap_1();
                let values = answers.clone();
                for (i, question) in proposal.ambiguities.iter().enumerate() {
                    panel = panel.child(div().child(question.clone())).child(
                        Input::new(&self.panel.answers[i])
                            .id(("optimizer-answer", i))
                            .small(),
                    );
                }
                let composed = optimizer::compose(original, proposal, &values, lang);
                let diff = optimizer::diff(original, &composed)
                    .into_iter()
                    .map(|(kind, line)| {
                        format!(
                            "{} {line}",
                            match kind {
                                optimizer::DiffKind::Same => " ",
                                optimizer::DiffKind::Added => "+",
                                optimizer::DiffKind::Removed => "-",
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                panel = panel
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("optimizer-adopt")
                                    .small()
                                    .label(text(lang, "Adopt", "採用"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let unchanged = matches!(&this.panel.state, OptimizerState::Proposed { original, .. } if original == this.wish.read(cx).value().as_ref());
                                        if let Some(value) = adoption(&this.panel.state, lang).filter(|_| unchanged) {
                                            this.wish
                                                .update(cx, |s, cx| s.set_value(value, window, cx));
                                        }
                                        dismiss(&mut this.panel.state);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("optimizer-cancel")
                                    .small()
                                    .label(text(lang, "Keep original", "保留原文"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        dismiss(&mut this.panel.state);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("optimizer-diff")
                            .max_h(px(100.))
                            .overflow_y_scroll()
                            .child(diff.clone().test_text(diff))
                            .test_support(),
                    );
                root = root.child(panel);
            }
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        base::{Failure, Res},
        desktop::ux::seams::{GhOutcome, Proposal},
    };
    use std::{
        prelude::v1::test,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct BlockingOptimizer(AtomicUsize);
    impl OptimizerRunner for BlockingOptimizer {
        fn propose(&self, _: &str, _: crate::desktop::ux::seams::ProposalLang) -> Res<Proposal> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(150));
            }
            Ok(Proposal {
                goal: "goal".into(),
                scope: "scope".into(),
                limits: "limits".into(),
                acceptance: "accept".into(),
                ambiguities: vec![],
            })
        }
    }
    struct BlockingGh(AtomicUsize);
    impl GhRunner for BlockingGh {
        fn run(&self, _: &[String], _: Duration) -> GhOutcome {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(150));
            }
            GhOutcome::Exit {
                code: 0,
                stdout: r#"{"number":1,"title":"title","body":"body","url":"url","labels":[]}"#
                    .into(),
                stderr: String::new(),
            }
        }
    }
    #[test]
    fn blocking_runners_time_out_and_retry() {
        let short = Duration::from_millis(25);
        let optimizer = Arc::new(BlockingOptimizer(AtomicUsize::new(0)));
        let result = optimize_with_timeout(optimizer.clone(), "original".into(), Lang::En, short);
        assert!(
            matches!(result, Err(OptimizerState::Failed { original, timeout: true, .. }) if original == "original")
        );
        assert!(optimize_with_timeout(optimizer, "original".into(), Lang::En, short).is_ok());
        let gh = Arc::new(BlockingGh(AtomicUsize::new(0)));
        let issue = github::parse_reference("o/r#1").unwrap();
        assert_eq!(
            import_with_timeout(gh.clone(), issue.clone(), short),
            Err(ImportError::TimedOut)
        );
        assert_eq!(
            import_with_timeout(gh, issue, short).unwrap().title,
            "title"
        );
    }
    #[test]
    fn optimizer_non_timeout_error_is_preserved() {
        let runner = Arc::new(crate::desktop::ux::seams::FakeOptimizer::new(vec![Err(
            Failure::new("failed", "broken"),
        )]));
        assert!(
            matches!(optimize_with_timeout(runner, "text".into(), Lang::En, Duration::from_secs(1)), Err(OptimizerState::Failed { original, timeout: false, message }) if original == "text" && message == "broken")
        );
    }
}
