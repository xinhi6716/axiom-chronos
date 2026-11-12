//! Plaza room and member turns.
pub mod access;
pub mod pins;
pub mod protocol;
pub mod router;

use crate::{
    base::{self, Failure, Res},
    modes::{
        context::{bounded, truncate_utf8, DEFAULT_CONTEXT_BYTES},
        providers::{Ask, Member, Provider, MAX_CALLS_PER_TURN},
        readonly::ReadOnly,
        runs,
        store::Transcript,
        Message, Mode, Run, Session,
    },
};
use router::{Router, Routing};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub struct Room {
    pub home: PathBuf,
    pub session: Session,
    pub run: Run,
}
pub struct PostOutcome {
    pub thread: String,
    pub routing: Option<Routing>,
    pub replies: Vec<Message>,
    pub skipped: Vec<(Member, String)>,
}
pub fn open(
    home: &Path,
    project: Option<&Path>,
    scope: &[String],
    budgets: access::Budgets,
) -> Res<Room> {
    let session = runs::create_session(home, project, scope)?;
    access::save(home, &session.id, &budgets)?;
    let run = runs::start_run(home, &session.id, Mode::Plaza, None, None)?;
    Ok(Room {
        home: home.to_path_buf(),
        session,
        run,
    })
}
pub fn resume(home: &Path, session: &str) -> Res<Room> {
    base::require_slug("session", session)?;
    let session = runs::load_session(home, session)?;
    access::load(home, &session.id)?;
    let run = runs::session_runs(home, &session.id)?
        .into_iter()
        .find(|r| r.mode == Mode::Plaza)
        .ok_or_else(|| Failure::new("not_found", "plaza run not found"))?;
    Ok(Room {
        home: home.to_path_buf(),
        session,
        run,
    })
}
fn distinct(members: &[Member]) -> Res<()> {
    let mut seen = BTreeSet::new();
    if members.iter().any(|m| !seen.insert(*m)) {
        return Err(Failure::new("invalid_argument", "members must be distinct"));
    }
    Ok(())
}
impl Room {
    fn transcript(&self) -> Res<Transcript> {
        Transcript::open(&self.home, &self.session.id)
    }
    fn context(&self, thread: &str) -> Res<String> {
        Ok(bounded(
            &self.transcript()?.thread(thread)?,
            DEFAULT_CONTEXT_BYTES,
        ))
    }
    fn turn(
        &self,
        thread: &str,
        prompt: &str,
        member: Member,
        provider: &dyn Provider,
        tools: &ReadOnly,
    ) -> Res<Message> {
        let transcript = self.transcript()?;
        let mut context = self.context(thread)?;
        let grants = access::grant();
        for index in 0..MAX_CALLS_PER_TURN {
            access::check(&self.home, &self.session.id, member)?;
            let ask = Ask {
                member,
                mode: Mode::Plaza,
                session: self.session.id.clone(),
                prompt: prompt.into(),
                context: truncate_utf8(&context, DEFAULT_CONTEXT_BYTES).to_owned(),
                tools: grants.clone(),
            };
            let reply = provider.ask(&ask)?;
            access::settle(&self.home, &self.session.id, member, &reply)?;
            if reply.tool_requests.is_empty() {
                return transcript.append(
                    member.name(),
                    "text",
                    &reply.text,
                    Some(thread),
                    Value::Null,
                );
            }
            for request in &reply.tool_requests {
                let result = if grants.contains(&request.tool) {
                    tools
                        .call(request)
                        .unwrap_or_else(|e| format!("Tool error: {e}"))
                } else {
                    let refusal = format!("{} refused tool {}", member.name(), request.tool);
                    transcript.append(
                        member.name(),
                        "refused",
                        &refusal,
                        Some(thread),
                        json!({"member": member, "tool": request.tool}),
                    )?;
                    refusal
                };
                context.push_str(&format!("\nTool {}: {}\n", request.tool, result));
            }
            if index + 1 == MAX_CALLS_PER_TURN {
                return transcript.append(
                    member.name(),
                    "text",
                    &reply.text,
                    Some(thread),
                    Value::Null,
                );
            }
            // Keep the latest tool results visible even if the existing transcript fills the context.
            if context.len() > DEFAULT_CONTEXT_BYTES {
                let start = context.len() - DEFAULT_CONTEXT_BYTES;
                let start = (start..context.len())
                    .find(|&i| context.is_char_boundary(i))
                    .unwrap_or(context.len());
                context = context[start..].to_owned();
            }
        }
        unreachable!()
    }
    pub fn post(
        &self,
        text: &str,
        thread: Option<&str>,
        router: &dyn Router,
        provider: &dyn Provider,
        tools: &ReadOnly,
    ) -> Res<PostOutcome> {
        let transcript = self.transcript()?;
        let new_thread = format!(
            "t{}",
            transcript
                .last_seq()?
                .checked_add(1)
                .ok_or_else(|| Failure::new("io", "transcript sequence exhausted"))?
        );
        let creating_thread = thread.is_none();
        let thread = thread.unwrap_or(&new_thread);
        let message = transcript.append("person", "text", text, Some(thread), Value::Null)?;
        let actual_thread = if creating_thread && message.seq.to_string() != new_thread[1..] {
            return Err(Failure::new(
                "io",
                "concurrent transcript append while creating thread",
            ));
        } else {
            thread.to_owned()
        };
        let mentions = protocol::mentions(text);
        let (routing, members) = if mentions.is_empty() {
            let routing = router.route(text, &self.context(&actual_thread)?)?;
            router::validate(&routing)?;
            transcript.append(
                "router",
                "router",
                &routing.note,
                Some(&actual_thread),
                json!({"picks": routing.picks}),
            )?;
            let members = routing.picks.iter().map(|p| p.member).collect();
            (Some(routing), members)
        } else {
            (None, mentions)
        };
        let mut replies = Vec::new();
        let mut skipped = Vec::new();
        for member in members {
            match self.turn(&actual_thread, text, member, provider, tools) {
                Ok(reply) => replies.push(reply),
                Err(e) if e.kind == "budget" => skipped.push((member, e.message)),
                Err(e) => return Err(e),
            }
        }
        Ok(PostOutcome {
            thread: actual_thread,
            routing,
            replies,
            skipped,
        })
    }
    pub fn round(
        &self,
        thread: &str,
        prompt: &str,
        members: &[Member],
        provider: &dyn Provider,
        tools: &ReadOnly,
    ) -> Res<Vec<Message>> {
        let members = if members.is_empty() {
            &Member::ALL[..]
        } else {
            members
        };
        distinct(members)?;
        let mut replies = Vec::new();
        for &member in members {
            match self.turn(thread, prompt, member, provider, tools) {
                Ok(reply) => replies.push(reply),
                Err(e) if e.kind == "budget" => {}
                Err(e) => return Err(e),
            }
        }
        Ok(replies)
    }
    pub fn debate(
        &self,
        thread: &str,
        topic: &str,
        a: Member,
        b: Member,
        judge: Member,
        provider: &dyn Provider,
        tools: &ReadOnly,
    ) -> Res<Vec<Message>> {
        distinct(&[a, b, judge])?;
        let mut replies = Vec::new();
        for member in [a, b] {
            match self.turn(thread, topic, member, provider, tools) {
                Ok(reply) => replies.push(reply),
                Err(e) if e.kind == "budget" => {}
                Err(e) => return Err(e),
            }
        }
        let arguments = replies
            .iter()
            .map(|m| format!("{}: {}", m.author, m.text))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!("Judge the debate on: {topic}\nArguments:\n{arguments}");
        match self.turn(thread, &prompt, judge, provider, tools) {
            Ok(reply) => replies.push(reply),
            Err(e) if e.kind == "budget" => {}
            Err(e) => return Err(e),
        }
        Ok(replies)
    }
    pub fn vote(
        &self,
        thread: &str,
        question: &str,
        options: &[String],
        voters: &[Member],
        provider: &dyn Provider,
    ) -> Res<protocol::VoteResult> {
        if options.len() < 2 || options.iter().collect::<BTreeSet<_>>().len() != options.len() {
            return Err(Failure::new(
                "invalid_argument",
                "vote requires at least two distinct options",
            ));
        }
        distinct(voters)?;
        let mut ballots = Vec::new();
        for &member in voters {
            match access::check(&self.home, &self.session.id, member) {
                Ok(()) => {}
                Err(e) if e.kind == "budget" => continue,
                Err(e) => return Err(e),
            }
            let ask = Ask {
                member,
                mode: Mode::Plaza,
                session: self.session.id.clone(),
                prompt: format!(
                    "{question}\nOptions: {}\nReply with one option.",
                    options.join(", ")
                ),
                context: self.context(thread)?,
                tools: vec![],
            };
            let reply = provider.ask(&ask)?;
            access::settle(&self.home, &self.session.id, member, &reply)?;
            ballots.push((member, reply.text));
        }
        let mut result = protocol::tally(options, &ballots);
        result.question = question.to_owned();
        self.transcript()?.append(
            "person",
            "vote",
            question,
            Some(thread),
            serde_json::to_value(&result).map_err(|e| Failure::new("io", e.to_string()))?,
        )?;
        Ok(result)
    }
}
