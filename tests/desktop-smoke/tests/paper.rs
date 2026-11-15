#![allow(dead_code)]
mod ux_support;

use axiom::{
    base::Failure,
    council::Status,
    desktop::{
        demo,
        i18n::Lang,
        model::{AppState, Screen},
        ux::{
            seams::{FakePaper, PageImage, PaperCall, Preview},
            CouncilView, Page,
        },
    },
    paper::{service::PaperStatus, *},
};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};
use ux_support::{capture, open, shows};

// AXIOM_HOME is process-global, so these tests keep their temporary homes isolated.
static HOME_LOCK: Mutex<()> = Mutex::new(());
struct Home {
    _lock: MutexGuard<'static, ()>,
    path: PathBuf,
    prior: Option<OsString>,
}
impl Home {
    fn new() -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("paper-smoke-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let prior = std::env::var_os("AXIOM_HOME");
        std::env::set_var("AXIOM_HOME", &path);
        Self {
            _lock: lock,
            path,
            prior,
        }
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        if let Some(prior) = &self.prior {
            std::env::set_var("AXIOM_HOME", prior);
        } else {
            std::env::remove_var("AXIOM_HOME");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn status() -> PaperStatus {
    PaperStatus {
        steps: vec![],
        evidence: None,
        requirements: None,
        shortlist: None,
        choice: None,
        estimate: None,
        passes: vec![],
        ready: None,
        pdf: None,
        running: false,
        last_error: None,
    }
}
fn estimate() -> SpendEstimate {
    SpendEstimate {
        per_member: BTreeMap::new(),
        priced_usd: Some(1.25),
        samples: 1,
        unpriced: vec![],
        line: "Estimated $1.25".into(),
        digest: "estimate-digest".into(),
    }
}
fn unknown<T>() -> Known<T> {
    Known::Unknown {
        reason: "not supplied".into(),
    }
}
fn journal(id: &str, name: &str, known: bool) -> RankedJournal {
    let source = SourceTag {
        kind: SourceKind::Sjr,
        url: None,
        fetched_at: "2025-03-04".into(),
        data_year: None,
        raw: "SJR".into(),
    };
    RankedJournal {
        journal: JournalRecord {
            id: id.into(),
            title: name.into(),
            issns: vec![],
            issn_l: None,
            publisher: unknown(),
            homepage: unknown(),
            quartiles: vec![],
            sjr: if known {
                Known::Known(Sourced { value: 1.5, source })
            } else {
                unknown()
            },
            access: unknown(),
            apc: unknown(),
            review_weeks: unknown(),
            topics: vec![],
            works_count: unknown(),
        },
        bucket: Bucket::NeedsCheck,
        filters: vec![],
        fit: 0.9,
        terms: vec![],
        free_text: vec![],
    }
}
fn shortlist(known: bool) -> Shortlist {
    Shortlist {
        requirements_hash: "requirements-hash".into(),
        ranked: vec![
            journal("first", "First Journal", known),
            journal("second", "Second Journal", false),
        ],
        excluded: vec![],
        generated_at: "2025-03-04".into(),
        hash: "shortlist-hash".into(),
    }
}
fn choice() -> JournalChoice {
    JournalChoice {
        journal_id: "first".into(),
        shortlist_hash: "shortlist-hash".into(),
        chosen_by: Chooser::Person,
        at: "2025-03-04".into(),
    }
}
fn pass(id: PassId, member: &str, reason: &str) -> PassRecord {
    PassRecord {
        pass: id,
        member: member.into(),
        model: None,
        provider: "fake".into(),
        status: Status::Ok,
        evidence_hash: "evidence".into(),
        input_hash: "input".into(),
        output_hash: Some("output".into()),
        usd: None,
        tokens: 0,
        raw_path: String::new(),
        change_log: vec![Change {
            location: "abstract".into(),
            kind: ChangeKind::Edited,
            reason: reason.into(),
        }],
        validation: vec![],
        at: "2025-03-04".into(),
    }
}
fn state(fake: &Arc<FakePaper>, page: bool) -> (AppState, String) {
    let mut state = demo::state();
    state.reduce_motion = true;
    state.backend = None;
    let run = &mut state.runs[state.current.unwrap()];
    run.cwd = None;
    let session = run.session.to_string();
    state.ux.council.clear();
    state
        .ux
        .council
        .insert(session.clone().into(), CouncilView::default());
    state.ux.seams.paper = Some(fake.clone());
    if page {
        state.ux.page = Some(Page::Paper(session.clone().into()));
    }
    (state, session)
}
fn setup(
    cx: &mut TestAppContext,
    lang: Lang,
    scripted: PaperStatus,
) -> (AnyWindowHandle, Arc<FakePaper>, String) {
    let fake = Arc::new(FakePaper::new());
    let (state, session) = state(&fake, true);
    if let Some(list) = &scripted.shortlist {
        fake.set_shortlist(&session, list.clone());
    }
    if let Some(estimate) = &scripted.estimate {
        fake.set_estimate(&session, estimate.clone());
    }
    fake.set_status(&session, scripted);
    let (window, _) = open(cx, lang, state, Screen::Run);
    (window, fake, session)
}
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    capture(cx, window);
}
fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> bool {
    let id: ElementId = id.to_owned().into();
    capture(cx, window);
    cx.update_window(window, |_, w, _| {
        w.try_find(id.clone()).is_some()
            || w.elements().iter().any(|e| e.path().contains(&id))
            || gpui_kit::base::test_support::texts(w)
                .iter()
                .any(|t| t.visible() && t.path().contains(&id))
    })
    .unwrap()
}
fn label(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    if lang == Lang::En {
        en
    } else {
        zh
    }
}

#[gpui_kit::test]
fn readiness_panel_has_observable_content(cx: &mut TestAppContext) {
    let _home = Home::new();
    let (window, _, _) = setup(cx, Lang::En, status());
    capture(cx, window);
    cx.update_window(window, |_, window, _| {
        let panel = ElementId::from("paper.readiness.status");
        let label = ElementId::from("paper-readiness-label");
        assert!(window
            .elements()
            .iter()
            .any(|element| { element.path().contains(&panel) && element.path().contains(&label) }));
    })
    .unwrap();
}

#[gpui_kit::test]
fn declared_paper_panels_are_elements_on_the_page(cx: &mut TestAppContext) {
    let _home = Home::new();
    let mut scripted = status();
    scripted.shortlist = Some(shortlist(true));
    scripted.estimate = Some(estimate());
    let (window, _, _) = setup(cx, Lang::En, scripted);
    for target in axiom::desktop::targets::all()
        .into_iter()
        .filter(|t| t.group == "paper")
    {
        for &panel in target.panels {
            assert!(shown(cx, window, panel), "{}: missing {panel}", target.id);
        }
    }
}

macro_rules! declared_target_ids_test {
    ($test:ident, $target:literal) => {
        #[gpui_kit::test]
        fn $test(cx: &mut TestAppContext) {
            let _home = Home::new();
            let fake = Arc::new(FakePaper::new());
            let (state, session) = state(&fake, true);
            let mut scripted = status();
            scripted.shortlist = Some(shortlist(true));
            scripted.estimate = Some(estimate());
            scripted.running = true;
            scripted.pdf = Some("target-id.pdf".into());
            fake.set_status(&session, scripted);
            fake.set_preview(
                &session,
                Preview::Unavailable {
                    reason: "unavailable".into(),
                    pdf: "target-id.pdf".into(),
                },
            );
            let (window, _) = open(cx, Lang::En, state, Screen::Run);
            let target = axiom::desktop::targets::find($target).unwrap();
            for &id in target.panels.iter().chain(target.actions.iter()) {
                assert!(shown(cx, window, id), "{}: missing {id}", target.id);
            }
        }
    };
}
declared_target_ids_test!(paper_page_declared_ids, "paper");
declared_target_ids_test!(paper_form_declared_ids, "paper.form");
declared_target_ids_test!(paper_shortlist_declared_ids, "paper.shortlist");
declared_target_ids_test!(paper_estimate_declared_ids, "paper.estimate");
declared_target_ids_test!(paper_progress_declared_ids, "paper.progress");
declared_target_ids_test!(paper_changelog_declared_ids, "paper.changelog");
declared_target_ids_test!(paper_preview_declared_ids, "paper.preview");
declared_target_ids_test!(paper_readiness_declared_ids, "paper.readiness");

#[gpui_kit::test]
fn slow_status_feed_keeps_form_usable_and_delivers_updates(cx: &mut TestAppContext) {
    use axiom::desktop::data::faults::{self, Fault, FaultPlan};
    use std::time::{Duration, Instant};
    let _home = Home::new();
    let fake = Arc::new(FakePaper::new());
    let (state, session) = state(&fake, true);
    let mut first = status();
    first.last_error = Some(("first".into(), "first update".into()));
    fake.set_status(&session, first);
    cx.update(|cx| {
        let mut plan = FaultPlan::default();
        plan.0.insert("paper.page".into(), Fault::Slow { ms: 1500 });
        faults::install(plan, cx);
    });
    let (window, _) = open(cx, Lang::En, state, Screen::Run);
    assert!(shown(cx, window, "paper-form"));
    click(cx, window, ("paper-tier", 1_usize));
    cx.update(|cx| faults::install(FaultPlan::default(), cx));
    let wait_for = |cx: &mut TestAppContext, expected: &str| {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            cx.executor().advance_clock(Duration::from_millis(50));
            let lines = capture(cx, window);
            if shows(&lines, "paper-error", expected) {
                break;
            }
            assert!(Instant::now() < deadline, "missing {expected}: {lines:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for(cx, "first update");
    let mut second = status();
    second.last_error = Some(("second".into(), "second update".into()));
    fake.set_status(&session, second);
    wait_for(cx, "second update");
}

#[gpui_kit::test]
fn every_paper_target_prepares_and_opens_each_declared_state(cx: &mut TestAppContext) {
    use axiom::desktop::{targets, theme, Shell, Store};
    let _home = Home::new();
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(gpui_kit::component::ThemeMode::Dark, None, cx);
        axiom::desktop::i18n::set(Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let fake = Arc::new(FakePaper::new());
    let (initial, _) = state(&fake, false);
    let state = cx.new(|_| initial);
    let shell_state = state.clone();
    let (window, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(shell_state, false, window, cx))
            })
        })
        .unwrap();
    for target in targets::all()
        .into_iter()
        .filter(|target| target.group == "paper")
    {
        for &capture in target.states {
            state.update(cx, |state, _| {
                targets::prepare(&target, capture, state).unwrap();
                if matches!(target.opening, targets::Opening::PaperStep(_)) {
                    state.ux.page = None;
                }
            });
            cx.update_window(window, |_, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_target(&target, capture, window, cx)
                })
            })
            .unwrap()
            .unwrap();
            assert!(
                matches!(
                    state.read_with(cx, |state, _| state.ux.page.clone()),
                    Some(Page::Paper(_))
                ),
                "{}",
                target.id
            );
        }
    }
}

