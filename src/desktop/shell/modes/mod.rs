mod chat;
mod plaza;
mod quick;
pub use chat::ChatPage;
pub use plaza::PlazaPage;
pub use quick::QuickTaskPage;

use crate::desktop::{
    i18n::{translate, Lang, Table},
    model::AppState,
    ux::seams::{ModeEngine, Speaker},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{button::Button, v_flex, Sizable as _},
    *,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
const T: Table = &[
    ("Back", "返回"),
    ("Mode service unavailable", "模式服務無法使用"),
    ("Send", "送出"),
    ("Make it a quick task", "轉為快速任務"),
    ("Make it a wish", "轉為願望"),
    ("Start", "開始"),
    ("Review", "審查"),
    ("Queued", "佇列中"),
    ("Building", "建置中"),
    ("Verifying", "驗證中"),
    ("Reviewing", "審查中"),
    ("Done", "完成"),
    ("Failed", "失敗"),
    ("Round", "輪流發言"),
    ("Debate", "辯論"),
    ("Vote", "表決"),
    ("Pin", "釘選"),
    ("You", "你"),
    ("Chat", "對話"),
    ("Quick task", "快速任務"),
    ("Plaza", "廣場討論"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
fn service(state: &Entity<AppState>, cx: &App) -> Option<Arc<dyn ModeEngine>> {
    state.read(cx).ux.seams.modes.clone()
}
fn back(state: Entity<AppState>, lang: Lang) -> Button {
    Button::new("page-back")
        .small()
        .child(tr(lang, "Back").test_text(tr(lang, "Back")))
        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx)))
}
fn marked(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
fn unavailable(state: Entity<AppState>, lang: Lang, id: &'static str) -> AnyElement {
    v_flex()
        .id(id)
        .test_support()
        .size_full()
        .p(px(20.))
        .child(back(state, lang))
        .child(tr(lang, "Mode service unavailable").test_text(tr(lang, "Mode service unavailable")))
        .into_any_element()
}
fn name(s: Speaker) -> &'static str {
    match s {
        Speaker::You => "You",
        Speaker::Core => "Core",
        Speaker::Astra => "Astra",
        Speaker::Fable => "Fable",
        Speaker::Argon => "Argon",
    }
}
fn display_name(s: Speaker, lang: Lang) -> &'static str {
    if s == Speaker::You {
        tr(lang, "You")
    } else {
        name(s)
    }
}
fn glyph(s: Speaker) -> &'static str {
    match s {
        Speaker::You => "•",
        Speaker::Core => "⌛",
        Speaker::Astra => "✦",
        Speaker::Fable => "♃",
        Speaker::Argon => "♊",
    }
}
pub(super) fn seat_label(s: Speaker) -> String {
    format!("{} {}", glyph(s), name(s))
}
fn unique(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{}-{}", prefix, NEXT.fetch_add(1, Ordering::Relaxed))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn mode_copy_is_bilingual_and_unique() {
        let mut keys = std::collections::HashSet::new();
        for (en, zh) in T {
            assert!(keys.insert(*en));
            assert_ne!(en, zh);
            assert!(!zh.is_empty());
        }
        for key in [
            "Back",
            "Mode service unavailable",
            "Send",
            "Make it a quick task",
            "Make it a wish",
            "Start",
            "Review",
            "Queued",
            "Building",
            "Verifying",
            "Reviewing",
            "Done",
            "Failed",
            "Round",
            "Debate",
            "Vote",
            "Pin",
            "You",
            "Chat",
            "Quick task",
            "Plaza",
        ] {
            assert!(keys.contains(key), "{key}");
        }
    }
}
