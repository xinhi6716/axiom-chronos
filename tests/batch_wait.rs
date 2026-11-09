mod common;
use axiom::{
    goal::{self, sleep_unless_stopped, wait_for_batch, Batch, BatchEnd, BATCH_POLL},
    hook,
};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn batch(dir: &Path, pid: u32) -> Batch {
    Batch {
        progress_file: dir.join("progress.txt").to_string_lossy().into_owned(),
        marker_file: dir.join("done.flag").to_string_lossy().into_owned(),
        pid,
    }
}

fn sleeping_child() -> Child {
    Command::new(common::python())
        .args(["-I", "-c", "import time; time.sleep(60)"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct Waited {
    end: Option<BatchEnd>,
    sleeps: Vec<Duration>,
    progress: Vec<String>,
}

/// The world changes only inside the sleeper, so a loop that stops sleeping never sees the change;
/// the watchdog then ends the wait and the test fails instead of hanging.
fn wait(batch: &Batch, mut step: impl FnMut(usize, &AtomicBool)) -> Waited {
    let stop = Arc::new(AtomicBool::new(false));
    let watchdog = Arc::clone(&stop);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        watchdog.store(true, Ordering::SeqCst);
    });
    let (mut sleeps, mut progress) = (Vec::new(), Vec::new());
    let end = wait_for_batch(
        batch,
        &stop,
        &mut |pause| {
            sleeps.push(pause);
            assert!(sleeps.len() < 50, "the wait never ended");
            step(sleeps.len(), &stop);
        },
        &mut |text| progress.push(text.to_owned()),
    );
    Waited {
        end,
        sleeps,
        progress,
    }
}

#[test]
fn marker_ends_the_wait_and_progress_prints_only_on_change() {
    let dir = common::temp();
    let b = batch(dir.path(), std::process::id());
    fs::write(&b.progress_file, "0/3\n").unwrap();
    let waited = wait(&b, |n, _| match n {
        1 | 3 => fs::write(&b.progress_file, format!("{n}/3\n")).unwrap(),
        4 => fs::write(&b.marker_file, "").unwrap(),
        _ => {}
    });
    assert_eq!(waited.end, Some(BatchEnd::Finished));
    assert_eq!(waited.progress, ["0/3", "1/3", "3/3"]);
    assert_eq!(waited.sleeps.len(), 4);
}

#[test]
fn progress_written_with_the_marker_is_still_shown() {
    let dir = common::temp();
    let b = batch(dir.path(), std::process::id());
    let waited = wait(&b, |n, _| {
        if n == 2 {
            fs::write(&b.progress_file, "all done").unwrap();
            fs::write(&b.marker_file, "").unwrap();
        }
    });
    assert_eq!(waited.end, Some(BatchEnd::Finished));
    assert_eq!(waited.progress, ["all done"]);
}

#[test]
fn process_death_without_a_marker_is_reported() {
    let dir = common::temp();
    let mut child = sleeping_child();
    let b = batch(dir.path(), child.id());
    let waited = wait(&b, |n, _| {
        if n == 2 {
            reap(&mut child);
        }
    });
    reap(&mut child);
    assert_eq!(waited.end, Some(BatchEnd::Died));
    assert_eq!(waited.sleeps.len(), 2);
}

#[test]
fn a_marker_outranks_a_dead_process() {
    let dir = common::temp();
    let mut child = sleeping_child();
    reap(&mut child);
    let b = batch(dir.path(), child.id());
    fs::write(&b.marker_file, "").unwrap();
    let waited = wait(&b, |_, _| panic!("no sleep is needed"));
    assert_eq!(waited.end, Some(BatchEnd::Finished));
}

#[test]
fn stop_flag_ends_the_wait_with_the_batch_undecided() {
    let dir = common::temp();
    let b = batch(dir.path(), std::process::id());
    let waited = wait(&b, |n, stop| {
        if n == 3 {
            stop.store(true, Ordering::SeqCst);
        }
    });
    assert_eq!(waited.end, None);
    assert_eq!(waited.sleeps.len(), 3);
    assert!(!Path::new(&b.marker_file).exists());
}

#[test]
fn every_check_is_followed_by_a_low_frequency_sleep() {
    assert!(BATCH_POLL >= Duration::from_secs(1), "{BATCH_POLL:?}");
    let dir = common::temp();
    let b = batch(dir.path(), std::process::id());
    let waited = wait(&b, |n, _| {
        if n == 6 {
            fs::write(&b.marker_file, "").unwrap();
        }
    });
    assert_eq!(waited.end, Some(BatchEnd::Finished));
    assert_eq!(waited.sleeps, vec![BATCH_POLL; 6]);
}

#[test]
fn progress_shows_small_json_whole_and_otherwise_the_last_line() {
    let dir = common::temp();
    let b = batch(dir.path(), std::process::id());
    let mut long_log = "x\n".repeat(5000);
    long_log.push_str("latest line\n\n");
    fs::write(&b.marker_file, "").unwrap();
    for (content, expected) in [
        ("{\n  \"done\": 7,\n  \"total\": 10\n}\n", Some("{\"done\":7,\"total\":10}")),
        ("first\nsecond\n\n", Some("second")),
        (long_log.as_str(), Some("latest line")),
        ("  \n\n", None),
    ] {
        fs::write(&b.progress_file, content).unwrap();
        let waited = wait(&b, |_, _| panic!("no sleep is needed"));
        assert_eq!(
            waited.progress,
            expected.map(str::to_owned).into_iter().collect::<Vec<_>>()
        );
    }
    fs::write(&b.progress_file, "y".repeat(1000)).unwrap();
    let waited = wait(&b, |_, _| panic!("no sleep is needed"));
    assert_eq!(waited.progress[0].chars().count(), 200);
    fs::remove_file(&b.progress_file).unwrap();
    assert!(wait(&b, |_, _| panic!("no sleep is needed"))
        .progress
        .is_empty());
}

#[test]
fn sleeper_returns_early_once_stopped() {
    let stop = Arc::new(AtomicBool::new(false));
    let raise = Arc::clone(&stop);
    let raiser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        raise.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    sleep_unless_stopped(&stop, Duration::from_secs(30));
    assert!(start.elapsed() < Duration::from_secs(5));
    raiser.join().unwrap();
    let start = Instant::now();
    sleep_unless_stopped(&stop, Duration::from_secs(30));
    assert!(start.elapsed() < Duration::from_secs(1));
    stop.store(false, Ordering::SeqCst);
    let start = Instant::now();
    sleep_unless_stopped(&stop, Duration::from_millis(250));
    assert!(start.elapsed() >= Duration::from_millis(250));
}

#[test]
fn old_goal_json_reads_and_stays_without_batch_fields() {
    let dir = common::temp();
    let home = dir.path();
    fs::create_dir(home.join("goals")).unwrap();
    fs::write(
        home.join("goals/s.json"),
        r#"{"wish":"w","cwd":null,"status":"waiting","amendments":[],"note":"batch; p; m","extra":1}"#,
    )
    .unwrap();
    let g = goal::load(home, "s").unwrap().unwrap();
    assert_eq!(g.status, "waiting");
    assert!(g.batch.is_none() && g.waited.is_none());
    goal::mark_asked(home, "s").unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("goals/s.json")).unwrap()).unwrap();
    assert_eq!(raw["extra"], 1);
    assert_eq!(raw["note"], "batch; p; m");
    for key in ["batch", "waited"] {
        assert!(raw.get(key).is_none(), "unexpected {key}");
    }
    let reopened = goal::reopen(home, "s").unwrap();
    assert_eq!(
        reopened.waited,
        Some(goal::Waited {
            note: "batch; p; m".into(),
            batch: None,
            end: None
        })
    );
    assert!(hook::stop_reason("s", &reopened).contains("batch; p; m"));
}