#[gpui_kit::test]
fn write_paper_entry_opens_page(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let fake = Arc::new(FakePaper::new());
        let (run_state, session) = state(&fake, false);
        fake.set_status(&session, status());
        let (window, entity) = open(cx, lang, run_state, Screen::Run);
        assert!(shown(cx, window, "write-paper"));
        click(cx, window, "write-paper");
        assert_eq!(
            entity.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Paper(session.into()))
        );
        assert!(shown(cx, window, "paper-page"));
        assert!(shown(cx, window, "paper-form"));
        let (mut no_service, _) = state(&fake, false);
        no_service.ux.seams.paper = None;
        let (window, _) = open(cx, lang, no_service, Screen::Run);
        assert!(!shown(cx, window, "write-paper"));
        let (mut no_council, _) = state(&fake, false);
        no_council.ux.council.clear();
        let (window, _) = open(cx, lang, no_council, Screen::Run);
        assert!(!shown(cx, window, "write-paper"));
    }
}

#[gpui_kit::test]
fn cannot_start_without_chosen_journal(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        for chosen in [false, true] {
            let mut s = status();
            s.shortlist = Some(shortlist(true));
            s.estimate = Some(estimate());
            if chosen {
                s.choice = Some(choice());
            }
            let (window, fake, _) = setup(cx, lang, s);
            assert!(shown(cx, window, "paper-start"));
            click(cx, window, "paper-start");
            assert!(!fake
                .calls()
                .iter()
                .any(|c| matches!(c, PaperCall::Start(..))));
            if chosen {
                assert!(shown(cx, window, "paper-estimate"));
                click(cx, window, "paper-ack");
                click(cx, window, "paper-start");
                assert!(fake.calls().iter().any(
                    |c| matches!(c, PaperCall::Start(_, digest) if digest == "estimate-digest")
                ));
            }
        }
    }
}

