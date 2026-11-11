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
    /// The council of Astra, Fable 5.2 and Argon. Costs real money, so it is off unless asked for.
    pub advanced_research: bool,
    pub council_budgets: Option<crate::council::Budgets>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Claude is up; `session` is the id to resume with later.
    Started {
        session: String,
    },
    Text(String),
    RateLimit(crate::usage::quota::ProviderQuota),
    ToolUse {
        name: String,
        label: Option<String>,
    },
    ToolResult {
        name: String,
        summary: String,
    },
    Result {
        text: String,
        is_error: bool,
    },
    /// Claude hit its usage limit; the wish resumes by itself at `until` (unix seconds)
    /// and reports `Started` again.
    Waiting {
        until: i64,
    },
    Exited {
        code: i32,
    },
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
            "rate_limit_event" if event["rate_limit_info"]["unifiedWindows"].is_object() => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                if let Some(quota) = crate::usage::quota::parse_claude_event(event, now) {
                    emit(Event::RateLimit(quota));
                }
            }
            "chronos" if text(&event["subtype"]) == "limit_wait" => emit(Event::Waiting {
                until: event["until"].as_i64().unwrap_or(0),
            }),
            _ => {}
        }
    }
}

/// One running wish. Dropping it stops the wish and waits for the process tree to die.
pub struct Wish {
    writer: Writer,
    stop: Arc<AtomicBool>,
    /// Set before `Exited` or `Failed` is sent, so a caller that has seen either never hears
    /// "waiting" while the worker thread is still unwinding.
    ended: Arc<AtomicBool>,
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
        let launch = session::Launch {
            advanced_research: options.advanced_research,
            council_budgets: options.council_budgets,
        };
        let spawned = session::spawn_with(
            &config,
            "wish",
            &text,
            project,
            options.max_turns,
            None,
            &launch,
        )?;
        Ok(Self::drive(config, spawned))
    }
    /// `message` is sent as the first prompt; without one the session gets the continuation prompt.
    pub fn resume(
        config: Config,
        session: &str,
        message: Option<&str>,
        max_turns: Option<u32>,
    ) -> Res<Self> {
        let (prompt, cwd) = session::resume_target(&config, session, message)?;
        let spawned = session::spawn(&config, "wish", &prompt, &cwd, max_turns, Some(session))?;
        Ok(Self::drive(config, spawned))
    }
    fn drive(config: Config, spawned: session::Spawned) -> Self {
        let writer = Arc::clone(&spawned.writer);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::channel();
        let flag = Arc::clone(&stop);
        let ended = Arc::new(AtomicBool::new(false));
        let ended_flag = Arc::clone(&ended);
        let worker = std::thread::spawn(move || {
            let mut translator = Translator::default();
            let result = session::drive(
                spawned,
                &config,
                &mut std::io::sink(),
                &flag,
                &mut |event| translator.event(event, &tx),
            );
            ended_flag.store(true, Ordering::SeqCst);
            let _ = tx.send(match &result {
                Ok(code) => Event::Exited { code: *code },
                Err(e) => Event::Failed(e.clone()),
            });
            result
        });
        Self {
            writer,
            stop,
            ended,
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
            return Err(if !self.ended.load(Ordering::SeqCst) && self.is_running() {
                Failure::new("waiting", "claude is waiting for its usage limit to reset")
            } else {
                Failure::new("finished", "the wish has finished")
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::quota::{Window, WindowKind};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn translator_emits_claude_rate_limit_at_receipt_time() {
        let event: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/quota/claude-rate-limit-1.jsonl"
        ))
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        Translator::default().event(&event, &tx);
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let Event::RateLimit(quota) = rx.try_recv().unwrap() else {
            panic!("expected rate limit")
        };
        let observed = quota.five_hour.as_ref().unwrap().observed_at;
        assert!((before..=after).contains(&observed));
        assert_eq!(
            quota.five_hour,
            Some(Window {
                kind: WindowKind::FiveHour,
                used: 0.17,
                resets_at: Some(1793652000),
                observed_at: observed,
                status: Some("allowed".into())
            })
        );
        assert_eq!(
            quota.weekly,
            Some(Window {
                kind: WindowKind::Weekly,
                used: 0.11,
                resets_at: Some(1794218400),
                observed_at: observed,
                status: Some("allowed".into())
            })
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn translator_ignores_malformed_claude_rate_limit() {
        let (tx, rx) = mpsc::channel();
        Translator::default().event(
            &serde_json::json!({"type":"rate_limit_event", "rate_limit_info":{}}),
            &tx,
        );
        assert!(rx.try_recv().is_err());
    }
}
