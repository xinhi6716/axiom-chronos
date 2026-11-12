use super::{council_tool::cross, Bridge};
use crate::{
    council::{
        self,
        paper_member::{PaperArgon, PaperFable},
        CrossRequest, Status,
    },
    engine::budgets::{self, Role},
    paper::{members::PaperMembers, paper_dir},
    research_program::investigation::{MemberCall, MemberReply, ResearchMember},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

struct CrossMember<'a> {
    name: &'static str,
    role: Role,
    bridge: &'a Bridge,
    setup: &'a council::Setup,
    store: PathBuf,
    cwd: PathBuf,
}

impl ResearchMember for CrossMember<'_> {
    fn name(&self) -> &str {
        self.name
    }

    fn call(&self, call: &MemberCall, _stop: &AtomicBool) -> MemberReply {
        if let Err(e) = budgets::check(&self.store, self.role, "paper") {
            return MemberReply {
                status: Status::Failed,
                output: None,
                raw: None,
                model: None,
                provider: self.name.into(),
                usd: None,
                tokens: 0,
                detail: Some(e.to_json().to_string()),
            };
        }
        let mut reply = cross(
            self.bridge,
            &self.cwd.to_string_lossy(),
            &CrossRequest {
                prompt: format!(
                    "{}\nReturn only JSON matching this schema:\n{}",
                    call.prompt, call.schema
                ),
                schema: call.schema.clone(),
                timeout_s: call.timeout_s,
                token_cap: self.setup.astra_token_cap,
            },
        );
        let _ = budgets::record(
            &self.store,
            self.role,
            reply.model.as_deref().unwrap_or(self.name),
            None,
            0,
            reply.tokens,
            "paper",
        );
        if reply.status == Status::Ok && reply.output.is_none() {
            match reply.raw.as_deref().map(serde_json::from_str::<Value>) {
                Some(Ok(value)) => reply.output = Some(value),
                Some(Err(e)) => {
                    reply.status = Status::Malformed;
                    reply.detail = Some(e.to_string());
                }
                None => {
                    reply.status = Status::Malformed;
                    reply.detail = Some("missing output".into());
                }
            }
        }
        MemberReply {
            status: reply.status,
            output: reply.output,
            raw: reply.raw,
            model: reply.model,
            provider: self.name.into(),
            usd: None,
            tokens: reply.tokens,
            detail: reply.detail,
        }
    }
}

pub struct ProductionMembers<'a> {
    astra: CrossMember<'a>,
    fable: PaperFable,
    argon: PaperArgon,
    reviewer: CrossMember<'a>,
}

impl<'a> ProductionMembers<'a> {
    pub fn new(bridge: &'a Bridge, setup: &'a council::Setup, store: &Path, cwd: &Path) -> Self {
        let member = |name, role| CrossMember {
            name,
            role,
            bridge,
            setup,
            store: store.to_owned(),
            cwd: cwd.to_owned(),
        };
        Self {
            astra: member("astra", Role::CouncilMember),
            fable: PaperFable::new(
                setup.fable.clone(),
                paper_dir(store).join("fable-meter.json"),
                setup.fable_cap_usd,
            ),
            argon: PaperArgon::new(setup.argon.clone()),
            reviewer: member("reviewer", Role::Reviewer),
        }
    }

    pub fn members(&self) -> PaperMembers<'_> {
        PaperMembers {
            astra: &self.astra,
            fable: &self.fable,
            argon: &self.argon,
            reviewer: &self.reviewer,
        }
    }
}