#[gpui_kit::test]
fn top_rank_not_preselected(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.shortlist = Some(shortlist(true));
        let (window, fake, session) = setup(cx, lang, s.clone());
        let lines = capture(cx, window);
        for name in ["First Journal", "Second Journal"] {
            assert!(shows(&lines, "paper-row", name));
        }
        assert!(!shows(&lines, "paper-row", label(lang, "Chosen", "已選擇")));
        assert!(!fake
            .calls()
            .iter()
            .any(|c| matches!(c, PaperCall::ChooseJournal(..))));
        click(cx, window, ("paper-choose", 0_usize));
        assert!(fake.calls().iter().any(|c| matches!(c, PaperCall::ChooseJournal(s, id, hash) if s == &session && id == "first" && hash == "shortlist-hash")));
        s.choice = Some(choice());
        fake.set_status(&session, s);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            if shows(
                &capture(cx, window),
                "paper-row",
                label(lang, "Chosen", "已選擇"),
            ) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "choice update did not arrive"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let chosen_rows = cx
            .update_window(window, |_, w, _| {
                gpui_kit::base::test_support::texts(w)
                    .into_iter()
                    .filter(|t| t.visible() && t.text() == label(lang, "Chosen", "已選擇"))
                    .map(|t| t.path().to_vec())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(chosen_rows.len(), 1, "{chosen_rows:?}");
        assert!(
            chosen_rows[0].contains(&ElementId::from(("paper-row-slot", 0_usize))),
            "{chosen_rows:?}"
        );
        assert!(
            !chosen_rows[0].contains(&ElementId::from(("paper-row-slot", 1_usize))),
            "{chosen_rows:?}"
        );
    }
}

#[gpui_kit::test]
fn stale_estimate_is_visible_and_blocks_start(cx: &mut TestAppContext) {
    let _home = Home::new();
    let mut scripted = status();
    scripted.shortlist = Some(shortlist(true));
    scripted.choice = Some(choice());
    scripted.estimate = Some(estimate());
    let (window, fake, session) = setup(cx, Lang::En, scripted);
    click(cx, window, "paper-ack");
    let mut current = estimate();
    current.digest = "changed-estimate".into();
    fake.set_estimate(&session, current);
    // Feed schedules against Instant, not the test executor's virtual clock.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let lines = loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        let lines = capture(cx, window);
        if shows(
            &lines,
            "paper-estimate-stale",
            "Estimate is stale; request a new estimate",
        ) || std::time::Instant::now() >= deadline
        {
            break lines;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        shows(
            &lines,
            "paper-estimate-stale",
            "Estimate is stale; request a new estimate"
        ),
        "{lines:?}"
    );
    click(cx, window, "paper-start");
    assert!(!fake
        .calls()
        .iter()
        .any(|call| matches!(call, PaperCall::Start(..))));
}

#[gpui_kit::test]
fn sources_and_unknowns_visible(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.shortlist = Some(shortlist(true));
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        assert!(
            shows(&lines, "paper-metric", "SJR: 1.5 · SJR · 2025-03-04"),
            "{lines:?}"
        );
        assert!(
            shows(&lines, "paper-metric", label(lang, "Unknown", "未知")),
            "{lines:?}"
        );

        // Negative control: no known metrics in either row, yet neither is filtered out.
        let mut s = status();
        s.shortlist = Some(shortlist(false));
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        for name in ["First Journal", "Second Journal"] {
            assert!(shows(&lines, "paper-row", name), "{lines:?}");
        }
        assert!(
            shows(&lines, "paper-metric", label(lang, "Unknown", "未知")),
            "{lines:?}"
        );
    }
}

