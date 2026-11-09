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
    translate(*cx.global::<Lang>(), table, en)
}

fn translate(lang: Lang, table: Table, en: &'static str) -> &'static str {
    if lang == Lang::En {
        return en;
    }
    table
        .iter()
        .chain(COMMON.iter())
        .find(|(key, _)| *key == en)
        .map_or(en, |(_, zh)| zh)
}

/// Puts `values` into the `{}` slots of a translated template, in order.
pub fn fill(template: &str, values: &[&str]) -> String {
    let mut text = String::with_capacity(template.len());
    let mut values = values.iter();
    let mut rest = template;
    while let Some(slot) = rest.find("{}") {
        text.push_str(&rest[..slot]);
        text.push_str(values.next().copied().unwrap_or_default());
        rest = &rest[slot + 2..];
    }
    text.push_str(rest);
    text
}

/// The system's language: English only when the system says so, Traditional Chinese otherwise.
pub fn system_lang() -> Lang {
    lang_for_locale(&system_locale())
}

fn lang_for_locale(locale: &str) -> Lang {
    let language = locale.split(['-', '_', '.', '@']).next().unwrap_or_default();
    if language.eq_ignore_ascii_case("en") {
        Lang::En
    } else {
        Lang::ZhTw
    }
}

/// POSIX precedence: the first of these that is set and not empty decides.
fn environment_locale(read: impl Fn(&str) -> Option<String>) -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| read(name).filter(|value| !value.is_empty()))
        .unwrap_or_default()
}

fn process_locale() -> String {
    environment_locale(|name| std::env::var(name).ok())
}

#[cfg(windows)]
fn system_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    let mut name = [0u16; LOCALE_NAME_MAX_LENGTH];
    // The count includes the terminating null, and zero reports failure.
    let written = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if written > 1 {
        String::from_utf16_lossy(&name[..written as usize - 1])
    } else {
        process_locale()
    }
}

#[cfg(not(windows))]
fn system_locale() -> String {
    process_locale()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_locales_are_recognized_in_every_spelling() {
        for locale in ["en", "EN", "en-US", "en_GB.UTF-8", "en_US@euro", "En-au"] {
            assert_eq!(lang_for_locale(locale), Lang::En, "{locale}");
        }
    }

    #[test]
    fn anything_else_is_traditional_chinese() {
        for locale in ["zh-TW", "zh_TW.UTF-8", "zh-Hant-HK", "ja-JP", "", "C", "POSIX", "enm"] {
            assert_eq!(lang_for_locale(locale), Lang::ZhTw, "{locale}");
        }
    }

    #[test]
    fn locale_variables_follow_posix_precedence_and_skip_empty_ones() {
        let read = |set: &'static [(&'static str, &'static str)]| {
            move |name: &str| set.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string())
        };
        assert_eq!(environment_locale(read(&[("LC_ALL", "en_US"), ("LANG", "zh_TW")])), "en_US");
        assert_eq!(environment_locale(read(&[("LC_ALL", ""), ("LC_MESSAGES", "ja_JP"), ("LANG", "zh_TW")])), "ja_JP");
        assert_eq!(environment_locale(read(&[("LC_ALL", ""), ("LANG", "en_GB")])), "en_GB");
        assert_eq!(environment_locale(read(&[])), "");
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_user_locale_is_a_language_tag() {
        let locale = system_locale();
        assert!(locale.len() >= 2, "{locale:?}");
        assert!(locale.is_ascii(), "{locale:?}");
    }

    #[test]
    fn slots_fill_in_order_and_extra_slots_stay_empty() {
        assert_eq!(fill("{} of {} done", &["3", "9"]), "3 of 9 done");
        assert_eq!(fill("已驗收 {} / {} 個單元", &["3", "9"]), "已驗收 3 / 9 個單元");
        assert_eq!(fill("no slots", &["x"]), "no slots");
        assert_eq!(fill("{} and {}", &["one"]), "one and ");
    }

    #[test]
    fn translation_prefers_the_screen_table_then_the_shared_one() {
        let table: Table = &[("Run", "跑"), ("Only here", "只在這裡")];
        assert_eq!(translate(Lang::ZhTw, table, "Run"), "跑");
        assert_eq!(translate(Lang::ZhTw, table, "Wishes"), "願望");
        assert_eq!(translate(Lang::ZhTw, table, "Unknown"), "Unknown");
        assert_eq!(translate(Lang::En, table, "Only here"), "Only here");
    }
}
