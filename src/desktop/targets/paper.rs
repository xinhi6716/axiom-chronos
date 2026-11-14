use super::*;
pub(super) fn targets() -> Vec<Target> {
    let mut targets = vec![
        entry("paper", "paper", Opening::Page("paper")),
        entry("paper.form", "paper", Opening::PaperStep("form")),
        entry("paper.shortlist", "paper", Opening::PaperStep("shortlist")),
        entry("paper.estimate", "paper", Opening::PaperStep("estimate")),
        entry("paper.progress", "paper", Opening::PaperStep("progress")),
        entry("paper.changelog", "paper", Opening::PaperStep("changelog")),
        entry("paper.preview", "paper", Opening::PaperStep("preview")),
        entry("paper.readiness", "paper", Opening::PaperStep("readiness")),
    ];
    for target in &mut targets {
        target.states = &[CaptureState::Live];
    }
    targets
}
pub(super) fn prepare(
    t: &Target,
    s: CaptureState,
    state: &mut AppState,
) -> Result<(), Unregistered> {
    match s {
        CaptureState::Live => {}
        CaptureState::Empty | CaptureState::Error | CaptureState::Action => {
            return Err(super::unavailable(t, s));
        }
    }
    match t.opening {
        Opening::Page("paper") => super::prepare_basic(t, s, state),
        Opening::PaperStep(
            "form" | "shortlist" | "estimate" | "progress" | "changelog" | "preview" | "readiness",
        ) => super::prepare_basic(&entry("paper", "paper", Opening::Page("paper")), s, state),
        _ => Err(super::unavailable(t, s)),
    }
}

pub(crate) fn open(
    t: &Target,
    s: CaptureState,
    state: &gpui_kit::Entity<AppState>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) -> Result<(), Unregistered> {
    match s {
        CaptureState::Live => {}
        CaptureState::Empty | CaptureState::Error | CaptureState::Action => {
            return Err(super::unavailable(t, s));
        }
    }
    match t.opening {
        Opening::Page("paper")
        | Opening::PaperStep(
            "form" | "shortlist" | "estimate" | "progress" | "changelog" | "preview" | "readiness",
        ) => {}
        _ => return Err(super::unavailable(t, s)),
    }
    let _ = window;
    state.update(cx, |state, cx| {
        if !matches!(state.ux.page, Some(Page::Paper(_))) {
            let session = state
                .runs
                .first()
                .map(|run| run.session.clone())
                .unwrap_or_default();
            state.ux.page = Some(Page::Paper(session));
        }
        cx.notify();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn every_paper_target_prepares_each_declared_state() {
        for target in targets() {
            for &capture in target.states {
                let mut state = crate::desktop::demo::state();
                state.ux.seams.paper = Some(std::sync::Arc::new(
                    crate::desktop::ux::seams::FakePaper::new(),
                ));
                assert_eq!(
                    prepare(&target, capture, &mut state),
                    Ok(()),
                    "{}",
                    target.id
                );
                assert!(
                    matches!(state.ux.page, Some(Page::Paper(_))),
                    "{}",
                    target.id
                );
            }
            let mut state = crate::desktop::demo::state();
            assert_eq!(
                prepare(&target, CaptureState::Action, &mut state),
                Err(Unregistered {
                    target: target.id.into(),
                    state: CaptureState::Action
                })
            );
        }
    }
}