#[gpui_kit::test]
fn buckets_exclusions_and_jcr_source(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut list = shortlist(true);
        list.ranked[0].bucket = Bucket::Passes;
        list.ranked[0].journal.quartiles.push(Sourced {
            value: QuartileValue {
                quartile: Quartile::Q1,
                scheme: QuartileScheme::Jcr,
                category: "Medicine".into(),
                year: 2024,
            },
            source: SourceTag {
                kind: SourceKind::JcrImport,
                url: None,
                fetched_at: "2025-03-04".into(),
                data_year: Some(2024),
                raw: "JCR".into(),
            },
        });
        list.excluded.push(Excluded {
            journal: journal("excluded", "Excluded Journal", false).journal,
            filters: vec![FilterResult {
                name: "APC".into(),
                status: ReqStatus::Unmet,
                detail: "exceeds limit".into(),
                source: None,
            }],
        });
        let mut s = status();
        s.shortlist = Some(list);
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        for (id, text) in [
            ("paper-row", label(lang, "Passes", "符合")),
            ("paper-row", label(lang, "Needs check", "需確認")),
            (
                "paper-excluded",
                label(
                    lang,
                    "Excluded · Excluded Journal",
                    "已排除 · Excluded Journal",
                ),
            ),
            (
                "paper-excluded",
                label(
                    lang,
                    "APC: Unmet · exceeds limit",
                    "APC: 不符合 · exceeds limit",
                ),
            ),
            ("paper-metric", "SJR: 1.5 · SJR · 2025-03-04"),
            (
                "paper-metric",
                label(
                    lang,
                    "Quartile: JCR Q1 (Medicine, 2024) · JCR · 2025-03-04",
                    "分區: JCR Q1 (Medicine, 2024) · JCR · 2025-03-04",
                ),
            ),
        ] {
            assert!(shows(&lines, id, text), "missing {id} / {text}: {lines:?}");
        }
    }
}

