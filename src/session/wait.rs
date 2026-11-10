use super::{
    run::{self, run_observed, until_interrupted},
    Config,
};
use crate::{
    base::{Failure, Res},
    goal::{self, Batch, BatchEnd, Goal},
};
use std::{
    collections::VecDeque,
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

/// Lines read from the input, plus those an ended run received and handed back, which come first.
struct Lines {
    read: mpsc::Receiver<Vec<u8>>,
    kept: VecDeque<Vec<u8>>,
}

/// Input shared by the consecutive runs of one process: one thread reads it, and each run gets a
/// reader that ends with the run, so a finished run cannot swallow a line typed for the next one.
struct SharedInput {
    lines: Arc<Mutex<Lines>>,
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
            lines: Arc::new(Mutex::new(Lines {
                read: rx,
                kept: VecDeque::new(),
            })),
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
    fn hand_back(&self) -> run::HandBack {
        let lines = Arc::clone(&self.lines);
        Box::new(move |line| {
            lines
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .kept
                .push_front((line + "\n").into_bytes());
        })
    }
}
impl Drop for SharedInput {
    fn drop(&mut self) {
        self.end_run();
    }
}
struct RunInput {
    lines: Arc<Mutex<Lines>>,
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
            let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
            let next = match lines.kept.pop_front() {
                Some(line) => Ok(line),
                None => lines.read.recv_timeout(RUN_END_CHECK),
            };
            match next {
                // The run ended while this reader waited, so the line is the next run's.
                Ok(line) if self.ended.load(Ordering::SeqCst) => {
                    lines.kept.push_front(line);
                    break;
                }
                Ok(line) => self.line = Cursor::new(line),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    drop(lines);
                    std::thread::sleep(Duration::from_millis(1));
                }
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
    run::resume_keeping(
        config,
        session,
        max_turns,
        message,
        lines.for_run(),
        Some(lines.hand_back()),
        out,
    )
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
    run_observing(
        config,
        skill,
        text,
        project,
        max_turns,
        input,
        out,
        launch,
        &mut |_| {},
    )
}

/// Like `run_with`, reporting the first session ID observed in a system init event.
#[allow(clippy::too_many_arguments)]
pub fn run_observing(
    config: &Config,
    skill: &str,
    text: &str,
    project: &Path,
    max_turns: Option<u32>,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
    launch: &run::Launch,
    on_session: &mut dyn FnMut(&str),
) -> Res<i32> {
    let mut lines = SharedInput::new(input);
    let mut session = None;
    let mut reported = false;
    let code = run_observed(
        config,
        skill,
        text,
        project,
        max_turns,
        lines.for_run(),
        Some(lines.hand_back()),
        out,
        None,
        &mut |event| {
            if event["type"] == "system" && event["subtype"] == "init" {
                session = event["session_id"].as_str().map(str::to_owned);
                if !reported {
                    if let Some(id) = session.as_deref() {
                        on_session(id);
                        reported = true;
                    }
                }
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

    /// Input whose lines arrive when the test sends them.
    struct Fed(mpsc::Receiver<Vec<u8>>);
    impl Read for Fed {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Ok(line) = self.0.recv() else {
                return Ok(0);
            };
            buf[..line.len()].copy_from_slice(&line);
            Ok(line.len())
        }
    }
    fn read_within(mut reader: Box<dyn BufRead + Send>) -> String {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(read(&mut reader)));
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the reader did not return")
    }

    struct SignalConsumed {
        reader: Box<dyn BufRead + Send>,
        signal: Option<mpsc::Sender<()>>,
    }
    impl Read for SignalConsumed {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.fill_buf()?.read(buf)?;
            self.consume(n);
            Ok(n)
        }
    }
    impl BufRead for SignalConsumed {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.reader.fill_buf()
        }
        fn consume(&mut self, amount: usize) {
            let complete = self.reader.fill_buf().unwrap()[..amount].contains(&b'\n');
            self.reader.consume(amount);
            if complete {
                if let Some(signal) = self.signal.take() {
                    signal.send(()).unwrap();
                }
            }
        }
    }

    #[test]
    fn a_line_held_by_the_finished_forwarder_goes_to_the_next_run_once() {
        let (feed, fed) = mpsc::channel();
        let mut lines = SharedInput::new(Box::new(io::BufReader::new(Fed(fed))));
        let (consumed, wait_consumed) = mpsc::channel();
        let one = Box::new(SignalConsumed {
            reader: lines.for_run(),
            signal: Some(consumed),
        });
        let writer = Arc::new(Mutex::new(None));
        let drive_writer = Arc::clone(&writer);
        run::forward_input_keeping(writer, one, Some(lines.hand_back()));
        feed.send(b"late\n".to_vec()).unwrap();
        wait_consumed.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(drive_writer);
        lines.end_run();

        let mut next = lines.for_run();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            tx.send(read(&mut next)).unwrap();
            tx.send(read(&mut next)).unwrap();
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5))
                .expect("the reader did not return"),
            "late\n"
        );
        drop(feed);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "");
    }

    #[test]
    fn a_line_typed_while_the_ended_run_is_receiving_goes_to_the_next_run() {
        for _ in 0..10 {
            let (feed, fed) = mpsc::channel();
            let mut lines = SharedInput::new(Box::new(io::BufReader::new(Fed(fed))));
            let one = lines.for_run();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || tx.send(read_within(one)));
            std::thread::sleep(RUN_END_CHECK / 4);
            lines.end_run();
            feed.send(b"for the next run\n".to_vec()).unwrap();
            assert_eq!(rx.recv().unwrap(), "", "the ended run took the line");
            assert_eq!(read_within(lines.for_run()), "for the next run\n");
        }
    }

    #[test]
    fn end_of_input_reaches_the_current_run() {
        let mut lines = SharedInput::new(Box::new(Cursor::new(Vec::new())));
        let mut reader = lines.for_run();
        assert_eq!(read(&mut reader), "");
    }

    const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

    fn python() -> String {
        if let Ok(p) = std::env::var("AXIOM_TEST_PYTHON") {
            return p;
        }
        let names = if cfg!(windows) {
            ["python", "python3"]
        } else {
            ["python3", "python"]
        };
        names
            .into_iter()
            .find(|p| {
                crate::base::proc::cmd(p)
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .expect("Python not found")
            .into()
    }

    fn fake_config(temp: &tempfile::TempDir) -> Config {
        let script = temp.path().join("claude.py");
        std::fs::write(&script, r#"
import json, pathlib, sys, threading
if '--version' in sys.argv:
    print('fake claude 1.0')
    sys.exit(0)
stage, record = map(pathlib.Path, sys.argv[1:3])
sys.stdin.readline()
if not stage.exists():
    stage.touch()
    if '--resume' not in sys.argv:
        print(json.dumps({'type': 'system', 'subtype': 'init', 'session_id': '12345678-1234-1234-1234-123456789abc'}), flush=True)
else:
    def receive():
        record.write_text(sys.stdin.readline(), encoding='utf-8')
    reader = threading.Thread(target=receive, daemon=True)
    reader.start()
    reader.join(2)
print(json.dumps({'type': 'result', 'is_error': False}), flush=True)
"#).unwrap();
        Config {
            claude: Ok(vec![
                python(),
                "-I".into(),
                script.to_string_lossy().into_owned(),
                temp.path().join("stage").to_string_lossy().into_owned(),
                temp.path().join("record").to_string_lossy().into_owned(),
            ]),
            home: temp.path().join("home"),
            bin: temp.path().join("axiom"),
            clock: super::super::run::Clock::system(),
        }
    }

    // The log is written after the first child has exited and its writer was emptied, but
    // before drive releases the writer. Keep drive there until the forwarder can take the line.
    struct FeedOnLog<'a> {
        feed: mpsc::Sender<Vec<u8>>,
        fed: bool,
        read_again: mpsc::Receiver<()>,
        activate_on_batch: Option<&'a Config>,
        output: Vec<u8>,
    }
    impl Write for FeedOnLog<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            let text = String::from_utf8_lossy(bytes);
            if text.contains("log: ") {
                if !self.fed {
                    self.fed = true;
                    self.feed.send(b"late line\n".to_vec()).unwrap();
                    self.read_again
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(750));
                }
            }
            if text.contains("batch finished; resuming") {
                if let Some(config) = self.activate_on_batch.take() {
                    goal::set_status(&config.home, SESSION, "blocked", Some("test")).unwrap();
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct SignalFed {
        fed: Fed,
        read_again: Option<mpsc::Sender<()>>,
        read_once: bool,
    }
    impl Read for SignalFed {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.read_once {
                if let Some(signal) = self.read_again.take() {
                    signal.send(()).unwrap();
                }
            }
            let n = self.fed.read(buf)?;
            self.read_once = true;
            Ok(n)
        }
    }
    fn input_and_output(config: Option<&Config>) -> (Box<dyn BufRead + Send>, FeedOnLog<'_>) {
        let (feed, fed) = mpsc::channel();
        let (signal, read_again) = mpsc::channel();
        (
            Box::new(io::BufReader::new(SignalFed {
                fed: Fed(fed),
                read_again: Some(signal),
                read_once: false,
            })),
            FeedOnLog {
                feed,
                fed: false,
                read_again,
                activate_on_batch: config,
                output: Vec::new(),
            },
        )
    }

    #[test]
    fn resume_run_hands_the_held_line_to_the_next_reader() {
        let temp = tempfile::tempdir().unwrap();
        let config = fake_config(&temp);
        goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();
        goal::set_status(&config.home, SESSION, "waiting", Some("waiting")).unwrap();
        let (input, mut out) = input_and_output(None);
        let mut lines = SharedInput::new(input);
        assert_eq!(
            resume_run(&config, SESSION, None, None, &mut lines, &mut out).unwrap(),
            0,
            "{}",
            String::from_utf8_lossy(&out.output)
        );
        assert_eq!(read_within(lines.for_run()), "late line\n");
    }

    #[test]
    fn run_with_hands_the_held_line_to_the_resumed_run() {
        let temp = tempfile::tempdir().unwrap();
        let config = fake_config(&temp);
        goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();
        let marker = temp.path().join("done");
        std::fs::write(&marker, "").unwrap();
        goal::set_status_with_batch(
            &config.home,
            SESSION,
            "waiting",
            Some("waiting"),
            Some(Batch {
                progress_file: temp.path().join("progress").to_string_lossy().into_owned(),
                marker_file: marker.to_string_lossy().into_owned(),
                pid: std::process::id(),
            }),
        )
        .unwrap();
        let (input, mut out) = input_and_output(Some(&config));
        assert_eq!(
            run_with(
                &config,
                "wish",
                "wish",
                temp.path(),
                None,
                input,
                &mut out,
                &run::Launch::default()
            )
            .unwrap(),
            0
        );
        let record = std::fs::read_to_string(temp.path().join("record")).unwrap_or_default();
        let message: serde_json::Value = serde_json::from_str(&record).unwrap_or_else(|_| {
            panic!(
                "resumed run did not receive the line: {}",
                String::from_utf8_lossy(&out.output)
            )
        });
        assert_eq!(message["message"]["content"][0]["text"], "late line");
    }
}
