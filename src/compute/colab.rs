use super::{
    consent::{build_manifest, ConsentStore},
    limits::{CapHit, CapWatch},
    local::start_process,
    routing::Compute,
    session::{argv, pending, ColabMeter, ColabSession},
    CliJob, CliOutput, ColabCli, ComputeRequest, JobEnd, JobOutcome, Target,
};
use crate::base::{proc, Failure, Res};
use std::{
    path::Path,
    process::Stdio,
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};

// Allocation closes an acquired session internally on error; observe that call.
struct AllocationCli<'a> {
    inner: &'a dyn ColabCli,
    close: Mutex<Option<bool>>,
}
impl ColabCli for AllocationCli<'_> {
    fn run(&self, args: &[String], cwd: &Path, timeout: Duration) -> Res<CliOutput> {
        let result = self.inner.run(args, cwd, timeout);
        if args.get(1).map(String::as_str) == Some("session")
            && args.get(2).map(String::as_str) == Some("close")
        {
            *self.close.lock().unwrap() = Some(matches!(&result, Ok(out) if out.code == 0));
        }
        result
    }
    fn start(&self, args: &[String], cwd: &Path) -> Res<Box<dyn CliJob>> {
        self.inner.start(args, cwd)
    }
}

pub struct WslColabCli;
/// Build the WSL wrapper without shell interpolation.
pub fn wsl_argv(cwd: &Path, argv: &[String]) -> Vec<String> {
    let mut result = vec![
        "wsl.exe".into(),
        "--cd".into(),
        cwd.to_string_lossy().into_owned(),
        "--exec".into(),
    ];
    result.extend_from_slice(argv);
    result
}
impl ColabCli for WslColabCli {
    fn run(&self, argv: &[String], cwd: &Path, timeout: Duration) -> Res<CliOutput> {
        if argv.is_empty() {
            return Err(Failure::new("process", "empty command"));
        }
        let mut cmd = proc::command(&wsl_argv(cwd, argv));
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child =
            proc::spawn_group(&mut cmd).map_err(|e| Failure::new("process", e.to_string()))?;
        let out = proc::capture(child, timeout)?;
        if out.timed_out {
            return Err(Failure::new("timeout", "WSL command timed out"));
        }
        Ok(CliOutput {
            code: out.exit_code,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
    fn start(&self, argv: &[String], cwd: &Path) -> Res<Box<dyn CliJob>> {
        if argv.is_empty() {
            return Err(Failure::new("process", "empty command"));
        }
        start_process(&wsl_argv(cwd, argv), cwd)
    }
}
pub fn run(c: &Compute, req: &ComputeRequest) -> Res<JobOutcome> {
    let manifest = build_manifest(&req.project, &req.project_dir, &req.inputs)?;
    let store = ConsentStore::open(c.home)?;
    let mut outcome = JobOutcome {
        job_id: req.job_id.clone(),
        target: Target::Colab,
        end: JobEnd::Failed,
        exit_code: None,
        gpu_seconds: 0.0,
        compute_units: 0.0,
        session_closed: None,
        uploads: 0,
        detail: String::new(),
    };
    if !store.is_granted(&manifest)? && !store.request(&manifest, c.approval)? {
        outcome.end = JobEnd::ConsentMissing;
        outcome.detail = "upload consent missing".into();
        return Ok(outcome);
    }
    if c.cancel.load(Ordering::SeqCst) {
        outcome.end = JobEnd::Cancelled;
        outcome.detail = "cancelled".into();
        return Ok(outcome);
    }
    let before = pending(c.home)?;
    let allocation_cli = AllocationCli {
        inner: c.colab,
        close: Mutex::new(None),
    };
    let mut session = match ColabSession::allocate(&allocation_cli, c.home, req) {
        Ok(session) => session,
        Err(e) => {
            outcome.detail = e.to_string();
            let after = pending(c.home)?;
            let still_pending = after
                .iter()
                .any(|p| p.job_id == req.job_id && !before.contains(p));
            outcome.session_closed = allocation_cli
                .close
                .lock()
                .unwrap()
                .map(|closed| closed && !still_pending);
            if outcome.session_closed == Some(false) {
                outcome
                    .detail
                    .push_str("; close failed and session is pending");
            }
            return Ok(outcome);
        }
    };
    let meter = ColabMeter::new(c.colab, session.id(), &req.project_dir);
    let mut watch = CapWatch::new(c.clock, &req.limits, Some(&meter));
    let uploaded = match session.upload(&store, &manifest) {
        Ok(n) => {
            outcome.uploads = n;
            true
        }
        Err(e) => {
            outcome.detail = e.to_string();
            false
        }
    };
    if uploaded {
        if c.cancel.load(Ordering::SeqCst) {
            outcome.end = JobEnd::Cancelled;
            outcome.detail = "cancelled".into();
        } else {
            match c
                .colab
                .start(&argv::exec(session.id(), &req.argv), &req.project_dir)
            {
                Err(e) => outcome.detail = e.to_string(),
                Ok(mut job) => loop {
                    if c.cancel.load(Ordering::SeqCst) {
                        job.kill();
                        outcome.end = JobEnd::Cancelled;
                        outcome.detail = "cancelled".into();
                        break;
                    }
                    let polled = match job.poll() {
                        Ok(result) => result,
                        Err(e) => {
                            job.kill();
                            outcome.detail = e.to_string();
                            break;
                        }
                    };
                    match watch.check() {
                        Ok(Some(cap)) => {
                            job.kill();
                            outcome.end = match cap {
                                CapHit::WallClock => JobEnd::WallClockCap,
                                CapHit::ComputeUnits => JobEnd::ComputeUnitCap,
                            };
                            outcome.detail = "compute cap reached".into();
                            break;
                        }
                        Err(e) => {
                            job.kill();
                            outcome.detail = e.to_string();
                            break;
                        }
                        Ok(None) => {}
                    }
                    if let Some(out) = polled {
                        outcome.exit_code = Some(out.code);
                        outcome.end = if out.code == 0 {
                            JobEnd::Completed
                        } else {
                            JobEnd::Failed
                        };
                        outcome.detail = out.stderr;
                        break;
                    }
                    std::thread::sleep(c.poll);
                },
            }
        }
    }
    if outcome.end == JobEnd::Completed {
        for path in &req.outputs {
            if c.cancel.load(Ordering::SeqCst) {
                outcome.end = JobEnd::Cancelled;
                outcome.detail = "cancelled".into();
                break;
            }
            match c.colab.run(
                &argv::download(session.id(), path),
                &req.project_dir,
                Duration::from_secs(30),
            ) {
                Ok(out) if out.code == 0 => {}
                Ok(out) => {
                    outcome.end = JobEnd::Failed;
                    outcome.detail = format!("download failed: {}", out.stderr);
                    break;
                }
                Err(e) => {
                    outcome.end = JobEnd::Failed;
                    outcome.detail = e.to_string();
                    break;
                }
            }
            if c.cancel.load(Ordering::SeqCst) {
                outcome.end = JobEnd::Cancelled;
                outcome.detail = "cancelled".into();
                break;
            }
        }
    }
    outcome.gpu_seconds = watch.elapsed_s();
    outcome.compute_units = watch.last_cu();
    let closed = session.close();
    outcome.session_closed = Some(closed);
    if !closed {
        if !outcome.detail.is_empty() {
            outcome.detail.push_str("; ");
        }
        outcome
            .detail
            .push_str("close failed and session is pending");
    }
    Ok(outcome)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapper_preserves_arguments() {
        let args = vec!["tool".into(), "space here".into(), "$(literal)".into()];
        assert_eq!(
            wsl_argv(Path::new("/project path"), &args),
            [
                "wsl.exe",
                "--cd",
                "/project path",
                "--exec",
                "tool",
                "space here",
                "$(literal)"
            ]
        );
    }
}
