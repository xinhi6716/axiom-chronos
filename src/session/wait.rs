use super::{
    run::{self, run_observed, until_interrupted},
    Config,
};
use crate::{
    base::{Failure, Res},
    goal::{self, Batch, BatchEnd, Goal},
};
use std::{
    io::{self, BufRead, Cursor, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

const INTERRUPTED_EXIT: i32 = 130;
const RUN_END_CHECK: Duration = Duration::from_millis(100);

/// Input shared by the consecutive runs of one process: one thread reads it, and each run gets a
/// reader that ends with the run, so a finished run cannot swallow a line typed for the next one.
struct SharedInput {
    lines: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    ended: Arc<AtomicBool>,
}
impl SharedInput {
    fn new(mut input: Box<dyn BufRead + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || loop {
            let mut line = Vec::new();
            match input.read_until(b'\n', &mut line) {
                Ok(n) if n > 0 && tx.send(line).is_ok() => {}
                _ => break,
            }
        });
        Self {
            lines: Arc::new(Mutex::new(rx)),
            ended: Arc::new(AtomicBool::new(true)),
        }
    }
    fn for_run(&mut self) -> Box<dyn BufRead + Send> {
        self.end_run();
        self.ended = Arc::new(AtomicBool::new(false));
        Box::new(RunInput {
            lines: Arc::clone(&self.lines),
            ended: Arc::clone(&self.ended),
            line: Cursor::new(Vec::new()),
        })
    }
    fn end_run(&self) {
        self.ended.store(true, Ordering::SeqCst);
    }
}
impl Drop for SharedInput {
    fn drop(&mut self) {
        self.end_run();
    }
}
struct RunInput {
    lines: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    ended: Arc<AtomicBool>,
    line: Cursor<Vec<u8>>,
}
impl Read for RunInput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.fill_buf()?.read(buf)?;
        self.consume(n);
        Ok(n)
    }
}
impl BufRead for RunInput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        while self.line.position() >= self.line.get_ref().len() as u64
            && !self.ended.load(Ordering::SeqCst)
        {
            let next = self
                .lines
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recv_timeout(RUN_END_CHECK);
            match next {
                Ok(line) => self.line = Cursor::new(line),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.line.fill_buf()
    }
    fn consume(&mut self, amount: usize) {
        self.line.consume(amount);
    }
}

fn say(out: &mut dyn Write, text: &str) {
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}
fn waiting_on_batch(config: &Config, session: &str) -> Option<(Goal, Batch)> {
    let mut goal = goal::load(&config.home, session).ok()??;
    let batch = goal.batch.take().filter(|_| goal.status == "waiting")?;
    Some((goal, batch))
}
fn check_resumable(config: &Config, session: &str, goal: &Goal) -> Res<()> {
    config.claude.as_ref().map_err(Clone::clone)?;
    if goal.cwd.as_deref().is_some_and(|c| Path::new(c).is_dir()) {
        Ok(())
    } else {
        Err(Failure::new(
            "invalid_argument",
            format!("goal {session} has no project directory"),
        ))
    }
}
fn watch(batch: &Batch, out: &mut dyn Write) -> Option<BatchEnd> {
    say(
        out,
        &format!(
            "waiting on batch pid {} until {} appears; no model runs meanwhile, and Ctrl-C leaves the batch running",
            batch.pid, batch.marker_file
        ),
    );
    until_interrupted(|stop| {
        goal::wait_for_batch(
            batch,
            stop,
            &mut |pause| goal::sleep_unless_stopped(stop, pause),
            &mut |text| say(out, &format!("batch: {text}")),
        )
    })
}

fn resume_run(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    message: Option<&str>,
    lines: &mut SharedInput,
    out: &mut dyn Write,
) -> Res<i32> {
    run::resume(config, session, max_turns, message, lines.for_run(), out)
}

