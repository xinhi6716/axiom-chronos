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
fn write_paper_entry_opens_page(cx: &mut TestAppContext) {
    let _home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let fake = Arc::new(FakePaper::new());
        let (run_state, session) = state(&fake, false);
        fake.set_status(&session, status());
        let (window, entity) = open(cx, lang, run_state, Screen::Run);
        assert!(shows(
            &capture(cx, window),
            "write-paper",
            label(lang, "Write paper", "撰寫論文")
        ));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        capture(cx, window);
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
            click(cx, window, ("paper-input", index));
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
        assert!(fake.calls().iter().any(|c| matches!(c, PaperCall::RequestShortlist(id, req, None) if id == &session && req == &expected)), "{:?}", fake.calls());
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
        pass(PassId::ArgonRevise, "argon", "Checked quantities"),
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
        for text in ["Astra · Star", "Fable · Jupiter", "Argon · Gemini"] {
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
        assert!(fake.calls().iter().any(|call| matches!(call, PaperCall::RequestShortlist(id, req, None) if id == &session && req == &expected)), "{:?}", fake.calls());
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
    s.shortlist = None;
    s.estimate = None;
    fake.set_status(&session, s);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    capture(cx, window);
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