fn on_disk(home: &Path) -> goal::Goal {
    goal::load(home, "s").unwrap().unwrap()
}

#[test]
fn a_resumed_waiting_goal_is_told_what_it_waited_on_until_the_wish_moves_on() {
    let dir = common::temp();
    let home = dir.path();
    let b = batch(home, 4321);
    goal::open(home, "s", "wish", None).unwrap();
    goal::set_status_with_batch(home, "s", "waiting", Some("screening 7800 papers"), Some(b.clone()))
        .unwrap();
    let expected = goal::Waited {
        note: "screening 7800 papers".into(),
        batch: Some(b.clone()),
        end: None,
    };
    let reopened = goal::reopen(home, "s").unwrap();
    assert_eq!(reopened.waited.as_ref(), Some(&expected));
    assert_eq!(on_disk(home).waited.as_ref(), Some(&expected));
    let reason = hook::stop_reason("s", &reopened);
    for fact in [
        "screening 7800 papers",
        b.progress_file.as_str(),
        b.marker_file.as_str(),
        "4321",
        "may still be running",
        "do not start it again",
    ] {
        assert!(reason.contains(fact), "{fact}: {reason}");
    }

    goal::interrupt(home, "s", "claude crashed").unwrap().unwrap();
    assert_eq!(on_disk(home).status, "blocked");
    assert_eq!(on_disk(home).waited.as_ref(), Some(&expected));
    let again = goal::reopen(home, "s").unwrap();
    assert_eq!(again.waited.as_ref(), Some(&expected));
    assert!(hook::stop_reason("s", &again).contains("screening 7800 papers"));

    goal::set_status(home, "s", "blocked", Some("needs a key")).unwrap();
    assert_eq!(on_disk(home).waited.as_ref(), Some(&expected));
    goal::reopen(home, "s").unwrap();
    goal::set_status(home, "s", "waiting", Some("second batch")).unwrap();
    assert!(on_disk(home).waited.is_none());
    goal::reopen(home, "s").unwrap();
    assert_eq!(on_disk(home).waited.unwrap().note, "second batch");
    goal::set_status(home, "s", "done", None).unwrap();
    assert!(on_disk(home).waited.is_none());
}

