//! Runs wishes for a GUI: start or resume one, observe structured events, interject, stop.
use crate::{
    base::{Failure, Res},
    session::{self, content_text, tool_label, Config, Writer},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub creative: bool,
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Claude is up; `session` is the id to resume with later.
    Started { session: String },
    Text(String),
    ToolUse { name: String, label: Option<String> },
    ToolResult { name: String, summary: String },
    Result { text: String, is_error: bool },
    Exited { code: i32 },
    Failed(Failure),
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn summary(s: &str) -> String {
    let mut chars = s.chars();
    let mut out: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[derive(Default)]
struct Translator {
    tools: BTreeMap<String, String>,
}
impl Translator {
    fn event(&mut self, event: &Value, tx: &mpsc::Sender<Event>) {
        let emit = |e| {
            let _ = tx.send(e);
        };
        match text(&event["type"]) {
            "system" if text(&event["subtype"]) == "init" => {
                if let Some(id) = event["session_id"].as_str() {
                    emit(Event::Started { session: id.into() });
                }
            }
            "assistant" => {
                for b in event["message"]["content"].as_array().into_iter().flatten() {
                    match text(&b["type"]) {
                        "text" => emit(Event::Text(text(&b["text"]).into())),
                        "tool_use" => {
                            let name = text(&b["name"]).to_string();
                            self.tools.insert(text(&b["id"]).into(), name.clone());
                            emit(Event::ToolUse {
                                label: tool_label(&name, &b["input"]),
                                name,
                            });
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for b in event["message"]["content"].as_array().into_iter().flatten() {
                    if text(&b["type"]) != "tool_result" {
                        continue;
                    }
                    let name = self
                        .tools
                        .remove(text(&b["tool_use_id"]))
                        .unwrap_or_default();
                    let summary = summary(&content_text(&b["content"]).unwrap_or_default());
                    emit(Event::ToolResult { name, summary });
                }
            }
            "result" => emit(Event::Result {
                text: text(&event["result"]).into(),
                is_error: event["is_error"].as_bool().unwrap_or(false),
            }),
            _ => {}
        }
    }
}

/// One running wish. Dropping it stops the wish and waits for the process tree to die.
pub struct Wish {
    writer: Writer,
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<Event>,
    worker: Option<JoinHandle<Res<i32>>>,
}
impl Wish {
    pub fn start(config: Config, project: &Path, wish: &str, options: &Options) -> Res<Self> {
        if wish.trim().is_empty() && !options.creative {
            return Err(Failure::new("invalid_argument", "the wish is empty"));
        }
        if !project.is_dir() {
            return Err(Failure::new(
                "invalid_argument",
                format!("not a project directory: {}", project.display()),
            ));
        }
        let text = session::wish_text(wish.trim(), options.creative, false);
        let spawned = session::spawn(&config, "wish", &text, project, options.max_turns, None)?;
        Ok(Self::drive(config, spawned))
    }
    pub fn resume(config: Config, session: &str, max_turns: Option<u32>) -> Res<Self> {
        let (prompt, cwd) = session::resume_target(&config, session)?;
        let spawned = session::spawn(&config, "wish", &prompt, &cwd, max_turns, Some(session))?;
        Ok(Self::drive(config, spawned))
    }
    fn drive(config: Config, spawned: session::Spawned) -> Self {
        let writer = Arc::clone(&spawned.writer);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::channel();
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let mut translator = Translator::default();
            let result = session::drive(
                spawned,
                &config,
                &mut std::io::sink(),
                &flag,
                &mut |event| translator.event(event, &tx),
            );
            let _ = tx.send(match &result {
                Ok(code) => Event::Exited { code: *code },
                Err(e) => Event::Failed(e.clone()),
            });
            result
        });
        Self {
            writer,
            stop,
            events,
            worker: Some(worker),
        }
    }
    pub fn events(&self) -> &mpsc::Receiver<Event> {
        &self.events
    }
    /// Sends one line to claude as a user message while the wish is still running.
    pub fn interject(&self, line: &str) -> Res<()> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return Err(Failure::new("invalid_argument", "nothing to send"));
        }
        let mut guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stdin) = guard.as_mut() else {
            return Err(Failure::new("finished", "the wish has finished"));
        };
        session::send(stdin, line).map_err(|e| Failure::new("io", e.to_string()))
    }
    pub fn is_running(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| !w.is_finished())
    }
    /// Asks the run loop to kill the process tree; the goal is marked blocked so it can be resumed.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    /// Waits for the wish to end and returns its exit code.
    pub fn wait(mut self) -> Res<i32> {
        self.join()
    }
    fn join(&mut self) -> Res<i32> {
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .unwrap_or_else(|_| Err(Failure::new("internal", "the session thread panicked"))),
            None => Err(Failure::new("finished", "the wish has finished")),
        }
    }
}
impl std::fmt::Debug for Wish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wish")
            .field("running", &self.is_running())
            .finish()
    }
}
impl Drop for Wish {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.stop();
            let _ = self.join();
        }
    }
}
