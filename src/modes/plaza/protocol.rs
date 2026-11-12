use crate::modes::providers::Member;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn mentions(text: &str) -> Vec<Member> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for (i, c) in text.char_indices() {
        if c != '@' {
            continue;
        }
        let name: String = text[i + 1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if let Some(member) = Member::parse(&name) {
            if seen.insert(member) {
                found.push(member);
            }
        }
    }
    found
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VoteResult {
    pub question: String,
    pub options: Vec<String>,
    pub ballots: Vec<(Member, String)>,
    pub tally: BTreeMap<String, u32>,
}
pub fn tally(options: &[String], ballots: &[(Member, String)]) -> VoteResult {
    let mut counts: BTreeMap<String, u32> = options.iter().cloned().map(|o| (o, 0)).collect();
    let ballots: Vec<_> = ballots
        .iter()
        .map(|(m, b)| (*m, b.trim().to_owned()))
        .collect();
    for (_, ballot) in &ballots {
        let key = if options.contains(ballot) {
            ballot.as_str()
        } else {
            "invalid"
        };
        *counts.entry(key.to_owned()).or_default() += 1;
    }
    VoteResult {
        question: String::new(),
        options: options.to_vec(),
        ballots,
        tally: counts,
    }
}
