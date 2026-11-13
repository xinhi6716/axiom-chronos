use super::{
    modes::{ChatPage, PlazaPage, QuickTaskPage},
    Shell,
};
use crate::desktop::{detail::SessionPage, doctor::storage::StoragePage, ux::Page};
use gpui_kit::*;

pub(super) fn page_escape_binding() -> KeyBinding {
    KeyBinding::new("escape", gpui_kit::component::input::Escape, Some("ChronosShell"))
}

pub(super) fn can_close_page(page_open: bool, overlay_open: bool, dialog_open: bool) -> bool {
    page_open && !overlay_open && !dialog_open
}

impl Shell {
    pub(super) fn page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let page = self.state.read(cx).ux.page.clone();
        if self.page_view.as_ref().map(|p| &p.0) != page.as_ref() {
            // The replaced screen's focus may have vanished; leave overlays and modals alone.
            if page.is_some() && self.palette.read(cx).overlay() == super::Overlay::Closed && !self.dialog_open(window, cx) {
                self.focus.focus(window, cx);
            }
            self.page_view = page.map(|page| {
                let view: AnyView = match &page {
                    // --- research workspace (phase 9) ---
                    Page::Research => cx.new(|cx| crate::desktop::research::ResearchPage::new(self.state.clone(), window, cx)).into(),
                    // --- end research workspace (phase 9) ---
                    Page::Routine => cx.new(|cx| super::modes::RoutinePage::new(self.state.clone(), window, cx)).into(),
                    Page::Storage => cx.new(|cx| StoragePage::new(self.state.clone(), cx)).into(),
                    Page::Chat => cx
                        .new(|cx| ChatPage::new(self.state.clone(), window, cx))
                        .into(),
                    Page::QuickTask => cx
                        .new(|cx| QuickTaskPage::new(self.state.clone(), window, cx))
                        .into(),
                    Page::Plaza => cx
                        .new(|cx| PlazaPage::new(self.state.clone(), window, cx))
                        .into(),
                    Page::Session(id) => cx
                        .new(|cx| SessionPage::new(self.state.clone(), id.clone(), window, cx))
                        .into(),
                    // p8: begin stub
                    Page::Paper(id) => cx
                        .new(|cx| crate::desktop::paper::PaperPage::new(self.state.clone(), id.clone(), window, cx))
                        .into(),
                    // p8: end stub
                };
                (page, view)
            });
        }
        self.page_view
            .as_ref()
            .map(|(_, v)| v.clone().into_any_element())
    }

    pub(super) fn escape_page(&mut self, _: &gpui_kit::component::input::Escape, window: &mut Window, cx: &mut Context<Self>) {
        if can_close_page(self.state.read(cx).ux.page.is_some(), self.palette.read(cx).overlay() != super::Overlay::Closed, self.dialog_open(window, cx)) {
            self.state.update(cx, |s, cx| s.close_page(cx));
        } else {
            cx.propagate();
        }
    }

}

impl Shell {
 pub(crate) fn apply_target(&mut self, t: &crate::desktop::targets::Target, s: crate::desktop::targets::CaptureState, window: &mut Window, cx: &mut Context<Self>) -> Result<(), crate::desktop::targets::Unregistered> {
  use crate::desktop::targets::{CaptureState, Opening, unavailable};
  if s != CaptureState::Live && !(matches!(t.opening, Opening::Screen(crate::desktop::model::Screen::Run | crate::desktop::model::Screen::Wishes | crate::desktop::model::Screen::Memory | crate::desktop::model::Screen::Usage)) && matches!(s, CaptureState::Empty | CaptureState::Error)) { return Err(unavailable(t,s)); }
  match t.opening {
   Opening::Screen(_) | Opening::Page(_) => Ok(()),
   Opening::Window("mini") => {
    // The shell's normal Mini transition always opens on-screen.
    if !self.may_flash { return Err(unavailable(t,s)); }
    crate::desktop::settings::set_mode(crate::desktop::settings::WindowMode::Mini, cx);
    self.apply_wanted(window, cx);
    if self.mini.is_some() { Ok(()) } else { Err(unavailable(t,s)) }
   },
   Opening::Window("focus.graph" | "focus.checklist") => {
    let panel = if t.id == "focus.graph" { crate::desktop::settings::FocusPanel::Graph } else { crate::desktop::settings::FocusPanel::Checklist };
    crate::desktop::settings::set_mode(crate::desktop::settings::WindowMode::Focus(panel), cx);
    self.apply_wanted(window, cx);
    Ok(())
   },
   Opening::Overlay("composer") | Opening::Dialog("composer") => { self.state.update(cx, |s, cx| s.set_composing(true, cx)); Ok(()) },
   Opening::Overlay("palette") => { self.toggle_palette(window,cx); Ok(()) },
   Opening::Dialog("close") => { self.show_close_dialog(false,window,cx); Ok(()) },
   _ => Err(unavailable(t,s)),
  }
 }
}
