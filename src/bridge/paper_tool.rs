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
        let reply = cross(
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
        let reply = map_raw_output(reply);
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

fn map_raw_output(mut reply: council::CrossReply) -> council::CrossReply {
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
    reply
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{base::Failure, bridge::Config, engine::policy, orch::events};
    use serde_json::json;
    use tempfile::tempdir;

    fn call(path: &Path) -> MemberCall {
        MemberCall {
            member: "paper".into(),
            stage: "test".into(),
            prompt: "test".into(),
            readable: vec![],
            out_dir: path.into(),
            schema: json!({}),
            timeout_s: 1,
        }
    }

    #[test]
    fn cross_members_enforce_their_own_engine_budgets_before_cross() {
        let dir = tempdir().unwrap();
        let bridge = Bridge::new(Config {
            data_home: Some(dir.path().into()),
            codex: Err(Failure::new("unavailable", "codex unavailable")),
            wsl: None,
            on_windows: false,
        });
        let setup = council::Setup::new(Ok(vec![]), Ok(vec![]));
        let members = ProductionMembers::new(&bridge, &setup, dir.path(), dir.path());
        let stop = AtomicBool::new(false);
        let request = call(dir.path());
        for (member, role) in [
            (&members.astra, Role::CouncilMember),
            (&members.reviewer, Role::Reviewer),
        ] {
            policy::set(dir.path(), &format!("budget.{}", role.name()), "0").unwrap();
            let reply = member.call(&request, &stop);
            assert_eq!(reply.status, Status::Failed);
            let detail: Value = serde_json::from_str(reply.detail.as_ref().unwrap()).unwrap();
            assert_eq!(detail["kind"], "budget_exceeded");
            assert!(reply.raw.is_none());
            let recorded = events::read_events(dir.path(), 0).unwrap();
            let event = recorded.last().unwrap();
            assert_eq!(event.data["role"], role.name());
            assert_eq!(event.data["call"], "paper");
            assert!(!recorded
                .iter()
                .any(|e| e.data["call"] == "paper" && e.kind == crate::engine::kinds::ROLE_SPEND));
        }
    }

    #[test]
    fn unavailable_codex_fails_for_both_cross_members() {
        let dir = tempdir().unwrap();
        let bridge = Bridge::new(Config {
            data_home: Some(dir.path().into()),
            codex: Err(Failure::new("unavailable", "codex unavailable")),
            wsl: None,
            on_windows: false,
        });
        let setup = council::Setup::new(Ok(vec![]), Ok(vec![]));
        let members = ProductionMembers::new(&bridge, &setup, dir.path(), dir.path());
        for member in [&members.astra, &members.reviewer] {
            let reply = member.call(&call(dir.path()), &AtomicBool::new(false));
            assert_eq!(reply.status, Status::Failed);
            assert!(reply.detail.unwrap().contains("codex unavailable"));
        }
    }

    #[test]
    fn raw_response_json_mapping() {
        let make = |raw: &str| council::CrossReply {
            status: Status::Ok,
            output: None,
            raw: Some(raw.into()),
            tokens: 0,
            model: None,
            detail: None,
        };
        let valid = map_raw_output(make("{\"result\":42}"));
        assert_eq!(valid.status, Status::Ok);
        assert_eq!(valid.output, Some(json!({"result":42})));
        let invalid = map_raw_output(make("not json"));
        assert_eq!(invalid.status, Status::Malformed);
        assert!(invalid.output.is_none());
        assert!(invalid.detail.is_some());
    }
}
