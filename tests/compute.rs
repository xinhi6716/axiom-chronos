mod common;

use axiom::{
    base::Res,
    compute::{
        consent::{build_manifest, ConsentStore},
        routing::{route, run_job, Compute, FakeLocalGpu, LocalCapacity, LocalGpu, RouteReason},
        session::{pending, FakeColab},
        CliJob, CliOutput, ColabCli, ComputeRequest, JobEnd, JobLimits, Target,
    },
    seams::{Clock, FakeApproval, FakeClock},
    usage::extensions,
};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;

fn req(dir: &Path) -> ComputeRequest {
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(dir.join("data/a"), b"a").unwrap();
    fs::write(dir.join("data/b"), b"b").unwrap();
    ComputeRequest {
        job_id: "job".into(),
        project: "project".into(),
        project_dir: dir.into(),
        argv: vec![
            "python".into(),
            "task.py".into(),
            "hello world".into(),
            "$(literal)".into(),
        ],
        inputs: vec!["data".into()],
        outputs: vec!["result file".into()],
        gpu: "T4".into(),
        vram_gb: 16.0,
        est_minutes: 10.0,
        limits: JobLimits {
            wall_clock_s: 10,
            compute_units: 2.0,
        },
    }
}
fn fits() -> LocalCapacity {
    LocalCapacity {
        vram_gb: 16.0,
        max_minutes: 10.0,
    }
}
fn has(cli: &FakeColab, step: &str) -> bool {
    cli.calls()
        .iter()
        .any(|a| a.get(2).is_some_and(|x| x == step))
}
struct CountCli {
    fake: FakeColab,
    kills: Arc<AtomicUsize>,
}
impl CountCli {
    fn new() -> Self {
        Self {
            fake: FakeColab::default(),
            kills: Arc::new(AtomicUsize::new(0)),
        }
    }
}
struct CountJob {
    inner: Box<dyn CliJob>,
    kills: Arc<AtomicUsize>,
}
impl CliJob for CountJob {
    fn poll(&mut self) -> Res<Option<CliOutput>> {
        self.inner.poll()
    }
    fn kill(&mut self) {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.inner.kill();
    }
}
impl ColabCli for CountCli {
    fn run(&self, args: &[String], cwd: &Path, timeout: Duration) -> Res<CliOutput> {
        self.fake.run(args, cwd, timeout)
    }
    fn start(&self, args: &[String], cwd: &Path) -> Res<Box<dyn CliJob>> {
        Ok(Box::new(CountJob {
            inner: self.fake.start(args, cwd)?,
            kills: self.kills.clone(),
        }))
    }
}
struct TickClock {
    ms: AtomicI64,
    step: i64,
}
impl Clock for TickClock {
    fn now_ms(&self) -> i64 {
        self.ms.fetch_add(self.step, Ordering::SeqCst)
    }
}
fn ctx<'a>(
    home: &'a Path,
    local: &'a FakeLocalGpu,
    cli: &'a dyn ColabCli,
    approval: &'a FakeApproval,
    clock: &'a dyn Clock,
    cancel: &'a AtomicBool,
) -> Compute<'a> {
    Compute {
        home,
        local,
        colab: cli,
        approval,
        clock,
        cancel,
        poll: Duration::from_millis(1),
    }
}
#[test]
fn routes_run_and_log_usage() {
    for (capacity, reason, target) in [
        (Some(fits()), RouteReason::LocalFits, Target::Local),
        (
            Some(LocalCapacity {
                vram_gb: 15.0,
                max_minutes: 1.0,
            }),
            RouteReason::VramTooSmall,
            Target::Colab,
        ),
        (
            Some(LocalCapacity {
                vram_gb: 16.0,
                max_minutes: 9.0,
            }),
            RouteReason::TimeTooShort,
            Target::Colab,
        ),
        (None, RouteReason::LocalUnavailable, Target::Colab),
    ] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        let local = FakeLocalGpu::new(capacity);
        let cli = FakeColab::default();
        let approval = FakeApproval::approving();
        let clock = FakeClock::new(1234);
        let cancel = AtomicBool::new(false);
        assert_eq!(route(&request, capacity.as_ref()).reason, reason);
        let result = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert_eq!(result.target, target);
        assert_eq!(result.end, JobEnd::Completed);
        assert_eq!(local.calls().len(), usize::from(target == Target::Local));
        assert_eq!(has(&cli, "create"), target == Target::Colab);
        let (records, skipped) = extensions::read(temp.path()).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(
            r.provider,
            if target == Target::Local {
                "gpu_local"
            } else {
                "colab"
            }
        );
        assert_eq!(r.project.as_deref(), Some("project"));
        assert_eq!(r.job.as_deref(), Some("job"));
        assert_eq!(r.ts_ms, 1234);
        assert_eq!(r.gpu_seconds, result.gpu_seconds);
        assert_eq!(r.compute_units, result.compute_units);
        assert_eq!(r.usd, None);
    }
}
#[test]
fn assumed_argv_fixture() {
    let temp = TempDir::new().unwrap();
    let request = req(temp.path());
    let local = FakeLocalGpu::new(None);
    let cli = FakeColab::default();
    let approval = FakeApproval::approving();
    let clock = FakeClock::new(0);
    let cancel = AtomicBool::new(false);
    let out = run_job(
        &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
        &request,
    )
    .unwrap();
    assert_eq!(out.end, JobEnd::Completed);
    assert_eq!(out.uploads, 2);
    assert_eq!(out.session_closed, Some(true));
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/compute/colab_argv.json")).unwrap();
    assert_eq!(fixture["assumed"], true);
    assert_eq!(serde_json::to_value(cli.calls()).unwrap(), fixture["calls"]);
}
#[test]
fn caps_kill_jobs_and_close_sessions() {
    for case in ["local_wall", "colab_wall", "colab_cu"] {
        let temp = TempDir::new().unwrap();
        let mut request = req(temp.path());
        request.limits.wall_clock_s = if case == "colab_cu" { 100 } else { 1 };
        let local = FakeLocalGpu::new(if case == "local_wall" {
            Some(fits())
        } else {
            None
        });
        local.set_duration(Duration::from_secs(60));
        let cli = CountCli::new();
        cli.fake.set_exec_duration(Duration::from_secs(60));
        if case == "colab_cu" {
            cli.fake.set_cu_readings(vec![0.5, 2.0]);
        }
        let approval = FakeApproval::approving();
        let clock = TickClock {
            ms: AtomicI64::new(0),
            step: if case == "colab_cu" { 0 } else { 500 },
        };
        let cancel = AtomicBool::new(false);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert_eq!(
            out.end,
            if case == "colab_cu" {
                JobEnd::ComputeUnitCap
            } else {
                JobEnd::WallClockCap
            }
        );
        if case == "local_wall" {
            assert_eq!(local.kills(), 1);
            assert_eq!(out.session_closed, None);
        } else {
            assert_eq!(cli.kills.load(Ordering::SeqCst), 1);
            assert!(has(&cli.fake, "close"));
            assert_eq!(out.session_closed, Some(true));
        }
        if case == "colab_cu" {
            assert_eq!(out.compute_units, 2.0);
        }
    }
}
#[test]
fn elapsed_cap_takes_precedence_over_successful_poll() {
    let temp = TempDir::new().unwrap();
    let mut request = req(temp.path());
    request.limits.wall_clock_s = 1;
    let local = FakeLocalGpu::new(None);
    let cli = CountCli::new();
    cli.fake.set_exec_duration(Duration::ZERO);
    let approval = FakeApproval::approving();
    let clock = TickClock {
        ms: AtomicI64::new(0),
        step: 1000,
    };
    let cancel = AtomicBool::new(false);
    let out = run_job(
        &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
        &request,
    )
    .unwrap();
    assert_eq!(out.end, JobEnd::WallClockCap);
    assert_eq!(cli.kills.load(Ordering::SeqCst), 1);
    assert!(has(&cli.fake, "close"));
}
#[test]
fn consent_denial_and_changed_manifest_prevent_create() {
    for changed in [false, true] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        if changed {
            let manifest =
                build_manifest(&request.project, &request.project_dir, &request.inputs).unwrap();
            ConsentStore::open(temp.path())
                .unwrap()
                .request(&manifest, &FakeApproval::approving())
                .unwrap();
            fs::write(temp.path().join("data/a"), b"changed").unwrap();
        }
        let local = FakeLocalGpu::new(None);
        let cli = FakeColab::default();
        let approval = FakeApproval::denying();
        let clock = FakeClock::new(0);
        let cancel = AtomicBool::new(false);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert_eq!(out.end, JobEnd::ConsentMissing);
        assert_eq!(out.uploads, 0);
        assert_eq!(out.session_closed, None);
        assert_eq!(cli.uploads(), 0);
        assert!(!has(&cli, "create"));
    }
}
#[test]
fn failures_close_and_failed_close_stays_pending() {
    for step in ["create_after_id", "upload", "exec", "download", "close"] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        let local = FakeLocalGpu::new(None);
        let cli = FakeColab::default();
        cli.fail(step, true);
        let approval = FakeApproval::approving();
        let clock = FakeClock::new(0);
        let cancel = AtomicBool::new(false);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert!(has(&cli, "close"), "{step}");
        if step == "close" {
            assert_eq!(out.session_closed, Some(false));
            assert_eq!(pending(temp.path()).unwrap().len(), 1);
            assert!(out.detail.contains("pending"));
        } else {
            assert_eq!(out.end, JobEnd::Failed, "{step}");
            assert!(pending(temp.path()).unwrap().is_empty());
            assert_eq!(out.session_closed, Some(true));
        }
    }
}
#[test]
fn cancel_running_job_kills_and_closes() {
    struct CancelOnStart<'a> {
        cli: CountCli,
        cancel: &'a AtomicBool,
    }
    impl ColabCli for CancelOnStart<'_> {
        fn run(&self, a: &[String], p: &Path, t: Duration) -> Res<CliOutput> {
            self.cli.run(a, p, t)
        }
        fn start(&self, a: &[String], p: &Path) -> Res<Box<dyn CliJob>> {
            let job = self.cli.start(a, p)?;
            self.cancel.store(true, Ordering::SeqCst);
            Ok(job)
        }
    }
    let temp = TempDir::new().unwrap();
    let request = req(temp.path());
    let local = FakeLocalGpu::new(None);
    let cancel = AtomicBool::new(false);
    let cli = CancelOnStart {
        cli: CountCli::new(),
        cancel: &cancel,
    };
    cli.cli.fake.set_exec_duration(Duration::from_secs(60));
    let approval = FakeApproval::approving();
    let clock = FakeClock::new(0);
    let out = run_job(
        &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
        &request,
    )
    .unwrap();
    assert_eq!(out.end, JobEnd::Cancelled);
    assert_eq!(cli.cli.kills.load(Ordering::SeqCst), 1);
    assert!(has(&cli.cli.fake, "close"));
    assert_eq!(out.session_closed, Some(true));
}

