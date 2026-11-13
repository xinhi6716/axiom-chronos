//! Token-coloured notifications with per-kind token durations.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{font, ButtonVariant, Tokens, TypeSize};
use crate::desktop::motion::Dur;
use crate::desktop::theme;
use gpui_kit::component::{notification::Notification as KitNotification, WindowExt as _};
use gpui_kit::{
    div, App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
};
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
}
impl NotificationKind {
    pub fn color(self, t: &Tokens) -> Hsla {
        match self {
            Self::Info => t.run,
            Self::Success => t.ok,
            Self::Warning => t.review,
            Self::Error => t.fail,
        }
    }
    pub fn duration(self) -> Dur {
        match self {
            Self::Info => Dur::Standard,
            Self::Success => Dur::Gentle,
            Self::Warning | Self::Error => Dur::Slow,
        }
    }
}
#[derive(IntoElement)]
pub struct Notification {
    id: ElementId,
    message: SharedString,
    kind: NotificationKind,
    tokens: Tokens,
}
impl Notification {
    fn resolved_style(&self, d: theme::Density) -> (Hsla, gpui_kit::Pixels) {
        (self.color(), font(d, TypeSize::Body))
    }

    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<SharedString>,
        kind: NotificationKind,
        tokens: &Tokens,
    ) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            kind,
            tokens: *tokens,
        }
    }
    pub fn color(&self) -> Hsla {
        self.kind.color(&self.tokens)
    }
    pub fn duration(&self) -> Dur {
        self.kind.duration()
    }
}
impl Notification {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let (color, size) = self.resolved_style(d);
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Secondary)
            .child(div().text_color(color).text_size(size).child(self.message))
    }
}
impl RenderOnce for Notification {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
fn notification_key(id: &ElementId, sequence: usize) -> ElementId {
    (id.clone(), sequence.to_string()).into()
}
fn dismiss_duration(kind: NotificationKind) -> std::time::Duration {
    kind.duration().get()
}
/// Show a notification and dismiss this particular instance after its token duration.
pub fn notify(window: &mut Window, cx: &mut App, n: Notification) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let key = notification_key(&n.id, NEXT.fetch_add(1, Ordering::Relaxed));
    let duration = dismiss_duration(n.kind);
    let id = key.clone();
    let message = n.message.clone();
    let kind = n.kind;
    let tokens = n.tokens;
    let note = KitNotification::new()
        .id1::<Notification>(key.clone())
        .autohide(false)
        .bg(tokens.surface)
        .border_color(kind.color(&tokens))
        .content(move |_, _, _| {
            Notification::new(id.clone(), message.clone(), kind, &tokens).into_any_element()
        });
    // needs=smoke: live push, timer, and dismissal require a GPUI window.
    window.push_notification(note, cx);
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(duration).await;
        let _ = handle.update(cx, |_, window, cx| {
            window.remove_notification1::<Notification>(key, cx)
        });
    })
    .detach();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_keys_are_distinct() {
        for _ in theme::both() {
            let id = ElementId::from("notice");
            assert_ne!(notification_key(&id, 0), notification_key(&id, 1));
            assert_ne!(
                notification_key(&id, 0),
                notification_key(&ElementId::from("other"), 0)
            );
        }
    }
    #[test]
    fn dismissal_uses_each_kind_duration() {
        for _ in theme::both() {
            for kind in [
                NotificationKind::Info,
                NotificationKind::Success,
                NotificationKind::Warning,
                NotificationKind::Error,
            ] {
                assert_eq!(dismiss_duration(kind), kind.duration().get());
            }
        }
    }
    #[test]
    fn resolved_notification_style_and_dismissal() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for (kind, color, duration) in [
                    (NotificationKind::Info, t.run, Dur::Standard),
                    (NotificationKind::Success, t.ok, Dur::Gentle),
                    (NotificationKind::Warning, t.review, Dur::Slow),
                    (NotificationKind::Error, t.fail, Dur::Slow),
                ] {
                    let n = Notification::new("notice", "Message", kind, &t);
                    assert_eq!(n.resolved_style(d), (color, font(d, TypeSize::Body)));
                    assert_eq!(n.duration(), duration);
                    assert_eq!(dismiss_duration(kind), kind.duration().get());
                    assert_eq!(n.into_toolbar(d).id, ElementId::from("notice"));
                }
            }
            let id = ElementId::from("notice");
            assert_ne!(notification_key(&id, 0), notification_key(&id, 1));
            assert_ne!(
                notification_key(&id, 0),
                notification_key(&ElementId::from("other"), 0)
            );
        }
    }
}
