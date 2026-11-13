use super::*;
use std::prelude::v1::test;

#[test]
fn facade_exposes_work_helpers() {
    let run = crate::desktop::demo::sample_run();
    assert_eq!(glyph_state(run.status, run.verified_end()), crate::desktop::wishes::glyph_state(run.status, run.verified_end()));
    assert_eq!(one_line(&run.wish), crate::desktop::wishes::one_line(&run.wish));
    assert_eq!(attention_key(&run), crate::desktop::wishes::read_state::attention_key(&run));
    let _hook = crate::desktop::model::facade::settings::PlainWordsApply(apply_plain_words);
}

#[test]
fn offers_match_control_for_all_inputs() {
    use crate::desktop::control as old;
    for status in [RunStatus::Active, RunStatus::Waiting, RunStatus::LimitWait, RunStatus::Interrupted, RunStatus::Blocked, RunStatus::Done] {
        for hosted in [false, true] { for folder in [false, true] { for report in [false, true] {
            let a = offer(status, hosted, folder, report);
            let b = old::offer(status, hosted, folder, report);
            assert_eq!(format!("{:?}", a.primary), format!("{:?}", b.primary));
            assert_eq!(format!("{:?}", a.entry), format!("{:?}", b.entry));
            assert_eq!(format!("{:?}", a.report), format!("{:?}", b.report));
            assert_eq!((a.stop,a.open_folder,a.batch,a.view_only), (b.stop,b.open_folder,b.batch,b.view_only));
        } } }
    }
}

#[test]
fn wishes_helpers_match_originals() {
    use crate::desktop::wishes as old;
    let mut runs = Vec::new();
    for status in [RunStatus::Active, RunStatus::Waiting, RunStatus::LimitWait, RunStatus::Interrupted, RunStatus::Blocked, RunStatus::Done] {
        let mut r = sample_run(); r.status = status;
        for verified in [false, true] { assert_eq!(glyph_state(status, verified), old::glyph_state(status, verified)); }
        assert_eq!(can_resume(status), old::can_resume(status));
        for lang in [Lang::En, Lang::ZhTw] { assert_eq!(started_label(&r.started, lang), old::started_label(&r.started, lang)); }
        runs.push(r);
    }
    for text in ["first\n second\tpart", "normal", "  "] { let s = SharedString::from(text); assert_eq!(one_line(&s), old::one_line(&s)); }
    assert_eq!(one_line_limit("  a\n bcd", 4).as_ref(), "a b");
    for query in ["", "  REVIEW   Reader", "不存在"] {
        let terms = search_terms(query); assert_eq!(terms, old::search_terms(query));
        assert_eq!(matches_terms(&["Review", "reader"], &terms), old::matches_terms(&["Review", "reader"], &terms));
        assert_eq!(filter_counts(&runs, &terms), old::filter_counts(&runs, &terms));
    }
}

#[test]
fn read_state_matches_and_round_trips() {
    use crate::desktop::wishes::read_state as old;
    let dir = tempfile::tempdir().unwrap();
    let run = sample_run(); let mut prefs = Store::at(Some(dir.path().join("prefs.json"))).settings.ux;
    let pairs = [(ReadAction::MarkRead, old::ReadAction::MarkRead), (ReadAction::Archive, old::ReadAction::Archive), (ReadAction::Unarchive, old::ReadAction::Unarchive), (ReadAction::MarkUnread, old::ReadAction::MarkUnread)];
    let mut previous = prefs.clone();
    for (a, b) in pairs {
        assert_eq!(attention_key(&run), old::attention_key(&run));
        assert_eq!(is_unread(&run, &prefs), old::is_unread(&run, &previous));
        assert_eq!(is_archived(&run, &prefs), old::is_archived(&run, &previous));
        for f in [ReadFilter::All, ReadFilter::Unread, ReadFilter::Archived] { assert_eq!(visible(&run, &prefs, f), old::visible(&run, &previous, f)); }
        assert_eq!(format!("{:?}", read_actions(&run, &prefs)), format!("{:?}", old::read_actions(&run, &previous)));
        for lang in [Lang::En, Lang::ZhTw] { assert_eq!(action_label(a, lang), old::action_label(b, lang)); }
        change_mark(&run, &mut prefs, a); old::change_mark(&run, &mut previous, b);
        assert_eq!(prefs.read, previous.read);
    }
    assert!(is_unread(&run, &prefs)); assert!(!is_archived(&run, &prefs));
}

