use super::{paper::SessionPaper, wiring::production};
use crate::{
    base::Failure,
    desktop::ux::seams::{FakePaper, PaperCall, PaperService, Preview},
    paper::{
        self, journals::matching, service::PaperStatus, Chooser, JournalChoice, JournalRecord,
        Requirements, Shortlist, Step,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const SESSION: &str = "paper-accept";
struct Rig {
    home: tempfile::TempDir,
    slots: tempfile::TempDir,
    store: PathBuf,
    paper: SessionPaper,
}
impl Rig {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let slots = tempfile::tempdir().unwrap();
        let goal = crate::goal::open(
            home.path(),
            SESSION,
            "wish",
            Some(home.path().to_str().unwrap()),
        )
        .unwrap();
        let store = crate::orch::store_for_goal(home.path(), &goal).unwrap();
        let paper = SessionPaper::new(home.path());
        Self {
            home,
            slots,
            store,
            paper,
        }
    }
    fn seed(&self) -> Shortlist {
        let list = shortlist();
        let dir = paper::paper_dir(&self.store);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("shortlist.json"),
            serde_json::to_vec(&list).unwrap(),
        )
        .unwrap();
        paper::readiness::record_gate(&self.store, Step::Shortlist, &list.hash, true, None)
            .unwrap();
        list
    }
    fn choose(&self, list: &Shortlist) {
        paper::readiness::record_journal(
            &self.store,
            &JournalChoice {
                journal_id: list.ranked[0].journal.id.clone(),
                shortlist_hash: list.hash.clone(),
                chosen_by: Chooser::Person,
                at: "2025-01-01".into(),
            },
        )
        .unwrap();
    }
    fn pdf(&self, fixture: &str) -> PathBuf {
        let path = paper::readiness::pdf_path(&self.store);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/paper")
                .join(fixture),
            &path,
        )
        .unwrap();
        path
    }
}
fn requirements() -> Requirements {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/paper/journals/requirements.json"
    ))
    .unwrap()
}
fn shortlist() -> Shortlist {
    let journals: Vec<JournalRecord> = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/paper/journals/journals.json"
    ))
    .unwrap();
    let list = matching::rank(&requirements(), &journals, &[], "2025-01-01T00:00:00Z");
    assert!(!list.ranked.is_empty());
    list
}
fn empty_status() -> PaperStatus {
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

#[test]
fn fake_scripts_status_shortlist_failures_and_calls() {
    let fake = FakePaper::new();
    let list = shortlist();
    let req = requirements();
    fake.set_status(SESSION, empty_status());
    fake.set_shortlist(SESSION, list.clone());
    assert!(!fake.status(SESSION).unwrap().running);
    fake.request_shortlist(SESSION, req.clone(), Some("jcr.csv".into()))
        .unwrap();
    assert_eq!(fake.status(SESSION).unwrap().shortlist.unwrap(), list);
    fake.fail("cancel", Failure::new("scripted", "failure"));
    assert_eq!(fake.cancel(SESSION).unwrap_err().kind, "scripted");
    fake.clear_failure("cancel");
    fake.cancel(SESSION).unwrap();
    assert_eq!(
        fake.calls(),
        vec![
            PaperCall::Status(SESSION.into()),
            PaperCall::RequestShortlist(SESSION.into(), req, Some("jcr.csv".into())),
            PaperCall::Status(SESSION.into()),
            PaperCall::Cancel(SESSION.into()),
            PaperCall::Cancel(SESSION.into())
        ]
    );
}
#[test]
fn status_empty_and_unknown_session() {
    let r = Rig::new();
    let status = r.paper.status(SESSION).unwrap();
    assert!(!status.running);
    assert!(status.shortlist.is_none());
    assert_eq!(
        r.paper.status("missing-session").unwrap_err().kind,
        "not_found"
    );
}
#[test]
fn choice_rejects_stale_or_unknown_without_writing() {
    let r = Rig::new();
    let list = r.seed();
    let path = paper::readiness::journal_path(&r.store);
    assert_eq!(
        r.paper
            .choose_journal(SESSION, &list.ranked[0].journal.id, "stale")
            .unwrap_err()
            .kind,
        "stale_shortlist"
    );
    assert!(!path.exists());
    assert!(r
        .paper
        .choose_journal(SESSION, "not-a-journal", &list.hash)
        .is_err());
    assert!(!path.exists());
}
#[test]
fn start_rechecks_choice_and_estimate() {
    let r = Rig::new();
    let list = r.seed();
    assert_eq!(
        r.paper.start(SESSION, "wrong").unwrap_err().kind,
        "no_journal"
    );
    r.choose(&list);
    assert_eq!(
        r.paper.start(SESSION, "wrong").unwrap_err().kind,
        "estimate_changed"
    );
}
#[test]
fn second_write_is_busy() {
    let _guard = OPEN_ENV.lock().unwrap_or_else(|error| error.into_inner());
    let old = std::env::var_os("AXIOM_CURL_CMD");
    struct Cleanup {
        old: Option<std::ffi::OsString>,
        release: PathBuf,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::write(&self.release, b"release");
            match self.old.take() {
                Some(v) => std::env::set_var("AXIOM_CURL_CMD", v),
                None => std::env::remove_var("AXIOM_CURL_CMD"),
            }
        }
    }
    let r = Rig::new();
    let ready = r.home.path().join("curl-ready");
    let release = r.home.path().join("curl-release");
    let _cleanup = Cleanup {
        old,
        release: release.clone(),
    };
    #[cfg(unix)]
    let argv = {
        let script = r.home.path().join("curl.sh");
        fs::write(
            &script,
            "touch \"$1\"\nwhile [ ! -f \"$2\" ]; do sleep 0.05; done\nexit 1\n",
        )
        .unwrap();
        vec![
            "/bin/sh".to_owned(),
            script.to_string_lossy().into_owned(),
            ready.to_string_lossy().into_owned(),
            release.to_string_lossy().into_owned(),
        ]
    };
    #[cfg(windows)]
    let argv = {
        let Some(shell) = crate::base::on_path("powershell.exe") else {
            eprintln!("skipping busy test: powershell.exe unavailable for blocking transport");
            return;
        };
        let script = r.home.path().join("curl.ps1");
        fs::write(
            &script,
            format!(
                "Set-Content -LiteralPath '{}' -Value ready\nwhile (!(Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 50 }}\nexit 1\n",
                ready.to_string_lossy().replace('\'', "''"),
                release.to_string_lossy().replace('\'', "''"),
            ),
        )
        .unwrap();
        vec![
            shell,
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.to_string_lossy().into_owned(),
        ]
    };
    #[cfg(not(any(unix, windows)))]
    {
        eprintln!("skipping busy test: no supported blocking transport");
        return;
    }
    #[cfg(any(unix, windows))]
    {
        std::env::set_var("AXIOM_CURL_CMD", serde_json::to_string(&argv).unwrap());
        r.paper
            .request_shortlist(SESSION, requirements(), None)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(ready.exists(), "shortlist did not reach blocking transport");
        assert_eq!(
            r.paper
                .request_shortlist(SESSION, requirements(), None)
                .unwrap_err()
                .kind,
            "busy"
        );
        fs::write(&release, b"release").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while r.paper.status(SESSION).unwrap().running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !r.paper.status(SESSION).unwrap().running,
            "worker did not stop"
        );
    }
}
#[test]
fn preview_missing_pdf() {
    let r = Rig::new();
    assert_eq!(r.paper.preview(SESSION, 1, 600).unwrap_err().kind, "no_pdf");
}
#[test]
fn preview_sample_and_bad_page() {
    let r = Rig::new();
    let pdf = r.pdf("pdf/sample.pdf");
    let result = r.paper.preview(SESSION, 1, 600).unwrap();
    #[cfg(windows)]
    match result {
        Preview::Ready(image) => {
            assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(image.page, 1);
            assert_eq!(image.pages, 1);
            assert!(image.width_px.abs_diff(600) <= 1);
        }
        other => panic!("expected rendered page: {other:?}"),
    }
    #[cfg(not(windows))]
    match result {
        Preview::Unavailable {
            reason,
            pdf: actual,
        } => {
            assert!(!reason.is_empty());
            assert_eq!(actual, pdf);
        }
        other => panic!("expected unavailable: {other:?}"),
    }
    #[cfg(windows)]
    let _ = pdf;
    assert_eq!(
        r.paper.preview(SESSION, 2, 600).unwrap_err().kind,
        "bad_page"
    );
}
#[test]
fn preview_corrupt_pdf_is_unavailable() {
    let r = Rig::new();
    let pdf = r.pdf("render/garbage.pdf");
    match r.paper.preview(SESSION, 1, 600).unwrap() {
        Preview::Unavailable {
            reason,
            pdf: actual,
        } => {
            assert!(!reason.is_empty());
            assert_eq!(actual, pdf);
        }
        Preview::Ready(image) => panic!("corrupt PDF rendered: {} bytes", image.png.len()),
    }
}
static OPEN_ENV: Mutex<()> = Mutex::new(());
struct RestoreEnv(Option<std::ffi::OsString>);
impl Drop for RestoreEnv {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("AXIOM_OPEN_CMD", v),
            None => std::env::remove_var("AXIOM_OPEN_CMD"),
        }
    }
}
#[test]
fn open_pdf_uses_configured_opener() {
    let _guard = OPEN_ENV.lock().unwrap_or_else(|error| error.into_inner());
    let _restore = RestoreEnv(std::env::var_os("AXIOM_OPEN_CMD"));
    let r = Rig::new();
    let record = r.home.path().join("open-argv.txt");
    #[cfg(unix)]
    let argv = {
        let script = r.home.path().join("recorder.sh");
        fs::write(&script, "printf '%s\\n' \"$2\" > \"$1\"\n").unwrap();
        vec![
            "/bin/sh".to_owned(),
            script.to_string_lossy().into_owned(),
            record.to_string_lossy().into_owned(),
        ]
    };
    #[cfg(windows)]
    let argv = {
        let Some(shell) = crate::base::on_path("powershell.exe") else {
            eprintln!("skipping opener recorder: powershell.exe unavailable");
            return;
        };
        let script = r.home.path().join("recorder.ps1");
        fs::write(
            &script,
            "param($record, $pdf)\nSet-Content -LiteralPath $record -Value $pdf\n",
        )
        .unwrap();
        vec![
            shell,
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.to_string_lossy().into_owned(),
            record.to_string_lossy().into_owned(),
        ]
    };
    #[cfg(not(any(unix, windows)))]
    {
        eprintln!("skipping opener recorder: no supported shell");
        return;
    }
    #[cfg(any(unix, windows))]
    {
        std::env::set_var("AXIOM_OPEN_CMD", serde_json::to_string(&argv).unwrap());
        assert_eq!(r.paper.open_pdf(SESSION).unwrap_err().kind, "no_pdf");
        let pdf = r.pdf("pdf/sample.pdf");
        r.paper.open_pdf(SESSION).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !record.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let recorded = fs::read_to_string(record).unwrap();
        assert!(recorded.contains(pdf.to_str().unwrap()), "{recorded:?}");
    }
}
#[test]
fn production_has_paper() {
    let r = Rig::new();
    assert!(production(r.home.path(), r.slots.path()).paper.is_some());
}
#[test]
fn production_sources_do_not_construct_fake_paper() {
    assert!(!include_str!("wiring.rs").contains("FakePaper"));
    assert!(!include_str!("paper.rs").contains("FakePaper"));
}