#[test]
fn a_waiting_goal_closed_as_blocked_keeps_what_it_waited_on() {
    let dir = common::temp();
    let home = dir.path();
    let b = batch(home, 4321);
    goal::open(home, "s", "wish", None).unwrap();
    goal::set_status_with_batch(home, "s", "waiting", Some("the batch"), Some(b.clone())).unwrap();
    goal::set_status(home, "s", "blocked", Some("needs a key")).unwrap();
    let g = on_disk(home);
    assert_eq!(g.note.as_deref(), Some("needs a key"));
    assert!(g.batch.is_none());
    let waited = g.waited.unwrap();
    assert_eq!(waited.note, "the batch");
    assert_eq!(waited.batch, Some(b));
    assert_eq!(waited.end, None);
}

#[test]
fn an_interjection_on_a_waiting_goal_keeps_what_it_waited_on() {
    let dir = common::temp();
    let home = dir.path();
    let b = batch(home, 4321);
    let waiting = || {
        goal::open(home, "s", "wish", None).unwrap();
        goal::set_status_with_batch(home, "s", "waiting", Some("the batch"), Some(b.clone()))
            .unwrap();
        goal::interject(home, "s", "a question").unwrap().unwrap();
        assert_eq!(on_disk(home).waited.unwrap().note, "the batch");
    };
    waiting();
    let answered = goal::classify(home, "s", "answer").unwrap();
    assert_eq!(answered.status, "waiting");
    assert_eq!(answered.note.as_deref(), Some("the batch"));
    assert_eq!(answered.batch.as_ref(), Some(&b));
    assert!(answered.waited.is_none());

    waiting();
    let amended = goal::classify(home, "s", "amend").unwrap();
    assert_eq!(amended.status, "active");
    assert!(amended.batch.is_none());
    assert_eq!(amended.waited.unwrap().batch.as_ref(), Some(&b));

    waiting();
    goal::interrupt(home, "s", "claude crashed").unwrap().unwrap();
    let g = on_disk(home);
    assert_eq!(g.status, "blocked");
    assert_eq!(g.waited.unwrap().batch.as_ref(), Some(&b));

    waiting();
    goal::interject(home, "s", "another question").unwrap().unwrap();
    assert_eq!(on_disk(home).waited.unwrap().note, "the batch");
    assert_eq!(goal::classify(home, "s", "stop").unwrap().status, "blocked");
    assert_eq!(on_disk(home).waited.unwrap().note, "the batch");
}

