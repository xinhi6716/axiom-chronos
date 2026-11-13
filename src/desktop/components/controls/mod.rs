mod button;
pub use button::Button;

mod icon_button;
pub use icon_button::IconButton;

mod badge;
pub use badge::Badge;

mod chip;
pub use chip::Chip;

mod tooltip;
pub use tooltip::Tooltip;

mod list_row;
pub use list_row::ListRow;

mod toolbar;
pub use toolbar::Toolbar;

mod toggle;
pub use toggle::Toggle;

mod text_field;
pub use text_field::TextField;

mod tabs;
pub use tabs::Tabs;

mod segmented;
pub use segmented::Segmented;

mod card;
pub use card::Card;

mod dialog_frame;
pub use dialog_frame::DialogFrame;

mod empty_state;
pub use empty_state::EmptyState;

mod progress;
pub use progress::Progress;

mod status_dot;
pub use status_dot::StatusDot;

mod split_handle;
pub use split_handle::SplitHandle;

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use gpui_kit::App;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{Arc, Mutex},
        time::Instant,
    };

    // GPUI test-support is not enabled. A headless App supplies a real FocusHandle
    // so the same Interaction and composed Button/Toolbar used by render can be driven.
    #[test]
    fn composed_controls_snap_and_show_focus_ring() {
        std::thread::spawn(|| {
            let result = Arc::new(Mutex::new(None));
            let outcome = result.clone();
            gpui_kit::platform::headless().run(move |cx: &mut App| {
                let checked = catch_unwind(AssertUnwindSafe(|| {
                    let now = Instant::now();
                    let mut interaction = button::Interaction::new(cx.focus_handle());
                    interaction.hover(true, now, true);
                    interaction.press(true, now, true);
                    for t in crate::desktop::theme::both() {
                        let field = text_field::FieldStyle::new("field", &t);
                        assert_style(field.style(&interaction, now, true, true), &t, true);
                        assert_style(field.style(&interaction, now, true, false), &t, false);
                        let buttons = [
                            Toggle::new("toggle-off", "Toggle", &t)
                                .checked(false)
                                .into_button(),
                            Toggle::new("toggle-on", "Toggle", &t)
                                .checked(true)
                                .into_button(),
                            Tabs::new("tabs", &t)
                                .item("tab", "Tab")
                                .selected(99)
                                .item_button(0)
                                .unwrap(),
                            Segmented::new("segmented", &t)
                                .item("segment", "Segment")
                                .selected(99)
                                .tabs
                                .item_button(0)
                                .unwrap(),
                            Chip::new("chip", "Chip", &t).into_button(),
                            Tooltip::new(Button::new("tip", "Tip", &t), "Help").into_trigger(),
                            ListRow::new("row", "Row", &t).into_button(),
                            StatusDot::new(
                                "status",
                                "Status",
                                crate::desktop::components::tokens::Status::Running,
                                &t,
                            )
                            .into_button(),
                        ];
                        for control in buttons {
                            assert!(control.tab_stop());
                            assert_style(control.style(&interaction, now, true, true), &t, true);
                            assert_style(control.style(&interaction, now, true, false), &t, false);
                        }
                        let toolbars = [
                            Toolbar::new("toolbar", &t),
                            Card::new("card", &t).into_toolbar(),
                            DialogFrame::new("dialog", &t).into_toolbar(),
                            EmptyState::new("empty", "Empty", &t).into_toolbar(),
                            Progress::bar("progress", 0.5, &t).toolbar,
                            SplitHandle::new("split", &t).toolbar,
                        ];
                        assert!(Button::new("button", "Button", &t).tab_stop());
                        for control in toolbars {
                            assert!(control.tab_stop());
                            assert_style(control.style(&interaction, now, true, true), &t, true);
                            assert_style(control.style(&interaction, now, true, false), &t, false);
                        }
                    }
                    interaction.hover(false, now, true);
                    assert_eq!(interaction.hover.value(now), 0.);
                    assert_eq!(interaction.press.value(now), 0.);
                }));
                *outcome.lock().unwrap() = Some(checked);
                cx.quit();
            });
            if let Err(panic) = result
                .lock()
                .unwrap()
                .take()
                .expect("app callback did not run")
            {
                std::panic::resume_unwind(panic);
            };
        })
        .join()
        .unwrap();
    }

    fn assert_style(
        style: button::InteractionStyle,
        tokens: &crate::desktop::components::tokens::Tokens,
        focused: bool,
    ) {
        assert!(!style.moving);
        assert_eq!((style.hover, style.press), (1., 1.));
        if focused {
            assert_eq!(style.ring, crate::desktop::theme::focus_ring(tokens));
            assert!(!style.ring.is_empty());
        } else {
            assert!(style.ring.is_empty());
        }
    }
}
