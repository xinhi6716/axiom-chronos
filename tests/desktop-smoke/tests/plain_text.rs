use axiom::desktop::{
    i18n::{self, Lang},
    plain_text::{self, Runner, Source},
    Store,
};
use gpui_kit::{private::serde_json, TestAppContext};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Fake {
    prompts: Mutex<Vec<String>>,
}

impl Runner for Fake {
    fn run(&self, prompt: &str) -> Result<String, String> {
        self.prompts.lock().unwrap().push(prompt.to_owned());
        let input: Vec<String> = serde_json::from_str(prompt).map_err(|e| e.to_string())?;
        let output: Vec<_> = input.iter().map(|text| format!("翻譯：{text}")).collect();
        serde_json::to_string(&output).map_err(|e| e.to_string())
    }
}

fn install(cx: &mut TestAppContext, lang: Lang) -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Store::open(true));
        i18n::set(lang, cx);
        plain_text::install(cx, true, Source::Fake(fake.clone()));
    });
    fake
}

#[gpui_kit::test]
fn traditional_chinese_batches_and_shows_translation(cx: &mut TestAppContext) {
    let fake = install(cx, Lang::ZhTw);
    cx.update(|cx| {
        plain_text::plain(cx, &"Fix the build".into());
    });
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.update(|cx| {
        plain_text::plain(cx, &"Review the changes".into());
    });

    // The worker measures the debounce with std::time::Instant, not the test clock.
    std::thread::sleep(Duration::from_millis(410));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();

    let prompts = fake.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("Fix the build"));
    assert!(prompts[0].contains("Review the changes"));
    drop(prompts);
    let result = cx.update(|cx| plain_text::plain(cx, &"Fix the build".into()));
    assert_eq!(result.original.as_deref(), Some("Fix the build"));
    assert_ne!(result.shown.as_ref(), "Fix the build");
}

#[gpui_kit::test]
fn english_does_not_run(cx: &mut TestAppContext) {
    let fake = install(cx, Lang::En);
    cx.update(|cx| {
        for text in ["Fix the build", "Review the changes"] {
            let result = plain_text::plain(cx, &text.into());
            assert_eq!(result.shown.as_ref(), text);
            assert!(result.original.is_none());
        }
    });
    std::thread::sleep(Duration::from_millis(410));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(fake.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn disabled_does_not_run(cx: &mut TestAppContext) {
    let fake = install(cx, Lang::ZhTw);
    cx.update(|cx| {
        plain_text::set_enabled(false, cx);
        assert!(!plain_text::enabled(cx));
        plain_text::plain(cx, &"Fix the build".into());
        plain_text::plain(cx, &"Review the changes".into());
    });
    std::thread::sleep(Duration::from_millis(410));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(fake.prompts.lock().unwrap().is_empty());
}
