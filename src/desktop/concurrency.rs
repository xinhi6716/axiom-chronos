//! Current reported model counts; no inference from cost, unit count or configured defaults.
use super::{i18n::{self, Lang, Table}, model::{ModelProvider, RunningModel}, ornament::Celestial};
use std::collections::BTreeMap;

const T: Table = &[
    ("{} · {} · {} running", "{} · {} · {} 個執行中"),
    ("Unknown", "未知模型"),
];

pub fn family(provider: ModelProvider, model: &str) -> (Celestial, &'static str) {
    let lower = model.to_ascii_lowercase();
    let words: Vec<_> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let candidates: &[(Celestial, &str)] = match provider {
        ModelProvider::Codex => &[(Celestial::Sun, "sol"), (Celestial::Moon, "luna"), (Celestial::Earth, "terra"), (Celestial::Star, "astra")],
        ModelProvider::Claude => &[(Celestial::Jupiter, "fable"), (Celestial::Saturn, "opus"), (Celestial::Venus, "sonnet"), (Celestial::Mercury, "haiku")],
        ModelProvider::Gemini => &[(Celestial::Gemini, "gemini"), (Celestial::Gemini, "argon")],
    };
    candidates.iter().find(|(_, name)| words.contains(name)).copied().unwrap_or((Celestial::Dot, "Unknown"))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Group {
    pub provider: ModelProvider,
    pub glyph: Celestial,
    pub count: u64,
    pub tooltip: String,
}

pub fn groups<'a>(models: impl Iterator<Item = &'a RunningModel>, lang: Lang) -> Vec<Group> {
    let mut counts = BTreeMap::<(ModelProvider, Celestial), BTreeMap<&str, u64>>::new();
    for model in models.filter(|m| m.count > 0 && !m.model.is_empty()) {
        *counts.entry((model.provider, family(model.provider, &model.model).0)).or_default()
            .entry(&model.model).or_default() += model.count;
    }
    counts.into_iter().map(|((provider, glyph), models)| {
        let name = match provider { ModelProvider::Codex => "Codex", ModelProvider::Claude => "Claude", ModelProvider::Gemini => "Gemini" };
        let count = models.values().sum();
        let tooltip = models.into_iter().map(|(model, count)| {
            let family = i18n::translate(lang, T, family(provider, model).1);
            let full = format!("{model} ({family})");
            i18n::fill(i18n::translate(lang, T, "{} · {} · {} running"), &[&full, name, &count.to_string()])
        }).collect::<Vec<_>>().join("\n");
        Group { provider, glyph, count, tooltip }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn family_matching_is_case_insensitive_and_respects_provider_and_word_boundaries() {
        use Celestial::*;
        for (provider, model, expected) in [
            (ModelProvider::Codex, "gpt-6.1-terra", Earth), (ModelProvider::Codex, "gpt-6-astra", Star),
            (ModelProvider::Codex, "GPT-6.1-SOL", Sun), (ModelProvider::Codex, "gpt-6-luna", Moon),
            (ModelProvider::Claude, "claude-sonnet-5-5", Venus), (ModelProvider::Claude, "claude-fable-5-2", Jupiter),
            (ModelProvider::Claude, "claude-opus-5", Saturn), (ModelProvider::Claude, "claude-haiku-5", Mercury),
            (ModelProvider::Gemini, "gemini-4-argon-high", Gemini), (ModelProvider::Gemini, "Argon", Gemini),
            (ModelProvider::Codex, "solar", Dot), (ModelProvider::Claude, "gpt-6-sol", Dot),
        ] { assert_eq!(family(provider, model).0, expected, "{model}"); }
    }
    #[test]
    fn groups_hide_zero_sort_by_provider_and_glyph_and_keep_full_unknown_ids() {
        let models = [(ModelProvider::Gemini,"gemini-4",1), (ModelProvider::Claude,"claude-haiku-5",2),
            (ModelProvider::Codex,"gpt-6.1-terra",12), (ModelProvider::Codex,"gpt-6-sol",2),
            (ModelProvider::Codex,"unfamiliar-model",3), (ModelProvider::Codex,"gpt-6-luna",0),
            (ModelProvider::Claude,"claude-sonnet-5-5",4), (ModelProvider::Codex,"gpt-6.2-terra",1)]
            .map(|(provider, model, count)| RunningModel { provider, model:model.into(), count });
        let rows = groups(models.iter(), Lang::En);
        assert_eq!(rows.iter().map(|g| (g.glyph,g.count)).collect::<Vec<_>>(), vec![
            (Celestial::Sun,2),(Celestial::Earth,13),(Celestial::Dot,3),(Celestial::Venus,4),(Celestial::Mercury,2),(Celestial::Gemini,1)]);
        assert!(rows[2].tooltip.contains("unfamiliar-model (Unknown) · Codex · 3 running"));
        assert_eq!(rows[1].tooltip.lines().count(),2);
        assert!(groups(models[..0].iter(),Lang::En).is_empty());
        assert!(groups([RunningModel { provider:ModelProvider::Codex, model:"sol".into(),count:0 }].iter(),Lang::En).is_empty());
        let zh=groups(models.iter(),Lang::ZhTw);
        assert!(zh[2].tooltip.contains("未知模型"));
        assert!(zh[0].tooltip.ends_with("2 個執行中"));
        for (en,zh) in T { assert!(!zh.is_empty()); assert_eq!(en.matches("{}").count(),zh.matches("{}").count()); }
    }
}
