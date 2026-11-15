//! Chat sessions and their child launches.
use super::{
    context::{self, DEFAULT_CONTEXT_BYTES},
    launch::{LaunchKind, LaunchRequest, Launched, Launcher},
    promotion::{self, Promotion},
    providers::{Ask, Member, Provider, MAX_CALLS_PER_TURN},
    readonly::{ReadOnly, READ_TOOLS},
    runs,
    store::Transcript,
    Message, Mode, Run, Session,
};
use crate::base::{Failure, Res};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ChatOptions {
    pub with_astra: bool,
    pub context_bytes: usize,
}
impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            with_astra: false,
            context_bytes: DEFAULT_CONTEXT_BYTES,
        }
    }
}

#[derive(Debug)]
pub struct Chat {
    pub home: PathBuf,
    pub session: Session,
    pub run: Run,
    pub opts: ChatOptions,
}
impl Chat {
    pub fn open(
        home: &Path,
        project: Option<&Path>,
        scope: &[String],
        opts: ChatOptions,
    ) -> Res<Self> {
        let session = runs::create_session(home, project, scope)?;
        let run = runs::start_run(home, &session.id, Mode::Chat, None, None)?;
        Ok(Self {
            home: home.to_path_buf(),
            session,
            run,
            opts,
        })
    }
    pub fn resume(home: &Path, session: &str, opts: ChatOptions) -> Res<Self> {
        let session = runs::load_session(home, session)?;
        let run = match runs::session_runs(home, &session.id)?
            .into_iter()
            .rev()
            .find(|run| run.mode == Mode::Chat && run.finished.is_none())
        {
            Some(run) => run,
            None => runs::start_run(home, &session.id, Mode::Chat, None, None)?,
        };
        Ok(Self {
            home: home.to_path_buf(),
            session,
            run,
            opts,
        })
    }
    pub fn say(&self, provider: &dyn Provider, tools: &ReadOnly, text: &str) -> Res<Vec<Message>> {
        let transcript = Transcript::open(&self.home, &self.session.id)?;
        let mut messages = vec![transcript.append("person", "message", text, None, Value::Null)?];
        let members = if self.opts.with_astra {
            &Member::ALL[..2]
        } else {
            &Member::ALL[..1]
        };
        for &member in members {
            for index in 0..MAX_CALLS_PER_TURN {
                let snapshot = transcript.snapshot(None)?;
                let reply = provider.ask(&Ask {
                    member,
                    mode: Mode::Chat,
                    session: self.session.id.clone(),
                    prompt: text.into(),
                    context: context::bounded(&snapshot.messages, self.opts.context_bytes),
                    tools: READ_TOOLS.iter().map(|tool| (*tool).to_owned()).collect(),
                })?;
                if reply.tool_requests.is_empty() || index + 1 == MAX_CALLS_PER_TURN {
                    messages.push(transcript.append(
                        member.name(),
                        "reply",
                        &reply.text,
                        None,
                        Value::Null,
                    )?);
                    break;
                }
                for request in &reply.tool_requests {
                    let (kind, result) = match tools.call(request) {
                        Ok(result) => ("tool", result),
                        Err(error) => ("refused", error.to_string()),
                    };
                    messages.push(transcript.append(
                        "tool",
                        kind,
                        &result,
                        None,
                        json!({"member": member.name()}),
                    )?);
                }
            }
        }
        Ok(messages)
    }
    pub fn small_action(&self, launcher: &dyn Launcher, goal: &str, verify: &str) -> Res<Launched> {
        if self.session.scope.is_empty() || verify.trim().is_empty() {
            return Err(Failure::new(
                "invalid_argument",
                "quick requires scope and verification",
            ));
        }
        launcher.launch(&LaunchRequest {
            kind: LaunchKind::Quick,
            session: self.session.id.clone(),
            parent_run: self.run.id.clone(),
            goal: goal.into(),
            scope: self.session.scope.clone(),
            verify: Some(verify.into()),
            review: false,
            context: Some(Transcript::open(&self.home, &self.session.id)?.snapshot(None)?),
            sources: vec![],
        })
    }
    pub fn promote(
        &self,
        launcher: &dyn Launcher,
        kind: LaunchKind,
        goal: &str,
        verify: Option<&str>,
    ) -> Res<Promotion> {
        promotion::promote(
            &self.home,
            &self.session,
            &self.run,
            kind,
            goal,
            verify,
            Transcript::open(&self.home, &self.session.id)?.snapshot(None)?,
            launcher,
        )
    }
    pub fn close(&self, outcome: &str) -> Res<Run> {
        runs::finish_run(&self.home, &self.run.id, outcome)
    }
}