#[gpui_kit::test]
fn edited_form_and_cancel(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.shortlist = None;
        let (window, fake, session) = setup(cx, lang, s.clone());
        for (index, value) in [
            (0_usize, "Research abstract"),
            (1, "alpha, beta"),
            (2, "123"),
            (3, "USD"),
            (4, "medicine, biology"),
            (5, "fast review"),
        ] {
            click(
                cx,
                window,
                (ElementId::from(("paper-input", index)), "input"),
            );
            cx.update_window(window, |_, w, cx| w.input(value, cx))
                .unwrap();
            capture(cx, window);
        }
        click(cx, window, ("paper-tier", 2_usize));
        click(cx, window, ("paper-scheme", 1_usize));
        click(cx, window, ("paper-access", 1_usize));
        click(cx, window, "paper-no-fee");
        click(cx, window, "paper-shortlist");
        let expected = Requirements {
            abstract_text: "Research abstract".into(),
            keywords: vec!["alpha".into(), "beta".into()],
            max_quartile: Some(Quartile::Q2),
            tier_scheme: QuartileScheme::Jcr,
            access: AccessWanted::Subscription,
            apc_limit: Some(Money {
                amount_minor: 123,
                currency: "USD".into(),
            }),
            require_no_fee: true,
            categories: vec!["medicine".into(), "biology".into()],
            free_text: vec!["fast review".into()],
        };
        assert!(fake.calls().iter().any(|c| matches!(c, PaperCall::RequestShortlist(id, req, None) if id == &session && req == &expected.clone().into())), "{:?}", fake.calls());
        s.running = true;
        fake.set_status(&session, s);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        capture(cx, window);
        click(cx, window, "paper-cancel");
        assert!(
            fake.calls()
                .iter()
                .any(|c| matches!(c, PaperCall::Cancel(id) if id == &session)),
            "{:?}",
            fake.calls()
        );
    }
}

fn passes() -> Vec<PassRecord> {
    vec![
        pass(PassId::AstraDraft, "astra", "Drafted introduction"),
        pass(PassId::FableRevise, "fable", "Clarified evidence"),
        pass(PassId::GeminiRevise, "gemini", "Checked quantities"),
        pass(PassId::FableStyle, "fable", "Polished style"),
    ]
}

#[gpui_kit::test]
fn four_pass_glyphs_correct(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.passes = passes();
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        for text in ["Astra · Star", "Fable · Jupiter", "Gemini · Gemini"] {
            assert!(shows(&lines, "paper-pass", text), "{lines:?}");
        }
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.path.iter().any(|id| id == "paper-pass")
                    && l.text.contains("Fable · Jupiter"))
                .count(),
            2,
            "{lines:?}"
        );
    }
}

