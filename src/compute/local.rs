use super::{
    limits::{CapHit, CapWatch},
    routing::{Compute, LocalCapacity, LocalGpu},
    CliJob, CliOutput, ComputeRequest, JobEnd, JobOutcome, Target,
};
use crate::base::{proc, Failure, Res};
use std::{
    io::Read,
    process::{Child, Stdio},
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};

pub struct NvidiaSmi {
    pub max_minutes: f64,
}
fn parse_vram_gb(output: &str) -> Option<f64> {
    output
        .lines()
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .reduce(f64::max)
        .map(|mib| mib / 1024.0)
}
impl LocalGpu for NvidiaSmi {
    fn capacity(&self) -> Option<LocalCapacity> {
        let args = [
            "nvidia-smi",
            "--query-gpu=memory.total",
            "--format=csv,noheader,nounits",
        ]
        .map(str::to_owned);
        let mut cmd = proc::command(&args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let result =
            proc::capture(proc::spawn_group(&mut cmd).ok()?, Duration::from_secs(10)).ok()?;
        if result.exit_code != 0 || result.timed_out {
            return None;
        }
        Some(LocalCapacity {
            vram_gb: parse_vram_gb(&String::from_utf8_lossy(&result.stdout))?,
            max_minutes: self.max_minutes,
        })
    }
    fn start(&self, req: &ComputeRequest) -> Res<Box<dyn CliJob>> {
        start_process(&req.argv, &req.project_dir)
    }
}
pub(crate) fn start_process(argv: &[String], cwd: &std::path::Path) -> Res<Box<dyn CliJob>> {
    if argv.is_empty() || argv[0].is_empty() {
        return Err(Failure::new("process", "empty command"));
    }
    let mut command = proc::command(argv);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        proc::spawn_group(&mut command).map_err(|e| Failure::new("process", e.to_string()))?;
    fn drain(mut stream: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
        let data = Arc::new(Mutex::new(Vec::new()));
        let target = data.clone();
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut out = target.lock().unwrap();
                out.extend_from_slice(&buf[..n]);
                if out.len() > 65536 {
                    let extra = out.len() - 65536;
                    out.drain(..extra);
                }
            }
        });
        data
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    Ok(Box::new(ProcessJob {
        child,
        stdout,
        stderr,
        ended: false,
    }))
}
struct ProcessJob {
    child: Child,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    ended: bool,
}
impl CliJob for ProcessJob {
    fn poll(&mut self) -> Res<Option<CliOutput>> {
        let status = self
            .child
            .try_wait()
            .map_err(|e| Failure::new("process", e.to_string()))?;
        Ok(status.map(|s| {
            self.ended = true;
            CliOutput {
                code: s.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&self.stdout.lock().unwrap()).into_owned(),
                stderr: String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned(),
            }
        }))
    }
    fn kill(&mut self) {
        if !self.ended {
            proc::kill_tree(&mut self.child);
            self.ended = true;
        }
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        self.kill();
    }
}
pub fn run(c: &Compute, req: &ComputeRequest) -> Res<JobOutcome> {
    let mut job = c.local.start(req)?;
    let mut watch = CapWatch::new(c.clock, &req.limits, None);
    let (end, exit_code, detail) = loop {
        if c.cancel.load(Ordering::SeqCst) {
            job.kill();
            break (JobEnd::Cancelled, None, "cancelled".into());
        }
        let polled = match job.poll() {
            Ok(result) => result,
            Err(e) => {
                job.kill();
                break (JobEnd::Failed, None, e.to_string());
            }
        };
        match watch.check() {
            Ok(Some(CapHit::WallClock)) => {
                job.kill();
                break (JobEnd::WallClockCap, None, "wall-clock cap reached".into());
            }
            Err(e) => {
                job.kill();
                break (JobEnd::Failed, None, e.to_string());
            }
            _ => {}
        }
        if let Some(out) = polled {
            break (
                if out.code == 0 {
                    JobEnd::Completed
                } else {
                    JobEnd::Failed
                },
                Some(out.code),
                out.stderr,
            );
        }
        std::thread::sleep(c.poll);
    };
    Ok(JobOutcome {
        job_id: req.job_id.clone(),
        target: Target::Local,
        end,
        exit_code,
        gpu_seconds: watch.elapsed_s(),
        compute_units: 0.0,
        session_closed: None,
        uploads: 0,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_vram_gb;
    #[test]
    fn parses_maximum_gpu_memory() {
        assert_eq!(parse_vram_gb(""), None);
        assert_eq!(parse_vram_gb("bad\n0\n-1\nNaN\ninf"), None);
        assert_eq!(parse_vram_gb(" 8192 \n16384\n4096\n"), Some(16.0));
    }
}
