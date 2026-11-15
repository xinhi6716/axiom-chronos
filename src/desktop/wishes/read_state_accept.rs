use super::read_state::*;
use crate::desktop::{demo::sample_run, i18n::Lang, model::{Need, NeedKind, UnitState}, settings::Settings, ux::prefs::{ReadFilter, ReadMark, UxPrefs}};
use std::prelude::v1::test;

#[test]
fn attention_and_persistence() {
    let mut run = sample_run();
    let key = attention_key(&run);
    let mut prefs = UxPrefs::default();
    assert!(is_unread(&run, &prefs));
    prefs.read.insert(run.session.to_string(), ReadMark { key, archived: false });
    assert!(!is_unread(&run, &prefs));
    let mut settings = Settings::default(); settings.ux = prefs.clone();
    assert_eq!(Settings::parse(&settings.to_json()).ux, prefs);
    assert_eq!(unread_count(&[run.clone()], &prefs), 0);
    let mut activity = run.clone(); activity.elapsed = Some("heartbeat".into()); activity.log.clear();
    assert_eq!(attention_key(&activity), key);
    run.needs.push(Need { kind: NeedKind::Blocked, title: "New need".into(), note: "detail".into() });
    assert!(is_unread(&run, &prefs));
    prefs.read.get_mut(run.session.as_ref()).unwrap().archived = true;
    assert!(!is_archived(&run, &prefs));
    let previous = attention_key(&run);
    run.units[0].state = UnitState::Failed;
    assert_ne!(attention_key(&run), previous);
    assert!(visible(&run, &prefs, ReadFilter::Unread));
}

#[test]
fn action_labels_and_archive_rules() {
    let run = sample_run(); let mut prefs = UxPrefs::default();
    assert_eq!(read_actions(&run, &prefs), vec![ReadAction::MarkRead, ReadAction::Archive]);
    prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(&run), archived: true });
    assert!(is_archived(&run, &prefs));
    assert_eq!(read_actions(&run, &prefs), vec![ReadAction::MarkUnread, ReadAction::Unarchive]);
    assert!(!visible(&run, &prefs, ReadFilter::All));
    assert!(visible(&run, &prefs, ReadFilter::Archived));
    for (action, en, zh) in [(ReadAction::MarkRead,"Mark as read","標為已讀"), (ReadAction::MarkUnread,"Mark as unread","標為未讀"), (ReadAction::Archive,"Archive","封存"), (ReadAction::Unarchive,"Move back from archive","移出封存")] {
        assert_eq!(action_label(action, Lang::En), en); assert_eq!(action_label(action, Lang::ZhTw), zh);
    }
}

#[test]
fn board_keeps_unread_or_needed_runs() {
    use crate::desktop::model::{board_runs_with_prefs, RunStatus};
    let mut state = crate::desktop::demo::state();
    let mut quiet = sample_run(); quiet.session = "quiet".into(); quiet.status = RunStatus::Active; quiet.needs.clear();
    let mut needed = quiet.clone(); needed.session = "needed".into(); needed.status = RunStatus::Blocked;
    let mut done = quiet.clone(); done.session = "done".into(); done.status = RunStatus::Done;
    state.runs = vec![quiet, needed, done];
    let mut prefs = UxPrefs::default();
    assert_eq!(board_runs_with_prefs(&state, &prefs), vec![1, 0]);
    change_mark(&state.runs[0], &mut prefs, ReadAction::MarkRead);
    assert_eq!(board_runs_with_prefs(&state, &prefs), vec![1]);
    for run in &state.runs { prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(run), archived: true }); }
    assert_eq!(board_runs_with_prefs(&state, &prefs), vec![1]);
    state.runs[0].needs.push(Need { kind: NeedKind::Blocked, title: "Attention".into(), note: "".into() });
    assert_eq!(board_runs_with_prefs(&state, &prefs), vec![0, 1]);
}

#[test]
fn mark_unmark_and_archive_transitions() {
    let mut run = sample_run(); let mut prefs = UxPrefs::default();
    change_mark(&run, &mut prefs, ReadAction::MarkRead);
    assert!(!is_unread(&run, &prefs));
    change_mark(&run, &mut prefs, ReadAction::MarkUnread);
    assert!(is_unread(&run, &prefs));
    change_mark(&run, &mut prefs, ReadAction::Archive);
    assert!(is_archived(&run, &prefs));
    change_mark(&run, &mut prefs, ReadAction::Unarchive);
    assert!(!is_archived(&run, &prefs)); assert!(!is_unread(&run, &prefs));
    change_mark(&run, &mut prefs, ReadAction::Archive);
    run.needs.push(Need { kind: NeedKind::Blocked, title: "new".into(), note: "".into() });
    assert!(is_unread(&run, &prefs)); assert!(!is_archived(&run, &prefs));
    change_mark(&run, &mut prefs, ReadAction::MarkRead);
    assert!(!is_unread(&run, &prefs)); assert!(!is_archived(&run, &prefs));
}

#[test]
fn key_is_stable_fnv_and_order_independent() {
    let mut run = sample_run();
    run.status = crate::desktop::model::RunStatus::Active;
    run.needs.clear(); run.units.clear(); run.report = None;
    assert_eq!(attention_key(&run), 0xeb649500db04c2b5);
    run.needs.push(Need { kind: NeedKind::Blocked, title: "B".into(), note: "".into() });
    run.needs.push(Need { kind: NeedKind::Waiting, title: "A".into(), note: "".into() });
    let key = attention_key(&run);
    run.needs.reverse();
    assert_eq!(attention_key(&run), key);
}

#[test]
fn new_failure_resurfaces_both_read_and_archived_runs() {
    let mut run = sample_run(); let mut read = UxPrefs::default(); let mut archived = UxPrefs::default();
    change_mark(&run, &mut read, ReadAction::MarkRead);
    change_mark(&run, &mut archived, ReadAction::Archive);
    run.units[0].state = UnitState::Failed;
    assert!(is_unread(&run, &read)); assert!(is_unread(&run, &archived));
    assert!(!is_archived(&run, &archived));
}
