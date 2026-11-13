//! Compatibility entry points for the shared plain-language engine.
pub use crate::desktop::model::facade::work::{install, enabled, plain, wants, Plain, Source, DemoRunner, Runner};
pub fn set_enabled(on: bool, cx: &mut gpui_kit::App) {
    super::settings::Store::edit(cx, |s| s.plain_words = on);
    crate::desktop::model::facade::work::set_enabled(on, cx);
}
