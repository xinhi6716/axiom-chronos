//! Interface text. English is the key; each screen keeps a table of its own translations.
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    ZhTw,
    En,
}

impl Global for Lang {}

pub type Table = &'static [(&'static str, &'static str)];

/// Words shared by several screens.
pub const COMMON: Table = &[
    ("Run", "執行"),
    ("Wishes", "願望"),
    ("Memory", "記憶"),
    ("Usage", "用量"),
    ("Settings", "設定"),
    ("New wish", "許願"),
    ("Running", "進行中"),
    ("Waiting on batch", "等待批次"),
    ("Needs you", "需要你"),
    ("Done", "完成"),
    ("Pending", "待處理"),
    ("Awaiting review", "待審查"),
    ("Verified", "已驗收"),
    ("Failed", "失敗"),
    ("Stop", "停止"),
    ("Resume", "接續"),
    ("Cancel", "取消"),
    ("Copy", "複製"),
    ("Open folder", "開啟資料夾"),
    ("Open in editor", "用編輯器開啟"),
    ("Astra", "Astra"),
    ("Critic", "第二審"),
    ("Architect", "切模組"),
    ("Build", "施工"),
    ("Full test", "完整測試"),
    ("Report", "報告"),
];

/// Translates `en`, looking in the screen's table and then the shared one.
pub fn tr(cx: &App, table: Table, en: &'static str) -> &'static str {
    if *cx.global::<Lang>() == Lang::En {
        return en;
    }
    table
        .iter()
        .chain(COMMON.iter())
        .find(|(key, _)| *key == en)
        .map_or(en, |(_, zh)| zh)
}

/// The system's language: English only when the system says so, Traditional Chinese otherwise.
pub fn system_lang() -> Lang {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    if locale.to_ascii_lowercase().starts_with("en") {
        Lang::En
    } else {
        Lang::ZhTw
    }
}
