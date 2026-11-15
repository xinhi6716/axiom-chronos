use super::*;
pub(super) fn targets() -> Vec<Target> {
    let mut targets = vec![
        entry("research", "research", Opening::Page("research")),
        entry("research.map", "research", Opening::ResearchTab("map")),
        entry(
            "research.sources",
            "research",
            Opening::ResearchTab("sources"),
        ),
        entry(
            "research.evidence",
            "research",
            Opening::ResearchTab("evidence"),
        ),
        entry(
            "research.dataset",
            "research",
            Opening::ResearchTab("dataset"),
        ),
        entry(
            "research.exports",
            "research",
            Opening::ResearchTab("exports"),
        ),
    ];
    for target in &mut targets {
        target.states = &[CaptureState::Live];
        // Each ID is attached to a real element in the research page.
        let ids: (&'static [&'static str], &'static [&'static str]) = match target.id {
            "research" => (&["research.page"], &["action:research.back", "action:research.open"]),
            "research.map" => (&["research.map.panel"], &["action:research.map.open"]),
            "research.sources" => (&["research.sources.panel"], &["action:research.sources.open"]),
            "research.evidence" => (&["research.evidence.panel"], &["action:research.evidence.open"]),
            "research.dataset" => (&["research.dataset.panel"], &["action:research.dataset.open"]),
            "research.exports" => (&["research.exports.panel"], &["action:research.exports.open"]),
            _ => unreachable!(),
        };
        (target.panels, target.actions) = ids;
    }
    targets
}
pub(super) fn prepare(
    t: &Target,
    s: CaptureState,
    state: &mut AppState,
) -> Result<(), Unregistered> {
    if s != CaptureState::Live {
        return Err(super::unavailable(t, s));
    }
    match t.opening {
        Opening::Page("research") => {
            state.ux.page = Some(Page::Research);
            Ok(())
        }
        Opening::ResearchTab("map" | "sources" | "evidence" | "dataset" | "exports") => {
            state.ux.page = Some(Page::Research);
            Ok(())
        },
        _ => Err(super::unavailable(t, s)),
    }
}

pub(crate) fn open(
    t: &Target,
    s: CaptureState,
    state: &gpui_kit::Entity<AppState>,
    _window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) -> Result<(), Unregistered> {
    if s != CaptureState::Live {
        return Err(super::unavailable(t, s));
    }
    let tab = match t.opening {
        Opening::Page("research") => None,
        Opening::ResearchTab(key) => Some(
            crate::desktop::research::Tab::from_key(key).ok_or_else(|| super::unavailable(t, s))?,
        ),
        _ => return Err(super::unavailable(t, s)),
    };
    let already_open = matches!(state.read(cx).ux.page, Some(Page::Research));
    crate::desktop::research::adapter::open_target(tab, already_open, cx);
    state.update(cx, |state, cx| state.open_page(Page::Research, cx));
    Ok(())
}
