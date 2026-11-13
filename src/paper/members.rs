use crate::{
    paper::{models::PassModels, PassId},
    research_program::investigation::{MemberCall, MemberReply, ResearchMember},
};
use std::sync::atomic::AtomicBool;
pub trait PaperMember: ResearchMember {
    fn call_model(&self, call: &MemberCall, model: &str, stop: &AtomicBool) -> MemberReply;
}
pub struct PaperMembers<'a> {
    pub astra: &'a dyn PaperMember,
    pub fable: &'a dyn PaperMember,
    pub gemini: &'a dyn PaperMember,
    pub reviewer: &'a dyn PaperMember,
}
pub fn expected_model(models: &PassModels, pass: PassId) -> Option<String> {
    match pass {
        PassId::AstraDraft => None,
        _ => Some(models.get(pass).to_owned()),
    }
}