#[gpui_kit::test]
fn saving_edited_models_invalidates_estimate(cx: &mut TestAppContext) {
    let _home = Home::new();
    let mut s = status();
    s.estimate = Some(estimate());
    let (window, fake, session) = setup(cx, Lang::En, s);
    capture(cx, window);
    assert!(shown(cx, window, "paper-estimate"));
    click(cx, window, "paper-ack");
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            (ElementId::from(("paper-input", 0_usize)), "input"),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-320.))),
            cx,
        );
    })
    .unwrap();
    capture(cx, window);
    click(
        cx,
        window,
        (ElementId::from(("paper-model", 2_usize)), "input"),
    );
    cx.update_window(window, |_, w, cx| {
        w.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        w.input("gemini-custom", cx);
    })
    .unwrap();
    capture(cx, window);
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            (ElementId::from(("paper-model", 2_usize)), "input"),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(320.))),
            cx,
        );
    })
    .unwrap();
    capture(cx, window);
    click(cx, window, "paper-save-models");
    let mut expected = models::PassModels::default();
    expected.gemini_revise = "gemini-custom".into();
    assert!(
        fake.calls().iter().any(|call| matches!(call,
            PaperCall::SetModels(id, models) if id == &session && models == &expected.clone().into()
        )),
        "{:?}",
        fake.calls()
    );
    let polls = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, PaperCall::Status(_)))
        .count();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    capture(cx, window);
    assert!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, PaperCall::Status(_)))
            .count()
            > polls
    );
    assert!(!shown(cx, window, "paper-estimate"));
    assert!(!shown(cx, window, "paper-ack"));
}

#[gpui_kit::test]
fn change_logs_shown(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.passes = passes();
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        for reason in [
            "Drafted introduction",
            "Clarified evidence",
            "Checked quantities",
            "Polished style",
        ] {
            assert!(shows(&lines, "paper-changelog", reason), "{lines:?}");
        }
    }
}

#[gpui_kit::test]
fn real_pdf_preview_or_unavailable_reason(cx: &mut TestAppContext) {
    let _home = Home::new();
    let pdf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/paper/pdf/sample.pdf");
    assert!(pdf.is_file(), "missing PDF fixture: {}", pdf.display());
    // A complete, valid 1x1 PNG, independent of the PDF rendering environment.
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 248, 255, 31, 0, 3, 0,
        1, 255, 252, 37, 220, 81, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    for lang in [Lang::En, Lang::ZhTw] {
        for ready in [false, true] {
            let fake = Arc::new(FakePaper::new());
            let (state, session) = state(&fake, true);
            let mut s = status();
            s.pdf = Some(pdf.clone());
            fake.set_status(&session, s);
            fake.set_preview(
                &session,
                if ready {
                    Preview::Ready(PageImage {
                        png: png.clone(),
                        width_px: 1,
                        height_px: 1,
                        page: 1,
                        pages: 1,
                    })
                } else {
                    Preview::Unavailable {
                        reason: "renderer unavailable".into(),
                        pdf: pdf.clone(),
                    }
                },
            );
            let (window, _) = open(cx, lang, state, Screen::Run);
            if ready {
                assert!(shown(cx, window, "paper-preview"));
                assert!(!shown(cx, window, "paper-preview-unavailable"));
            } else {
                assert!(shows(
                    &capture(cx, window),
                    "paper-preview-unavailable",
                    "renderer unavailable"
                ));
                click(cx, window, "paper-open-pdf");
                assert!(fake
                    .calls()
                    .iter()
                    .any(|c| matches!(c, PaperCall::OpenPdf(s) if s == &session)));
            }
        }
    }
}

#[gpui_kit::test]
fn failure_visible(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        for status_failure in [true, false] {
            let fake = Arc::new(FakePaper::new());
            let (state, session) = state(&fake, true);
            let mut s = status();
            if status_failure {
                fake.fail("status", Failure::new("offline", "status unavailable"));
            } else {
                s.last_error = Some(("blocked".into(), "paper blocked".into()));
            }
            fake.set_status(&session, s);
            let (window, _) = open(cx, lang, state, Screen::Run);
            assert!(shows(
                &capture(cx, window),
                "paper-error",
                if status_failure {
                    "status unavailable"
                } else {
                    "paper blocked"
                }
            ));
        }
    }
}