#[test]
fn completion_after_poll_crosses_cap_is_not_success() {
    // CliJob is owned; use an Arc clock so the wrapper can advance it during poll.
    struct OwnedAdvanceJob {
        clock: Arc<FakeClock>,
        inner: Box<dyn CliJob>,
        kills: Arc<AtomicUsize>,
    }
    impl CliJob for OwnedAdvanceJob {
        fn poll(&mut self) -> Res<Option<CliOutput>> {
            self.clock.advance(1000);
            self.inner.poll()
        }
        fn kill(&mut self) {
            self.kills.fetch_add(1, Ordering::SeqCst);
            self.inner.kill();
        }
    }
    struct AdvancingCli {
        clock: Arc<FakeClock>,
        cli: FakeColab,
        kills: Arc<AtomicUsize>,
    }
    impl ColabCli for AdvancingCli {
        fn run(&self, a: &[String], p: &Path, t: Duration) -> Res<CliOutput> {
            self.cli.run(a, p, t)
        }
        fn start(&self, a: &[String], p: &Path) -> Res<Box<dyn CliJob>> {
            Ok(Box::new(OwnedAdvanceJob {
                clock: self.clock.clone(),
                inner: self.cli.start(a, p)?,
                kills: self.kills.clone(),
            }))
        }
    }
    let temp = TempDir::new().unwrap();
    let mut request = req(temp.path());
    request.limits.wall_clock_s = 1;
    let local = FakeLocalGpu::new(None);
    let clock = Arc::new(FakeClock::new(0));
    let cli = AdvancingCli {
        clock: clock.clone(),
        cli: FakeColab::default(),
        kills: Arc::new(AtomicUsize::new(0)),
    };
    let approval = FakeApproval::approving();
    let cancel = AtomicBool::new(false);
    let result = run_job(
        &ctx(
            temp.path(),
            &local,
            &cli,
            &approval,
            clock.as_ref(),
            &cancel,
        ),
        &request,
    )
    .unwrap();
    assert_eq!(result.end, JobEnd::WallClockCap);
    assert_eq!(cli.kills.load(Ordering::SeqCst), 1);
    assert!(has(&cli.cli, "close"));
}

