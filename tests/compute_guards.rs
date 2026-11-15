use axiom::{
    compute::{
        consent::{build_manifest, render, ConsentStore},
        limits::{CapHit, CapWatch, CuMeter, FakeMeter},
        session::{argv, pending, recover, ColabMeter, ColabSession, FakeColab},
        CliOutput, ColabCli, ComputeRequest, JobLimits,
    },
    seams::{FakeApproval, FakeClock},
};
use std::{fs, path::Path, time::Duration};
use tempfile::TempDir;

fn limits() -> JobLimits {
    JobLimits {
        wall_clock_s: 10,
        compute_units: 2.0,
    }
}
fn request(dir: &Path) -> ComputeRequest {
    ComputeRequest {
        job_id: "job".into(),
        project: "project".into(),
        project_dir: dir.into(),
        argv: vec!["python".into(), "task.py".into()],
        inputs: vec!["data".into()],
        outputs: vec!["result".into()],
        gpu: "T4".into(),
        vram_gb: 16.0,
        est_minutes: 1.0,
        limits: limits(),
    }
}
fn ran(cli: &FakeColab, name: &str) -> bool {
    cli.calls()
        .iter()
        .any(|a| a.get(2).map(String::as_str) == Some(name))
}

#[test]
fn independent_caps_and_priority() {
    assert!(limits().validate().is_ok());
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(JobLimits {
            wall_clock_s: 10,
            compute_units: invalid
        }
        .validate()
        .is_err());
    }
    assert!(JobLimits {
        wall_clock_s: 0,
        compute_units: 1.0
    }
    .validate()
    .is_err());
    let clock = FakeClock::new(0);
    let meter = FakeMeter::new(0.0);
    let mut watch = CapWatch::new(&clock, &limits(), Some(&meter));
    clock.advance(9000);
    meter.set(1.9);
    assert_eq!(watch.check().unwrap(), None);
    assert_eq!(watch.last_cu(), 1.9);
    meter.set(2.0);
    assert_eq!(watch.check().unwrap(), Some(CapHit::ComputeUnits));
    clock.advance(1000);
    assert_eq!(watch.check().unwrap(), Some(CapHit::WallClock));
    assert_eq!(watch.last_cu(), 2.0);
    let clock = FakeClock::new(0);
    let mut wall = CapWatch::new(&clock, &limits(), None);
    clock.advance(9999);
    assert_eq!(wall.check().unwrap(), None);
    clock.advance(1);
    assert_eq!(wall.elapsed_s(), 10.0);
    assert_eq!(wall.check().unwrap(), Some(CapHit::WallClock));
}