#[gpui_kit::test]
fn zh_tw_and_english(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        s.shortlist = Some(shortlist(false));
        let (window, _, _) = setup(cx, lang, s);
        let lines = capture(cx, window);
        assert!(shown(cx, window, "paper-page"));
        assert!(shown(cx, window, "paper-form"));
        for text in [
            label(lang, "Write paper", "撰寫論文"),
            label(lang, "Start", "開始"),
            label(lang, "Back", "返回"),
        ] {
            assert!(shows(&lines, "paper-page", text), "{lines:?}");
        }
        assert!(
            shows(&lines, "paper-metric", label(lang, "Unknown", "未知")),
            "{lines:?}"
        );
    }
}

#[gpui_kit::test]
fn shortlist_button_sends_form_payload(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = status();
        let expected = Requirements {
            abstract_text: "Research abstract".into(),
            keywords: vec!["alpha".into(), "beta".into()],
            max_quartile: Some(Quartile::Q2),
            tier_scheme: QuartileScheme::Jcr,
            access: AccessWanted::Subscription,
            apc_limit: Some(Money {
                amount_minor: 123,
                currency: "USD".into(),
            }),
            require_no_fee: true,
            categories: vec!["medicine".into()],
            free_text: vec!["fast review".into()],
        };
        s.requirements = Some(expected.clone());
        let (window, fake, session) = setup(cx, lang, s);
        click(cx, window, "paper-shortlist");
        assert!(fake.calls().iter().any(|call| matches!(call, PaperCall::RequestShortlist(id, req, None) if id == &session && req == &expected.clone().into())), "{:?}", fake.calls());
    }
}

#[gpui_kit::test]
fn late_evidence_and_cleared_estimate_are_applied(cx: &mut TestAppContext) {
    let home = Home::new();
    let mut s = status();
    s.shortlist = Some(shortlist(true));
    s.choice = Some(choice());
    s.estimate = Some(estimate());
    let (window, fake, session) = setup(cx, Lang::En, s.clone());
    click(cx, window, "paper-ack");
    let goal = axiom::goal::open(
        &home.path,
        &session,
        "wish",
        Some(home.path.to_str().unwrap()),
    )
    .unwrap();
    let store = axiom::orch::store_for_goal(&home.path, &goal).unwrap();
    evidence::save(
        &store,
        &EvidenceBundle {
            version: 1,
            program: "program".into(),
            question: "Late evidence question".into(),
            artifacts: vec![],
            claims: vec![],
            evidence: vec![],
            quantities: vec![],
            references: vec![],
            hash: "evidence-hash".into(),
        },
    )
    .unwrap();
    s.evidence = Some("evidence-hash".into());
    fake.set_evidence_question(&session, "Late evidence question");
    s.shortlist = None;
    s.estimate = None;
    fake.set_status(&session, s);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while shown(cx, window, "paper-estimate") && std::time::Instant::now() < deadline {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!shown(cx, window, "paper-estimate"));
    click(cx, window, "paper-start");
    assert!(!fake
        .calls()
        .iter()
        .any(|c| matches!(c, PaperCall::Start(..))));
    click(cx, window, "paper-shortlist");
    assert!(fake.calls().iter().any(|c| matches!(c, PaperCall::RequestShortlist(_, req, _) if req.abstract_text == "Late evidence question")), "{:?}", fake.calls());
}

#[gpui_kit::test]
fn rejected_choice_preserves_estimate(cx: &mut TestAppContext) {
    let _home = Home::new();
    let mut s = status();
    s.shortlist = Some(shortlist(true));
    s.choice = Some(choice());
    s.estimate = Some(estimate());
    let (window, fake, _) = setup(cx, Lang::En, s);
    fake.fail(
        "choose_journal",
        Failure::new("rejected", "choice rejected"),
    );
    click(cx, window, "paper-ack");
    click(cx, window, ("paper-choose", 1_usize));
    assert!(shown(cx, window, "paper-estimate"));
    click(cx, window, "paper-start");
    assert!(fake
        .calls()
        .iter()
        .any(|c| matches!(c, PaperCall::Start(_, digest) if digest == "estimate-digest")));
}

#[gpui_kit::test]
fn validation_and_enum_labels_are_localized(cx: &mut TestAppContext) {
    let _home = Home::new();
    let mut s = status();
    let mut p = pass(PassId::AstraDraft, "astra", "reason");
    p.validation.push("validation detail".into());
    s.passes.push(p);
    s.steps
        .push((Step::Evidence, false, Some("step detail".into())));
    let (window, _, _) = setup(cx, Lang::ZhTw, s);
    let lines = capture(cx, window);
    assert!(shows(&lines, "paper-validation", "validation detail"));
    assert!(!shows(&lines, "paper-error", "validation detail"));
    assert!(shows(&lines, "paper-changelog", "編輯"));
    assert!(shows(&lines, "paper-error", "證據"));
}

