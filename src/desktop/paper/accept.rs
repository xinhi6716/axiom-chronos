use super::*;
use crate::desktop::ornament::Celestial;
use std::prelude::v1::test;
#[test]
fn status_poll_interval_respects_floor() {
    assert!(Duration::from_secs(STATUS_POLL_SECONDS) >= Duration::from_millis(1500));
    assert!(!include_str!("mod.rs").contains("Duration::from_millis(800)"));
}

#[test]
fn gemini_pass_defaults_to_hydrogen() {
    assert_eq!(
        PassModels::default().gemini_revise,
        "gemini-4-hydrogen-xhigh"
    );
}

#[test]
fn sources_repeat_first_value_if_feed_did_not_advance_cursor() {
    let fake = Arc::new(crate::desktop::ux::seams::FakePaper::new());
    fake.set_status("session", PaperView::default());
    fake.set_preview(
        "session",
        PreviewView::Unavailable {
            reason: "no renderer".into(),
            pdf: "paper.pdf".into(),
        },
    );
    let mut status = StatusSource {
        service: fake.clone(),
        session: "session".into(),
        epoch: Arc::new(AtomicU64::new(0)),
        models: None,
        previous: None,
    };
    assert!(matches!(
        status.poll(Cursor(0)).unwrap().0,
        Delta::Replace(_)
    ));
    let (retry, cursor) = status.poll(Cursor(0)).unwrap();
    assert!(matches!(retry, Delta::Replace(_)));
    assert!(matches!(status.poll(cursor).unwrap().0, Delta::Unchanged));
    let mut preview = PreviewSource {
        service: fake,
        session: "session".into(),
        page: 1,
        generation: 1,
        loaded: false,
    };
    assert!(matches!(
        preview.poll(Cursor(0)).unwrap().0,
        Delta::Replace(_)
    ));
    let (retry, cursor) = preview.poll(Cursor(0)).unwrap();
    assert!(matches!(retry, Delta::Replace(_)));
    assert!(matches!(preview.poll(cursor).unwrap().0, Delta::Unchanged));
}

#[test]
fn late_preview_from_old_pdf_cannot_replace_new_pdf() {
    use std::sync::mpsc;
    let (release, wait) = mpsc::channel::<()>();
    let (done, receive) = mpsc::channel();
    let old = std::thread::spawn(move || {
        wait.recv().unwrap();
        done.send(PreviewSnapshot {
            generation: 1,
            result: Ok(PreviewView::Unavailable {
                reason: "document A".into(),
                pdf: "a.pdf".into(),
            }),
        })
        .unwrap();
    });
    let new = PreviewSnapshot {
        generation: 2,
        result: Ok(PreviewView::Unavailable {
            reason: "document B".into(),
            pdf: "b.pdf".into(),
        }),
    };
    let mut visible = None;
    let mut error = None;
    apply_preview(2, &mut visible, &mut error, &new);
    release.send(()).unwrap();
    let late = receive.recv().unwrap();
    apply_preview(2, &mut visible, &mut error, &late);
    old.join().unwrap();
    assert_eq!(visible, new.result.ok());
    assert!(error.is_none());
}
#[test]
fn pass_mapping() {
    for (p, n, g) in [
        (1, "Astra", Celestial::Star),
        (2, "Fable", Celestial::Jupiter),
        (3, "Gemini", Celestial::Gemini),
        (4, "Fable", Celestial::Jupiter),
    ] {
        let (name, glyph) = progress::glyph(p);
        assert_eq!(name, n);
        assert_eq!(glyph, g);
    }
}
#[test]
fn gate_requires_choice_estimate_and_ack() {
    assert!(!can_start(false, true, true, false));
    assert!(!can_start(true, false, true, false));
    assert!(!can_start(true, true, false, false));
    assert!(!can_start(true, true, true, true));
    assert!(can_start(true, true, true, false));
}

#[test]
fn stale_display_estimate_blocks_start() {
    let mut view = crate::desktop::ux::seams::PaperView::default();
    view.estimate = Some(EstimateRow {
        digest: "old".into(),
        usd: None,
        tokens: None,
        stale: true,
        line: String::new(),
    });
    assert!(!start_with_estimate(
        true,
        view.estimate.as_ref(),
        true,
        false
    ));
}
#[test]
fn no_preselection_and_stale_choice_are_rejected() {
    use crate::desktop::ux::seams::JournalRow;
    assert!(!is_chosen(None, "first", "current"));
    let choice = JournalRow {
        id: "second".into(),
        hash: "current".into(),
        ..Default::default()
    };
    assert!(!is_chosen(Some(&choice), "first", "current"));
    assert!(!is_chosen(Some(&choice), "second", "stale"));
    assert!(is_chosen(Some(&choice), "second", "current"));
}

#[test]
fn form_conversion_covers_requirements_and_jcr() {
    use crate::desktop::ux::seams::{AccessWanted, Money, Quartile, QuartileScheme};
    let mut fields: Vec<String> = [
        "abstract",
        " alpha, , beta ",
        " 123 ",
        " USD ",
        " medicine, biology,, ",
        " fast, open ",
        " reports/jcr.csv ",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let (req, path) = form::convert(
        &fields,
        Some(Quartile::Q2),
        QuartileScheme::Jcr,
        AccessWanted::Subscription,
        true,
    )
    .unwrap();
    assert_eq!(req.abstract_text, "abstract");
    assert_eq!(req.keywords, ["alpha", "beta"]);
    assert_eq!(req.categories, ["medicine", "biology"]);
    assert_eq!(req.free_text, ["fast", "open"]);
    assert_eq!(req.max_quartile, Some(Quartile::Q2));
    assert_eq!(req.tier_scheme, QuartileScheme::Jcr);
    assert_eq!(req.access, AccessWanted::Subscription);
    assert!(req.require_no_fee);
    assert_eq!(
        req.apc_limit,
        Some(Money {
            amount_minor: 123,
            currency: "USD".into()
        })
    );
    assert_eq!(path, Some(std::path::PathBuf::from("reports/jcr.csv")));
    fields[3] = " ".into();
    assert!(form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::Either,
        false
    )
    .is_err());
    fields[2] = "not a number".into();
    fields[3] = "USD".into();
    assert!(form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::OpenAccess,
        false
    )
    .is_err());
    fields[2] = " ".into();
    fields[6] = " ".into();
    let (req, path) = form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::OpenAccess,
        false,
    )
    .unwrap();
    assert_eq!(req.apc_limit, None);
    assert_eq!(req.access, AccessWanted::OpenAccess);
    assert_eq!(path, None);
}

#[test]
fn views_do_not_import_paper_or_council_engines() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/desktop/paper");
    let forbidden = [
        concat!("crate::", "paper"),
        concat!("crate::", "council"),
        concat!("mesh::", "council"),
    ];
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            for pattern in forbidden {
                assert!(
                    !source.contains(pattern),
                    "{} contains {pattern}",
                    path.display()
                );
            }
        }
    }
}