#[test]
fn batch_fields_are_saved_next_to_the_note_and_cleared_by_later_states() {
    let dir = common::temp();
    let home = dir.path();
    let b = batch(home, 77);
    goal::open(home, "s", "wish", None).unwrap();
    let result =
        goal::set_status_with_batch(home, "s", "waiting", Some("note"), Some(b.clone())).unwrap();
    assert_eq!(result["goal"]["batch"]["pid"], 77);
    assert_eq!(result["goal"]["batch"]["marker_file"], b.marker_file.as_str());
    assert_eq!(result["goal"]["batch"]["progress_file"], b.progress_file.as_str());
    assert_eq!(result["goal"]["note"], "note");
    let g = goal::load(home, "s").unwrap().unwrap();
    assert_eq!(g.batch.as_ref(), Some(&b));
    goal::set_status(home, "s", "waiting", Some("no batch now")).unwrap();
    assert!(goal::load(home, "s").unwrap().unwrap().batch.is_none());
    goal::set_status_with_batch(home, "s", "waiting", Some("note"), Some(b.clone())).unwrap();
    assert!(goal::reopen(home, "s").unwrap().batch.is_none());
}

#[test]
fn batch_fields_are_validated() {
    let dir = common::temp();
    let home = dir.path();
    goal::open(home, "s", "wish", None).unwrap();
    let good = batch(home, 77);
    let cases = [
        Batch {
            progress_file: "relative/progress".into(),
            ..good.clone()
        },
        Batch {
            marker_file: "done.flag".into(),
            ..good.clone()
        },
        Batch {
            pid: 0,
            ..good.clone()
        },
    ];
    for bad in cases {
        let error =
            goal::set_status_with_batch(home, "s", "waiting", Some("n"), Some(bad)).unwrap_err();
        assert_eq!(error.kind, "invalid_argument");
        assert_eq!(goal::load(home, "s").unwrap().unwrap().status, "active");
    }
    for status in ["blocked", "done"] {
        assert_eq!(
            goal::set_status_with_batch(home, "s", status, Some("n"), Some(good.clone()))
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
    }
    assert_eq!(
        goal::set_status_with_batch(home, "s", "waiting", None, Some(good))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}

#[test]
fn a_recorded_end_reopens_the_goal_once_and_reaches_the_continue_prompt() {
    let dir = common::temp();
    let home = dir.path();
    let b = batch(home, 77);
    for (end, expected, excluded) in [
        (BatchEnd::Finished, "finished", "failure"),
        (BatchEnd::Died, "died without writing", "finished"),
    ] {
        goal::open(home, "s", "wish", None).unwrap();
        goal::set_status_with_batch(home, "s", "waiting", Some("the note"), Some(b.clone()))
            .unwrap();
        assert!(goal::batch_ended(home, "s", &b, end).unwrap());
        let g = goal::load(home, "s").unwrap().unwrap();
        assert_eq!(g.status, "active");
        assert!(g.batch.is_none() && g.note.is_none());
        assert_eq!(
            g.waited,
            Some(goal::Waited {
                note: "the note".into(),
                batch: Some(b.clone()),
                end: Some(end)
            })
        );
        let reason = hook::stop_reason("s", &g);
        assert!(reason.contains(expected), "{reason}");
        assert!(!reason.contains(excluded), "{reason}");
        for fact in ["the note", b.marker_file.as_str(), b.progress_file.as_str(), "77"] {
            assert!(reason.contains(fact), "{fact}: {reason}");
        }
        assert!(!goal::batch_ended(home, "s", &b, end).unwrap());
        let reopened = goal::reopen(home, "s").unwrap();
        assert_eq!(reopened.waited, g.waited);
        goal::interrupt(home, "s", "crash").unwrap().unwrap();
        assert_eq!(goal::load(home, "s").unwrap().unwrap().waited, g.waited);
    }
}

#[test]
fn only_the_batch_the_goal_waits_on_can_end_it() {
    let dir = common::temp();
    let home = dir.path();
    goal::open(home, "s", "wish", None).unwrap();
    let first = batch(home, 77);
    let second = Batch {
        pid: 78,
        ..first.clone()
    };
    goal::set_status_with_batch(home, "s", "waiting", Some("n"), Some(second)).unwrap();
    assert!(!goal::batch_ended(home, "s", &first, BatchEnd::Finished).unwrap());
    let g = goal::load(home, "s").unwrap().unwrap();
    assert_eq!(g.status, "waiting");
    assert!(g.waited.is_none());
    assert_eq!(goal::batch_ended(home, "none", &first, BatchEnd::Died).unwrap_err().kind, "not_found");
}