#[test]
fn local_cancel_and_failure() {
    for cancelled in [true, false] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        let local = FakeLocalGpu::new(Some(fits()));
        local.set_exit_code(1);
        let cli = FakeColab::default();
        let approval = FakeApproval::approving();
        let clock = FakeClock::new(0);
        let cancel = AtomicBool::new(cancelled);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        if cancelled {
            assert_eq!(out.end, JobEnd::Cancelled);
            assert_eq!(local.kills(), 1);
        } else {
            assert_eq!(out.end, JobEnd::Failed);
            assert_eq!(out.exit_code, Some(1));
            assert_eq!(local.kills(), 0);
        }
    }
}

#[test]
fn allocation_and_exec_failure_with_close_failure_stay_pending() {
    for step in ["create_after_id", "exec"] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        let local = FakeLocalGpu::new(None);
        let cli = FakeColab::default();
        cli.fail(step, true);
        cli.fail("close", true);
        let approval = FakeApproval::approving();
        let clock = FakeClock::new(0);
        let cancel = AtomicBool::new(false);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert_eq!(out.end, JobEnd::Failed);
        assert_eq!(out.session_closed, Some(false));
        assert!(out.detail.contains("close failed") && out.detail.contains("pending"));
        assert!(has(&cli, "close"));
        assert_eq!(pending(temp.path()).unwrap().len(), 1);
    }
}

