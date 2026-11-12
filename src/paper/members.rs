use crate::research_program::investigation::ResearchMember;

pub struct PaperMembers<'a> {
    pub astra: &'a dyn ResearchMember,
    pub fable: &'a dyn ResearchMember,
    pub argon: &'a dyn ResearchMember,
    pub reviewer: &'a dyn ResearchMember,
}

pub fn expected_model(member: &str) -> Option<&'static str> {
    match member {
        "fable" => Some(crate::council::FABLE_MODEL),
        "argon" => Some(crate::council::ARGON_MODEL),
        _ => None,
    }
}