/// While the session's goal waits on a batch: wait with no model running, then resume the session.
fn follow(
    config: &Config,
    session: &str,
    mut code: i32,
    max_turns: Option<u32>,
    lines: &mut SharedInput,
    out: &mut dyn Write,
) -> Res<i32> {
    while let Some((goal, batch)) = waiting_on_batch(config, session) {
        lines.end_run();
        let end = if code == INTERRUPTED_EXIT {
            None
        } else {
            check_resumable(config, session, &goal)?;
            watch(&batch, out)
        };
        let Some(end) = end else {
            say(
                out,
                &format!("goal waiting; continue with: axiom wait {session}"),
            );
            return Ok(INTERRUPTED_EXIT);
        };
        if !goal::batch_ended(&config.home, session, &batch, end)? {
            say(
                out,
                &format!("goal {session} no longer waits on this batch; not resuming it"),
            );
            return Ok(code);
        }
        say(
            out,
            &match end {
                BatchEnd::Finished => {
                    format!("batch finished; resuming with: axiom wish --resume {session}")
                }
                BatchEnd::Died => format!(
                    "batch died without its completion marker; resuming to diagnose it with: axiom wish --resume {session}"
                ),
            },
        );
        code = resume_run(config, session, max_turns, None, lines, out)?;
    }
    Ok(code)
}

/// Runs a skill, then waits and resumes for as long as the session's goal ends waiting on a batch.
pub fn run(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    run_with(
        config,
        skill,
        text,
        project,
        max_turns,
        input,
        out,
        &run::Launch::default(),
    )
}

/// `run` with what the launch opts in to.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
    launch: &run::Launch,
) -> Res<i32> {
    let mut lines = SharedInput::new(input);
    let mut session = None;
    let code = run_observed(
        config,
        skill,
        text,
        project,
        max_turns,
        lines.for_run(),
        out,
        None,
        &mut |event| {
            if event["type"] == "system" && event["subtype"] == "init" {
                session = event["session_id"].as_str().map(str::to_owned);
            }
        },
        launch,
    )?;
    match session {
        Some(id) => follow(config, &id, code, max_turns, &mut lines, out),
        None => Ok(code),
    }
}

/// Resumes a session, then waits and resumes for as long as its goal ends waiting on a batch.
pub fn resume(
    config: &Config,
    session: &str,
    max_turns: Option<u32>,
    message: Option<&str>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    let mut lines = SharedInput::new(input);
    let code = resume_run(config, session, max_turns, message, &mut lines, out)?;
    follow(config, session, code, max_turns, &mut lines, out)
}

pub fn wait_and_resume(
    config: &Config,
    session: &str,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<i32> {
    let goal = goal::load(&config.home, session)?
        .ok_or_else(|| Failure::new("not_found", format!("no goal for session {session}")))?;
    if goal.status != "waiting" {
        return Err(Failure::new(
            "invalid_argument",
            format!("goal {session} is {}, not waiting", goal.status),
        ));
    }
    if goal.batch.is_none() {
        return Err(Failure::new(
            "invalid_argument",
            format!("goal {session} names no batch to wait on; once its batch is done, continue with: axiom wish --resume {session}"),
        ));
    }
    follow(config, session, 0, None, &mut SharedInput::new(input), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(reader: &mut dyn BufRead) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line
    }

    #[test]
    fn a_finished_run_cannot_take_the_next_runs_line() {
        let mut lines = SharedInput::new(Box::new(Cursor::new(b"first\nsecond\n".to_vec())));
        let mut one = lines.for_run();
        assert_eq!(read(&mut one), "first\n");
        lines.end_run();
        assert_eq!(read(&mut one), "");
        let mut two = lines.for_run();
        assert_eq!(read(&mut two), "second\n");
        assert_eq!(read(&mut two), "");
    }

    struct Silent(mpsc::Receiver<()>);
    impl Read for Silent {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    #[test]
    fn a_blocked_reader_stops_when_its_run_ends() {
        let (_keep_open, silence) = mpsc::channel();
        let mut lines = SharedInput::new(Box::new(io::BufReader::new(Silent(silence))));
        let mut reader = lines.for_run();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(read(&mut reader)));
        lines.end_run();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "");
    }

    #[test]
    fn end_of_input_reaches_the_current_run() {
        let mut lines = SharedInput::new(Box::new(Cursor::new(Vec::new())));
        let mut reader = lines.for_run();
        assert_eq!(read(&mut reader), "");
    }
}