#[test]
fn subjects_clock_and_height_match() {
    let run = sample_run();
    for selection in [Selection::None, Selection::Module(0), Selection::Unit(0), Selection::Module(usize::MAX)] {
        let a = subject_of(&run, &selection); let b = crate::desktop::control::subject_of(&run, &selection);
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a,b) {
            assert_eq!((&a.goal,&a.module,&a.unit), (&b.goal,&b.module,&b.unit));
            for lang in [Lang::En, Lang::ZhTw] { assert_eq!(address_line(&a,lang), crate::desktop::control::address_line(&b,lang)); }
        }
    }
    assert_eq!(RUNNING_ENTRY_HEIGHT, crate::desktop::control::RUNNING_ENTRY_HEIGHT);
    assert_eq!(clock(-1), "23:59:59");
    let now = now_clock(); assert_eq!(now.len(), 8); assert_eq!(now.as_bytes()[2], b':'); assert_eq!(now.as_bytes()[5], b':');
}

#[test]
fn facade_ux_types_and_live_wrappers() {
    let mut p: crate::desktop::ux::activity::Projection = Projection::default();
    let _: crate::desktop::ux::events::EventSummary = EventSummary { tone: Tone::Info, en: String::new(), zh: String::new(), raw: None };
    let _: crate::desktop::ux::events::Notice = Notice { key: "x".into(), kind: NoticeKind::MaxTurns { turn_limit: Some(4) }, at_ms: None };
    p.last_seen_ms = Some(1000); p.open.push(OpenActivity { id:"1".into(), module:"m".into(), actor:"core".into(), phase:"review".into(), since_ms:1000 });
    let run = sample_run();
    for lang in [Lang::En, Lang::ZhTw] {
        assert_eq!(heartbeat_text(p.pulse(2000), 2000, lang), crate::desktop::live::activity::heartbeat_text(p.pulse(2000), 2000, lang));
        assert_eq!(activity_text(&p.open[0], lang), crate::desktop::live::activity::activity_text(&p.open[0], lang));
        let a=live_stage(&run,&p,2000,lang); let b=crate::desktop::live::activity::live_stage(&run,&p,2000,lang);
        assert_eq!(a.map(|s| (s.name,s.actor,s.since_ms)), b.map(|s| (s.name,s.actor,s.since_ms)));
        let n=Notice { key:"x".into(),kind:NoticeKind::MaxTurns { turn_limit:Some(4) },at_ms:None };
        let a=notice_copy(&n,lang); let b=crate::desktop::live::events::notice_copy(&n,lang);
        assert_eq!((a.title,a.cause,a.effect,a.action), (b.title,b.cause,b.effect,b.action));
        for text in ["Work progress has been updated.", "工作進度已更新。", "Continue with the next step.", "繼續下一步。", "A real summary."] { assert_eq!(board_sentence(text,lang), crate::desktop::demo::board_sentence(text,lang)); }
    }
    let a=workers(&run,&p,None); let b=crate::desktop::live::activity::workers(&run,&p,None);
    assert_eq!(a.len(),b.len()); for (a,b) in a.iter().zip(b.iter()) { assert_eq!(format!("{:?}",a.kind),format!("{:?}",b.kind)); assert_eq!((&a.who,&a.what,a.since_ms,&a.unit),(&b.who,&b.what,b.since_ms,&b.unit)); }
    use crate::desktop::ux::seams::sessions::{SessionKind, WorkerSession};
    let sessions = [WorkerSession { id:"s".into(),run:"r".into(),unit:Some("u".into()),title:"t".into(),kind:SessionKind::Terminal }];
    for (run,unit) in [("r",Some("u")), ("other",Some("u")), ("r",None)] { assert_eq!(worker_session(run,unit,&sessions), crate::desktop::concurrency::worker_session(run,unit,&sessions)); }
}