#[test]
fn cancel_before_exec_and_during_download_closes() {
    struct CancelAt<'a> {
        cli: FakeColab,
        cancel: &'a AtomicBool,
        step: &'static str,
    }
    impl ColabCli for CancelAt<'_> {
        fn run(&self, a: &[String], p: &Path, t: Duration) -> Res<CliOutput> {
            let result = self.cli.run(a, p, t);
            if a.get(2).is_some_and(|step| step == self.step) {
                self.cancel.store(true, Ordering::SeqCst);
            }
            result
        }
        fn start(&self, a: &[String], p: &Path) -> Res<Box<dyn CliJob>> {
            self.cli.start(a, p)
        }
    }
    for step in ["upload", "download"] {
        let temp = TempDir::new().unwrap();
        let request = req(temp.path());
        let local = FakeLocalGpu::new(None);
        let cancel = AtomicBool::new(false);
        let cli = CancelAt {
            cli: FakeColab::default(),
            cancel: &cancel,
            step,
        };
        let approval = FakeApproval::approving();
        let clock = FakeClock::new(0);
        let out = run_job(
            &ctx(temp.path(), &local, &cli, &approval, &clock, &cancel),
            &request,
        )
        .unwrap();
        assert_eq!(out.end, JobEnd::Cancelled, "{step}");
        assert_eq!(out.session_closed, Some(true));
        assert!(has(&cli.cli, "close"));
        let exec_called = cli
            .cli
            .calls()
            .iter()
            .any(|a| a.get(1).is_some_and(|s| s == "exec"));
        assert_eq!(exec_called, step == "download");
    }
}

#[test]
fn nvidia_start_runs_real_child_in_project_and_maps_exit() {
    let temp = TempDir::new().unwrap();
    let gpu = axiom::compute::local::NvidiaSmi { max_minutes: 1.0 };
    for code in [0, 1] {
        let mut request = req(temp.path());
        request.argv = vec![
            common::python(),
            "-c".into(),
            format!(
                "import pathlib,sys; pathlib.Path('cwd_marker').write_text('ok'); sys.exit({code})"
            ),
        ];
        let mut job = gpu.start(&request).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let output = loop {
            if let Some(out) = job.poll().unwrap() {
                break out;
            }
            assert!(std::time::Instant::now() < deadline, "child timed out");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(output.code, code);
        assert_eq!(
            fs::read_to_string(temp.path().join("cwd_marker")).unwrap(),
            "ok"
        );
    }
}

#[test]
fn nvidia_start_kills_real_sleeping_child() {
    let temp = TempDir::new().unwrap();
    let mut request = req(temp.path());
    request.argv = vec![
        common::python(),
        "-c".into(),
        "import os,pathlib,time; pathlib.Path('pid').write_text(str(os.getpid())); time.sleep(60)"
            .into(),
    ];
    let gpu = axiom::compute::local::NvidiaSmi { max_minutes: 1.0 };
    let mut job = gpu.start(&request).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let pid = loop {
        if let Ok(text) = fs::read_to_string(temp.path().join("pid")) {
            if let Ok(pid) = text.parse::<u32>() {
                break pid;
            }
        }
        assert!(std::time::Instant::now() < deadline, "child did not start");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(axiom::base::proc::pid_alive(pid));
    job.kill();
    assert!(axiom::base::proc::wait_pid_exit(
        pid,
        Duration::from_secs(10)
    ));
}