// Hold the first render after it has captured A while B is selected and rendered.
#[gpui_kit::test]
fn switching_pdf_discards_in_flight_preview(cx: &mut TestAppContext) {
    use axiom::desktop::ux::seams::{
        EstimateRow, PaperService, PaperView, PassModels, PreviewInput, PreviewView,
        ShortlistInput, StartInput,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    struct Delayed {
        fake: Arc<FakePaper>,
        calls: AtomicUsize,
        started: AtomicBool,
        release: AtomicBool,
    }
    impl PaperService for Delayed {
        fn status(&self, s: &str) -> axiom::base::Res<PaperView> {
            self.fake.status(s)
        }
        fn models(&self, s: &str) -> axiom::base::Res<PassModels> {
            self.fake.models(s)
        }
        fn set_models(&self, s: &str, m: PassModels) -> axiom::base::Res<()> {
            self.fake.set_models(s, m)
        }
        fn request_shortlist(&self, s: &str, i: ShortlistInput) -> axiom::base::Res<()> {
            self.fake.request_shortlist(s, i)
        }
        fn choose_journal(&self, s: &str, id: &str, hash: &str) -> axiom::base::Res<()> {
            self.fake.choose_journal(s, id, hash)
        }
        fn estimate(&self, s: &str) -> axiom::base::Res<EstimateRow> {
            self.fake.estimate(s)
        }
        fn estimate_is_fresh(&self, s: &str, d: &str) -> axiom::base::Res<bool> {
            self.fake.estimate_is_fresh(s, d)
        }
        fn start(&self, s: &str, i: StartInput) -> axiom::base::Res<()> {
            self.fake.start(s, i)
        }
        fn cancel(&self, s: &str) -> axiom::base::Res<()> {
            self.fake.cancel(s)
        }
        fn open_pdf(&self, s: &str) -> axiom::base::Res<()> {
            self.fake.open_pdf(s)
        }
        fn preview(&self, s: &str, i: PreviewInput) -> axiom::base::Res<PreviewView> {
            let value = self.fake.preview(s, i);
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.started.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(10);
                while !self.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            value
        }
    }
    let _home = Home::new();
    let fake = Arc::new(FakePaper::new());
    let (mut state, session) = state(&fake, true);
    let delayed = Arc::new(Delayed {
        fake: fake.clone(),
        calls: AtomicUsize::new(0),
        started: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    state.ux.seams.paper = Some(delayed.clone());
    let mut a = status();
    a.pdf = Some("a.pdf".into());
    fake.set_status(&session, a);
    let page = |pages| {
        Preview::Ready(PageImage {
            png: vec![
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 248,
                255, 31, 0, 3, 0, 1, 255, 252, 37, 220, 81, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
                96, 130,
            ],
            width_px: 1,
            height_px: 1,
            page: 1,
            pages,
        })
    };
    fake.set_preview(&session, page(11));
    let (window, _) = open(cx, Lang::En, state, Screen::Run);
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "paper.progress.passes",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-240.))),
            cx,
        );
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !delayed.started.load(Ordering::SeqCst) {
        cx.executor().advance_clock(Duration::from_millis(50));
        capture(cx, window);
        assert!(Instant::now() < deadline, "A render did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut b = status();
    b.pdf = Some("b.pdf".into());
    fake.set_status(&session, b);
    fake.set_preview(&session, page(22));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        cx.executor().advance_clock(Duration::from_millis(50));
        let lines = capture(cx, window);
        assert!(!shows(&lines, "paper.preview.document", "1 / 11"));
        if shows(&lines, "paper.preview.document", "1 / 22") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "B preview did not arrive: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    delayed.release.store(true, Ordering::SeqCst);
    for _ in 0..30 {
        cx.executor().advance_clock(Duration::from_millis(50));
        let lines = capture(cx, window);
        assert!(!shows(&lines, "paper.preview.document", "1 / 11"));
        assert!(shows(&lines, "paper.preview.document", "1 / 22"));
        std::thread::sleep(Duration::from_millis(5));
    }
}
