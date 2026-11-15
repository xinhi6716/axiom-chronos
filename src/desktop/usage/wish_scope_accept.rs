use super::{
    compare::compare,
    fmt_cost,
    wish_scope::{aggregate, search, wishes},
};
use crate::desktop::{
    model::{ModelCost, Usage},
    ux::{prefs::UxPrefs, usage::UsageSession},
};

fn session(wish: &str, id: &str, usd: Option<f64>, time: u64) -> UsageSession {
    UsageSession {
        wish: wish.to_owned().into(),
        session: id.to_owned().into(),
        label: format!("Wish {wish}").into(),
        usage: Usage {
            total_usd: usd,
            models: vec![ModelCost {
                model: "model".into(),
                usd,
                input: Some(1),
                cached: Some(0),
                cache_write: Some(0),
                output: Some(1),
                total: Some(2),
            }],
            ..Default::default()
        },
        complete: true,
        started_at: Some(time),
    }
}
#[test]
fn scoped_aggregate_compare_and_search() {
    let sessions = vec![
        session("a", "one", Some(1.), 1),
        session("a", "two", Some(2.), 3),
        session("a", "two", Some(2.), 3),
        session("b", "three", Some(5.), 2),
    ];
    let a = aggregate(&sessions, Some("a"));
    assert_eq!(
        (a.sessions, a.total_usd, a.models[0].usd),
        (2, Some(3.), Some(3.))
    );
    let (a, b, delta) = compare(&sessions, Some("a"), Some("b"));
    assert_eq!(
        (a.total_usd, b.total_usd, delta),
        (Some(3.), Some(5.), Some(-2.))
    );
    let list = wishes(&sessions);
    assert_eq!(
        list.iter().map(|x| x.0.as_ref()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(search(&list, "WISH B"), vec![1]);
}
#[test]
fn unknowns_and_prefs_round_trip() {
    let mut missing = session("a", "two", None, 2);
    missing.complete = false;
    let sessions = vec![
        session("a", "one", Some(1.), 1),
        missing,
        session("b", "three", Some(5.), 3),
    ];
    let a = aggregate(&sessions, Some("a"));
    assert!(a.incomplete);
    assert_eq!(a.total_usd, None);
    assert_eq!(a.models[0].usd, None);
    assert_eq!(fmt_cost(a.total_usd), "—");
    assert_eq!(compare(&sessions, Some("a"), Some("b")).2, None);
    let prefs = UxPrefs {
        usage_wish: Some("a".into()),
        usage_compare: Some("b".into()),
        ..Default::default()
    };
    assert_eq!(UxPrefs::parse(&prefs.to_json()), prefs);
}

#[test]
fn incomplete_session_and_duplicate_ids_use_first_occurrence_before_wish_filter() {
    let mut active = session("a", "running", Some(4.0), 4);
    active.complete = false;
    let sessions = vec![
        session("a", "same", Some(1.0), 1),
        session("b", "same", Some(99.0), 2),
        active,
        session("b", "other", Some(2.0), 3),
    ];
    let a = aggregate(&sessions, Some("a"));
    let b = aggregate(&sessions, Some("b"));
    assert!(a.incomplete);
    assert_eq!(a.total_usd, None);
    assert_eq!(a.models[0].usd, None);
    assert_eq!((b.sessions, b.total_usd), (1, Some(2.0)));
    assert_eq!(search(&wishes(&sessions), "OTHER"), Vec::<usize>::new());
}

#[test]
fn duplicate_session_first_wish_wins_without_changing_its_total() {
    let first = session("a", "same", Some(1.0), 1);
    let duplicate = session("b", "same", Some(99.0), 2);
    let baseline = aggregate(std::slice::from_ref(&first), Some("a"));
    let sessions = vec![first, duplicate, session("b", "other", Some(2.0), 3)];
    assert_eq!(aggregate(&sessions, Some("a")), baseline);
    let b = aggregate(&sessions, Some("b"));
    assert_eq!((b.sessions, b.total_usd), (1, Some(2.0)));
}

#[test]
fn usage_sessions_include_unmatched_metadata_and_count_missing_once() {
    let mut state = crate::desktop::demo::state();
    state.runs.clear();
    let mut missing = session("a", "missing", None, 1);
    missing.complete = false;
    state.ux.usage_sessions = vec![missing.clone(), missing, session("b", "other", None, 2), session("a", "known", Some(3.0), 3)];
    let sessions = super::usage_sessions(&state);
    assert_eq!(wishes(&sessions).len(), 2);
    let selected = super::wish_scope::unique(&sessions, None);
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.iter().filter(|s| super::wish_scope::is_incomplete(s)).count(), 2);
    assert_eq!(aggregate(&sessions, Some("a")).sessions, 2);
    assert_eq!(aggregate(&sessions, Some("a")).total_usd, None);
    let mut summary = super::Summary::default();
    for s in selected { super::add_usage_details(&mut summary, &s.usage); }
    assert_eq!((summary.input, summary.output), (3, 3));
    let mut run = crate::desktop::demo::sample_run();
    run.session = "known".into();
    run.usage.total_usd = Some(8.0);
    state.runs.push(run);
    let sessions = super::usage_sessions(&state);
    assert_eq!(aggregate(&sessions, Some("a")).sessions, 2);
    assert_eq!(sessions.iter().find(|s| s.session.as_ref() == "known").unwrap().usage.total_usd, Some(8.0));
}

#[test]
fn missing_wish_identity_falls_back_to_session() {
    let sessions = vec![session("", "fallback", Some(3.0), 1)];
    assert_eq!(wishes(&sessions)[0].0.as_ref(), "fallback");
    assert_eq!(aggregate(&sessions, Some("fallback")).total_usd, Some(3.0));
}