#[test]
fn manifest_consent_and_upload_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("data")).unwrap();
    fs::write(root.join("data/b"), b"second").unwrap();
    fs::write(root.join("data/a"), b"first").unwrap();
    let inputs = vec!["data".into()];
    let first = build_manifest("p", root, &inputs).unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>(),
        ["data/a", "data/b"]
    );
    assert_eq!(
        render(&first),
        format!(
            "data/a  5 bytes  {}\ndata/b  6 bytes  {}\nTotal: 2 files, 11 bytes",
            &first.entries[0].sha256[..12],
            &first.entries[1].sha256[..12]
        )
    );
    assert_eq!(first, build_manifest("p", root, &inputs).unwrap());
    let store = ConsentStore::open(root).unwrap();
    let cli = FakeColab::default();
    let mut req = request(root);
    req.project = "p".into();
    let mut session = ColabSession::allocate(&cli, root, &req).unwrap();
    assert_eq!(
        session.upload(&store, &first).unwrap_err().kind,
        "consent_missing"
    );
    assert_eq!(cli.uploads(), 0);
    let denied = FakeApproval::denying();
    assert!(!store.request(&first, &denied).unwrap());
    assert!(!store.is_granted(&first).unwrap());
    let approved = FakeApproval::approving();
    assert!(store.request(&first, &approved).unwrap());
    let asked = approved.asked();
    assert_eq!(asked[0].subject, "compute:p");
    assert_eq!(asked[0].action, "upload");
    assert_eq!(asked[0].digest, first.digest);
    assert_eq!(asked[0].summary, render(&first));
    assert!(store.is_granted(&first).unwrap());
    let reopened = ConsentStore::open(root).unwrap();
    assert!(reopened.is_granted(&first).unwrap());
    assert_eq!(session.upload(&reopened, &first).unwrap(), 2);
    assert_eq!(cli.uploads(), 2);
    fs::write(root.join("data/a"), b"changed").unwrap();
    let changed = build_manifest("p", root, &inputs).unwrap();
    assert!(!store.is_granted(&changed).unwrap());
    assert_eq!(
        session.upload(&store, &changed).unwrap_err().kind,
        "consent_missing"
    );
    assert_eq!(
        session.upload(&store, &first).unwrap_err().kind,
        "consent_missing"
    );
    assert_eq!(cli.uploads(), 2);
    let another = build_manifest("other", root, &inputs).unwrap();
    assert!(!store.is_granted(&another).unwrap());
    store.revoke("p").unwrap();
    assert!(!reopened.is_granted(&first).unwrap());
}
#[test]
fn paths_are_confined() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("ok"), b"ok").unwrap();
    assert!(build_manifest("p", root.path(), &["ok".into()]).is_ok());
    for path in ["../ok", "/tmp/a", "a/../ok", "C:\\outside", "..\\outside"] {
        assert_eq!(
            build_manifest("p", root.path(), &[path.into()])
                .unwrap_err()
                .kind,
            "path_escape"
        );
    }
}
#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), b"x").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert_eq!(
        build_manifest("p", root.path(), &["escape/secret".into()])
            .unwrap_err()
            .kind,
        "path_escape"
    );
}
#[test]
fn session_close_drop_and_recovery() {
    let root = TempDir::new().unwrap();
    let req = request(root.path());
    let cli = FakeColab::new("session-1");
    {
        let s = ColabSession::allocate(&cli, root.path(), &req).unwrap();
        assert_eq!(s.id(), "session-1");
        assert_eq!(pending(root.path()).unwrap().len(), 1);
    }
    assert!(ran(&cli, "close"));
    assert!(pending(root.path()).unwrap().is_empty());
    cli.fail("close", true);
    {
        let mut s = ColabSession::allocate(&cli, root.path(), &req).unwrap();
        assert!(!s.close());
        assert_eq!(pending(root.path()).unwrap().len(), 1);
    }
    assert_eq!(
        recover(root.path(), &cli).unwrap().still_pending,
        ["session-1"]
    );
    assert_eq!(pending(root.path()).unwrap().len(), 1);
    cli.fail("close", false);
    let report = recover(root.path(), &cli).unwrap();
    assert_eq!(report.closed, ["session-1"]);
    assert!(report.still_pending.is_empty());
    assert!(pending(root.path()).unwrap().is_empty());
}
#[test]
fn create_upload_exec_download_failure_paths_close() {
    let root = TempDir::new().unwrap();
    let req = request(root.path());
    let cli = FakeColab::default();
    cli.fail("create", true);
    assert!(ColabSession::allocate(&cli, root.path(), &req).is_err());
    assert!(!ran(&cli, "close"));
    assert!(pending(root.path()).unwrap().is_empty());
    let cli = FakeColab::new("known");
    cli.fail_create_after_id(true);
    cli.fail("close", true);
    assert!(ColabSession::allocate(&cli, root.path(), &req).is_err());
    assert!(ran(&cli, "close"));
    assert_eq!(pending(root.path()).unwrap()[0].session_id, "known");
    cli.fail("close", false);
    assert_eq!(recover(root.path(), &cli).unwrap().closed, ["known"]);
    fs::create_dir(root.path().join("data")).unwrap();
    fs::write(root.path().join("data/a"), b"input").unwrap();
    let manifest = build_manifest("project", root.path(), &req.inputs).unwrap();
    let store = ConsentStore::open(root.path()).unwrap();
    assert!(store
        .request(&manifest, &FakeApproval::approving())
        .unwrap());
    for step in ["upload", "exec", "download"] {
        let cli = FakeColab::new(step);
        cli.fail(step, true);
        cli.fail("close", true);
        {
            let mut s = ColabSession::allocate(&cli, root.path(), &req).unwrap();
            if step == "upload" {
                assert!(s.upload(&store, &manifest).is_err());
                assert_eq!(cli.uploads(), 1);
            } else if step == "exec" {
                let mut job = cli
                    .start(&argv::exec(s.id(), &req.argv), root.path())
                    .unwrap();
                assert_ne!(job.poll().unwrap().unwrap().code, 0);
            } else {
                let args = argv::download(s.id(), "result");
                assert_ne!(
                    cli.run(&args, root.path(), Duration::from_secs(1))
                        .unwrap()
                        .code,
                    0
                );
            }
        }
        assert!(ran(&cli, "close"));
        assert_eq!(pending(root.path()).unwrap()[0].session_id, step);
        cli.fail("close", false);
        assert_eq!(recover(root.path(), &cli).unwrap().closed, [step]);
        assert!(pending(root.path()).unwrap().is_empty());
    }
}
#[test]
fn argv_and_usage() {
    let root = TempDir::new().unwrap();
    let cli = FakeColab::default();
    cli.set_cu_readings(vec![0.5, 2.0]);
    let meter = ColabMeter::new(&cli, "id", root.path());
    assert_eq!(meter.used().unwrap(), 0.5);
    assert_eq!(meter.used().unwrap(), 2.0);
    cli.fail("usage", true);
    assert!(meter.used().is_err());
    let commands = [
        argv::create("T4"),
        argv::upload("id", "data/a"),
        argv::exec("id", &["python".into(), "run.py".into()]),
        argv::usage("id"),
        argv::download("id", "out/a"),
        argv::close("id"),
    ];
    for command in commands {
        for item in command {
            assert!(!Path::new(&item).is_absolute(), "{item}");
        }
    }
    assert_eq!(
        argv::exec("id", &["/usr/bin/python".into(), "k:v".into()])[6..],
        ["/usr/bin/python", "k:v"]
    );
    assert_eq!(argv::upload("id", "/absolute")[6], "/absolute");
    let _ = CliOutput {
        code: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
}

#[test]
fn upload_rejects_unsafe_manifest_paths_before_any_upload() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("ok"), b"ok").unwrap();
    let mut manifest = build_manifest("project", root.path(), &["ok".into()]).unwrap();
    let store = ConsentStore::open(root.path()).unwrap();
    let cli = FakeColab::default();
    let mut session = ColabSession::allocate(&cli, root.path(), &request(root.path())).unwrap();
    for path in ["/absolute", "../outside", "C:\\outside"] {
        manifest.entries[0].path = path.into();
        assert!(store
            .request(&manifest, &FakeApproval::approving())
            .unwrap());
        assert_eq!(
            session.upload(&store, &manifest).unwrap_err().kind,
            "path_escape"
        );
        assert_eq!(cli.uploads(), 0);
    }
}