#[test]
fn wrapper_has_no_second_global() {
    let wrapper = include_str!("../../plain_text.rs");
    assert!(!wrapper.contains("impl Global")); assert!(!wrapper.contains("static "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{prelude::v1::test, sync::Mutex};

    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        response: String,
    }
    impl Fake {
        fn new(response: &str) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                response: response.into(),
            }
        }
        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }
    impl Runner for Fake {
        fn run(&self, prompt: &str) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::from_str(prompt).unwrap());
            Ok(self.response.clone())
        }
    }
    fn process(core: &mut Core, runner: &dyn Runner) -> bool {
        process_batch(core, runner, None)
    }

    #[test]
    fn original_then_rewrite_and_disabled_original() {
        let text: SharedString = "Original".into();
        let mut core = Core::default();
        let Source::Fake(runner) = Source::Fake(Arc::new(Fake::new(r#"["譯文"]"#))) else { unreachable!() };
        assert_eq!(plain_result(&text, core.lookup(&text, true)), Plain { shown: text.clone(), original: None });
        assert!(process(&mut core, runner.as_ref()));
        assert_eq!(plain_result(&text, core.lookup(&text, true)), Plain { shown: "譯文".into(), original: Some(text.clone()) });
        assert_eq!(plain_result(&text, core.lookup(&text, false)), Plain { shown: text, original: None });
    }

    #[test]
    fn claude_arguments_are_pinned() {
        assert_eq!(CLAUDE_ARGS, [
            "-p", "--model", "claude-haiku-5-5", "--tools", "", "--strict-mcp-config",
            "--setting-sources", "local", "--output-format", "text",
        ]);
    }

    #[test]
    fn demo_runner_covers_demo_unit_titles_and_log_lines() {
        let state = crate::desktop::demo::state();
        let texts: Vec<&str> = state.runs.iter().flat_map(|run| {
            run.units.iter().map(|unit| unit.title.as_ref())
                .chain(run.log.iter().map(|line| line.text.as_ref()))
        }).collect();
        assert!(!texts.is_empty());
        let response = DemoRunner.run(&serde_json::to_string(&texts).unwrap()).unwrap();
        let translated: Vec<String> = serde_json::from_str(&response).unwrap();
        assert_eq!(translated.len(), texts.len());
        for (text, result) in texts.iter().zip(translated.iter()) {
            if wants(text) {
                assert_ne!(*text, result, "missing demo translation: {text}");
            }
        }
    }

    #[test]
    fn batch_maps_responses_in_order_and_deduplicates() {
        let mut core = Core::default();
        let fake = Fake::new(r#"["第一", "第二"]"#);
        assert_eq!(core.lookup("First", true), None);
        core.lookup("First", true);
        core.lookup("Second", true);
        assert_eq!(core.queue.len(), 2);
        assert!(process(&mut core, &fake));
        assert_eq!(*fake.calls.lock().unwrap(), vec![vec!["First", "Second"]]);
        assert_eq!(core.lookup("First", true).as_deref(), Some("第一"));
        assert_eq!(core.lookup("Second", true).as_deref(), Some("第二"));
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn cache_survives_restart_and_changed_text_misses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let fake = Fake::new(r#"["原文翻譯"]"#);
        let mut first = Core::default();
        first.lookup("Original text", true);
        assert!(process_batch(&mut first, &fake, Some(&path)));
        let mut restarted = Core {
            cache: load(&path),
            ..Core::default()
        };
        assert_eq!(
            restarted.lookup("Original text", true).as_deref(),
            Some("原文翻譯")
        );
        assert_eq!(restarted.lookup("Original text changed", true), None);
        assert_eq!(restarted.queue.len(), 1);
        let before = std::fs::read(&path).unwrap();
        let invalid_path = dir.path().join("invalid.json");
        assert!(!process_batch(&mut restarted, &Fake::new("[]"), Some(&invalid_path)));
        assert!(!invalid_path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"{bad").unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn bad_responses_are_not_cached_or_retried() {
        for response in ["not json", "[]", "[1]"] {
            let mut core = Core::default();
            let fake = Fake::new(response);
            core.lookup("Failure", true);
            assert!(!process(&mut core, &fake));
            assert!(core.cache.is_empty());
            core.lookup("Failure", true);
            assert!(core.queue.is_empty());
            assert!(!process(&mut core, &fake));
            assert_eq!(fake.calls(), 1);
        }
    }

    #[test]
    fn disabled_or_english_never_enqueue() {
        let mut core = Core::default();
        for (on, lang) in [(false, Lang::ZhTw), (true, Lang::En)] {
            assert_eq!(core.lookup("Some English", active(on, lang)), None);
            assert!(core.queue.is_empty());
            assert!(core.take_batch().is_empty());
        }
        assert_eq!(core.lookup("Some English", active(true, Lang::ZhTw)), None);
        assert!(!core.queue.is_empty());
    }

    #[test]
    fn filters_wanted_text() {
        assert!(wants("Error at file.rs:12"));
        assert!(!wants("繁體中文"));
        assert!(!wants("混合 English"));
        assert!(!wants("English 日本語"));
        assert!(!wants("English 한국어"));
        assert!(!wants("12345"));
        assert!(wants(&"a".repeat(1999)));
        assert!(!wants(&"a".repeat(2000)));
    }

    #[test]
    fn batches_are_limited_to_forty() {
        let mut core = Core::default();
        for i in 0..41 {
            core.lookup(&format!("Item {i}"), true);
        }
        assert_eq!(core.take_batch().len(), 40);
        assert_eq!(core.take_batch().len(), 1);
    }
}

fn no_view_imports(source: &str) -> bool {
    use syn::visit::{self, Visit};
    fn forbidden(parts: &[String]) -> bool {
        let rest = if parts.starts_with(&["crate".into(), "desktop".into()]) { &parts[2..] }
            else if parts.first().is_some_and(|p| p == "super" || p == "desktop") { &parts[1..] }
            else { return false };
        let names = ["settings", "palette", "wishes", "control", "detail", "usage", "quota", "plain_text", "evolve", "memory", "doctor", "run_view", "todo", "mesh", "shell", "composer", "mini", "superswitch", "research", "paper", "launch"];
        rest.first().is_some_and(|p| names.contains(&p.as_str())) ||
            rest.first().is_some_and(|p| p == "ux") && rest.get(1).is_some_and(|p| p == "celestial" || p == "composer")
    }
    fn uses(tree: &syn::UseTree, path: &mut Vec<String>) -> bool {
        match tree {
            syn::UseTree::Path(p) => { path.push(p.ident.to_string()); let found = uses(&p.tree, path); path.pop(); found }
            syn::UseTree::Name(n) => { path.push(n.ident.to_string()); let found = forbidden(path); path.pop(); found }
            syn::UseTree::Rename(n) => { path.push(n.ident.to_string()); let found = forbidden(path); path.pop(); found }
            syn::UseTree::Glob(_) => forbidden(path),
            syn::UseTree::Group(g) => g.items.iter().any(|item| uses(item, path)),
        }
    }
    struct Scan(bool);
    impl<'ast> Visit<'ast> for Scan {
        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) { self.0 |= uses(&item.tree, &mut Vec::new()); }
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.0 |= forbidden(&path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>());
            visit::visit_path(self, path);
        }
    }
    let mut scan = Scan(false);
    scan.visit_file(&syn::parse_file(source).unwrap());
    !scan.0
}
#[test]
fn no_view_paths_and_negative_controls() {
    let source = include_str!("work.rs").split("#[cfg(test)]").next().unwrap();
    assert!(no_view_imports(source));
    assert!(!no_view_imports("use crate::desktop::wishes::Filter;"));
    assert!(!no_view_imports("use crate::desktop::{control::offer};"));
}
