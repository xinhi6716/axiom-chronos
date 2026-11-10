//! Mechanical convergence: stances to claim status, with no judgement and no further model call.
use super::{Claim, ClaimStatus, Council, Stance, Status, MEMBERS};
use serde_json::{json, Value};

const MIN_EVIDENCE_CHARS: usize = 10;
pub const DIGEST_MAX_CHARS: usize = 16_000;
pub(super) const OPEN_MAX: usize = 16;
const AGREED_MAX: usize = 10;

pub(super) fn cut(text: &str, max: usize) -> String {
    let mut chars = text.trim().chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

/// A stance as recorded: `disagree` without checkable evidence counts for nothing.
pub(super) fn stance_of(raw: &str, evidence: &str) -> Option<Stance> {
    match raw {
        "agree" => Some(Stance::Agree),
        "concede" => Some(Stance::Concede),
        "disagree" if evidence.trim().chars().count() >= MIN_EVIDENCE_CHARS => {
            Some(Stance::Disagree)
        }
        "disagree" => Some(Stance::Unsupported),
        _ => None,
    }
}

/// Consensus needs a second affirmative voice: when the others are absent, silence would
/// otherwise turn every claim of the one remaining member into "consensus".
pub(super) fn settle(claim: &mut Claim) {
    let count = |want: &[Stance]| claim.stances.values().filter(|s| want.contains(s)).count();
    let affirmative = count(&[Stance::Author, Stance::Agree, Stance::Concede]);
    let dissent = count(&[Stance::Disagree]);
    claim.status = if dissent == 0 && affirmative >= 2 {
        ClaimStatus::Consensus
    } else if dissent >= 1 && affirmative >= 2 && affirmative > dissent {
        ClaimStatus::Majority
    } else {
        ClaimStatus::Disputed
    };
}

fn counts(council: &Council) -> (usize, usize, usize) {
    let of = |s: ClaimStatus| council.claims.iter().filter(|c| c.status == s).count();
    (
        of(ClaimStatus::Consensus),
        of(ClaimStatus::Majority),
        of(ClaimStatus::Disputed),
    )
}

fn sitting(council: &Council) -> usize {
    council.rounds.first().map_or(0, |r| r.positions.len())
}

/// The line for the Astra stage.
pub fn stage_note(council: &Council) -> String {
    if sitting(council) < 2 {
        return "ordinary wish: no council".to_owned();
    }
    let (agreed, majority, disputed) = counts(council);
    let mut note = format!(
        "{agreed} of {} claims agreed, {} disputed",
        council.claims.len(),
        majority + disputed
    );
    if council.degraded {
        note.push_str(&format!("; {} of 3 members", sitting(council)));
    }
    note
}

fn plain_note(council: &Council) -> String {
    let absent: Vec<String> = MEMBERS
        .iter()
        .filter_map(|name| {
            let member = council.members.iter().find(|m| m.name == *name)?;
            (member.status != Status::Ok).then(|| {
                format!(
                    "{name}: {}{}",
                    member.status.word(),
                    member
                        .detail
                        .as_deref()
                        .map(|d| format!(" ({})", cut(d, 120)))
                        .unwrap_or_default()
                )
            })
        })
        .collect();
    if sitting(council) < 2 {
        return format!(
            "Neither other member could sit, so this is an ordinary wish with Astra alone. {}",
            absent.join("; ")
        );
    }
    if council.degraded {
        return format!(
            "Degraded council: {} of 3 members held a position. {}",
            sitting(council),
            absent.join("; ")
        );
    }
    "All three members completed both rounds.".to_owned()
}

/// The next batch of open claims that still lack a ruling, shaped like the digest's `open`.
pub(super) fn unruled_batch(council: &Council) -> (Vec<Value>, usize) {
    let unruled: Vec<&Claim> = council
        .claims
        .iter()
        .filter(|c| c.status != ClaimStatus::Consensus && c.ruling.is_none())
        .collect();
    let mut entries: Vec<Value> = unruled.iter().take(OPEN_MAX).map(|c| open_entry(c)).collect();
    while json!(entries).to_string().chars().count() > DIGEST_MAX_CHARS / 2 && entries.len() > 1 {
        entries.pop();
    }
    (entries, unruled.len())
}

fn open_entry(claim: &Claim) -> Value {
    json!({
        "id": claim.id,
        "status": claim.status,
        "text": cut(&claim.text, 200),
        "evidence": cut(&claim.evidence, 120),
        "stances": claim.stances,
        "dissent": claim.dissent.iter().take(1).map(|d| cut(&d.evidence, 160)).collect::<Vec<_>>(),
        "ruling": claim.ruling,
    })
}

/// What the core reads. Nothing here is a raw position; every field is cut, and the whole is
/// cut again until it fits.
pub fn digest(council: &Council, path: &std::path::Path, reused: bool) -> Value {
    let (agreed, majority, disputed) = counts(council);
    let open: Vec<&Claim> = council
        .claims
        .iter()
        .filter(|c| c.status != ClaimStatus::Consensus)
        .collect();
    let settled: Vec<&Claim> = council
        .claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Consensus)
        .collect();
    let outcome = if sitting(council) < 2 {
        "ordinary"
    } else if council.degraded {
        "degraded"
    } else {
        "full"
    };
    let mut entries: Vec<Value> = open.iter().take(OPEN_MAX).map(|c| open_entry(c)).collect();
    let mut result = json!({
        "status": "completed",
        "outcome": outcome,
        "reused": reused,
        "note": plain_note(council),
        "stage_note": stage_note(council),
        "degraded": council.degraded,
        "members": council.members.iter().map(|m| json!({"name":m.name,"status":m.status,"tokens":m.tokens,"usd":m.usd})).collect::<Vec<_>>(),
        "cost": council.cost,
        "counts": {"consensus":agreed,"majority":majority,"disputed":disputed,"dropped_at_intake":council.dropped},
        "agreed": settled.iter().take(AGREED_MAX).map(|c| json!({"id":c.id,"text":cut(&c.text,100)})).collect::<Vec<_>>(),
        "open": entries,
        "omitted_open": open.len().saturating_sub(OPEN_MAX),
        "more_open": open.len() > OPEN_MAX,
        "omitted_agreed": settled.len().saturating_sub(AGREED_MAX),
        "council_path": path,
    });
    while result.to_string().chars().count() > DIGEST_MAX_CHARS && !entries.is_empty() {
        entries.pop();
        result["omitted_open"] = json!(open.len() - entries.len());
        result["more_open"] = json!(true);
        result["open"] = json!(entries);
    }
    result
}