#[test]
fn invalid_meter_readings_are_errors() {
    let root = TempDir::new().unwrap();
    let clock = FakeClock::new(0);
    let fake = FakeMeter::new(1.0);
    let mut watch = CapWatch::new(&clock, &limits(), Some(&fake));
    assert_eq!(watch.check().unwrap(), None);
    for value in [-1.0, f64::NAN] {
        fake.set(value);
        assert!(watch.check().is_err());
        assert_eq!(watch.last_cu(), 1.0);
    }
    let cli = FakeColab::default();
    cli.set_cu_readings(vec![1.0, -1.0, f64::NAN]);
    let meter = ColabMeter::new(&cli, "id", root.path());
    assert_eq!(meter.used().unwrap(), 1.0);
    assert!(meter.used().is_err());
    assert!(meter.used().is_err());
}

#[test]
fn fake_job_duration_and_kill() {
    let root = TempDir::new().unwrap();
    let cli = FakeColab::default();
    let args = argv::exec("id", &["python".into()]);
    cli.set_exec_duration(Duration::from_millis(40));
    let mut job = cli.start(&args, root.path()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(job.poll().unwrap().unwrap().code, 0);
    cli.set_exec_duration(Duration::from_secs(60));
    let mut killed = cli.start(&args, root.path()).unwrap();
    assert!(killed.poll().unwrap().is_none());
    killed.kill();
    assert_ne!(killed.poll().unwrap().unwrap().code, 0);
}
