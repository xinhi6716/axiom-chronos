use super::super::model::Usage;
use gpui_kit::SharedString;
#[derive(Clone, Debug, PartialEq)]
pub struct UsageSession {
    pub wish: SharedString,
    pub session: SharedString,
    pub label: SharedString,
    pub usage: Usage,
    pub complete: bool,
    pub started_at: Option<u64>,
}
