//! The window frame: title bar, navigation rail and the active screen.
use super::model::*;
use super::theme::{self, Tokened as _};
use super::i18n::tr;
use super::{composer::Composer, doctor::DoctorView, memory::MemoryView, run_view::RunView, usage::UsageView, wishes::WishesView};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, ThemeMode, TitleBar};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub struct Shell {
    state: Entity<AppState>,
    run: Entity<RunView>,
    wishes: Entity<WishesView>,
    memory: Entity<MemoryView>,
    usage: Entity<UsageView>,
    doctor: Entity<DoctorView>,
    composer: Entity<Composer>,
}

impl Shell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            run: cx.new(|cx| RunView::new(state.clone(), window, cx)),
            wishes: cx.new(|cx| WishesView::new(state.clone(), window, cx)),
            memory: cx.new(|cx| MemoryView::new(state.clone(), window, cx)),
            usage: cx.new(|cx| UsageView::new(state.clone(), window, cx)),
            doctor: cx.new(|cx| DoctorView::new(state.clone(), window, cx)),
            composer: cx.new(|cx| Composer::new(state.clone(), window, cx)),
            state,
        }
    }

    fn nav_item(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        view: Screen,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.tokens();
        let active = self.state.read(cx).view == view;
        let state = self.state.clone();
        v_flex()
            .id(id)
            .w(px(64.))
            .py_2()
            .gap_1()
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(if active { t.gold } else { t.muted })
            .when(active, |this| this.bg(t.raised))
            .hover(|this| this.bg(t.raised))
            .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_view(view, cx)))
            .child(gpui_kit::component::Icon::new(icon).size(px(18.)))
            .child(tr(cx, &[], label))
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let state = self.state.read(cx);
        let view = state.view;
        let composing = state.composing;
        let status = state.run().map(|r| (r.project.clone(), r.status));
        let content: AnyElement = match view {
            Screen::Run => self.run.clone().into_any_element(),
            Screen::Wishes => self.wishes.clone().into_any_element(),
            Screen::Memory => self.memory.clone().into_any_element(),
            Screen::Usage => self.usage.clone().into_any_element(),
            Screen::Doctor => self.doctor.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .relative()
            .bg(tokens.canvas)
            .text_color(tokens.text)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_sm().text_color(tokens.gold).child("Axiom Chronos"))
                                .when_some(status, |this, (project, status)| {
                                    this.child(div().text_sm().text_color(tokens.muted).child(project))
                                        .child(div().text_xs().text_color(tokens.status(status)).child(tr(cx, &[], status.label())))
                                }),
                        )
                        .child(
                            Button::new("toggle-theme")
                                .ghost()
                                .xsmall()
                                .icon(if cx.theme().is_dark() { IconName::Sun } else { IconName::Moon })
                                .on_click(cx.listener(|_, _, window, cx| {
                                    let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
                                    theme::apply(mode, Some(window), cx);
                                    window.refresh();
                                })),
                        ),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .h_full()
                            .w(px(80.))
                            .p_2()
                            .gap_1()
                            .items_center()
                            .bg(tokens.surface)
                            .border_r_1()
                            .border_color(tokens.border)
                            .child(self.nav_item("nav-run", IconName::LayoutDashboard, "Run", Screen::Run, cx))
                            .child(self.nav_item("nav-wishes", IconName::Inbox, "Wishes", Screen::Wishes, cx))
                            .child(self.nav_item("nav-memory", IconName::BookOpen, "Memory", Screen::Memory, cx))
                            .child(self.nav_item("nav-usage", IconName::ChartPie, "Usage", Screen::Usage, cx))
                            .child(self.nav_item("nav-doctor", IconName::Settings, "Settings", Screen::Doctor, cx)),
                    )
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
            .when(composing, |this| this.child(self.composer.clone()))
    }
}
