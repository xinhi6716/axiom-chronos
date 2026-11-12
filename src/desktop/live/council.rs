#![allow(dead_code)]
use crate::{
    council::{ClaimStatus, Council, Stance as ClaimStance, Status},
    desktop::ux::celestial::{CouncilView, Dialogue, Member, Seat, Stance},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
};

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
fn read(path: &Path) -> Result<Option<Vec<u8>>, ()> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(());
    }
    Ok(Some(bytes))
}
fn member(s: &str) -> Option<Member> {
    match s.to_ascii_lowercase().as_str() {
        "astra" => Some(Member::Astra),
        "fable" => Some(Member::Fable),
        "argon" => Some(Member::Argon),
        _ => None,
    }
}
fn seat(view: &mut CouncilView, m: Member, value: Seat) {
    if let Some((_, s)) = view.seats.iter_mut().find(|(who, _)| *who == m) {
        *s = value;
    } else {
        view.seats.push((m, value));
    }
}
pub fn read_council(astra_dir: &Path) -> Option<CouncilView> {
    let data = read(&astra_dir.join("council.json")).ok()?;
    let events = read(&astra_dir.join("council-events.jsonl")).ok()?;
    if data.is_none() && events.is_none() {
        return None;
    }
    let mut view = CouncilView::default();
    let completed = data.is_some();
    if let Some(data) = data {
        let c: Council = serde_json::from_slice(&data).ok()?;
        view.spent_usd = Some(c.cost.usd);
        view.cap_usd = Some(c.cost.cap_usd);
        if let Some(r) = c.rounds.last() {
            view.round = u32::try_from(r.n).unwrap_or(u32::MAX);
            view.phase = r.phase.clone().into();
        } else if let Some(p) = c.phases.last() {
            view.phase = p.phase.clone().into();
        }
        for m in &c.members {
            if let Some(who) = member(&m.name) {
                let state = if m.status == Status::Ok {
                    Seat::Done
                } else {
                    Seat::Degraded {
                        status: m.status.word().into(),
                        reason: m.detail.clone().unwrap_or_default(),
                    }
                };
                seat(&mut view, who, state);
            }
        }
        for claim in &c.claims {
            match claim.status {
                ClaimStatus::Consensus => view.agreed = view.agreed.saturating_add(1),
                ClaimStatus::Majority => view.majority = view.majority.saturating_add(1),
                ClaimStatus::Disputed => view.disputed = view.disputed.saturating_add(1),
            }
            if let Some(to) = member(&claim.author) {
                for (name, stance) in &claim.stances {
                    if let Some(from) = member(name) {
                        let stance = match stance {
                            ClaimStance::Agree | ClaimStance::Concede => Some(Stance::Agree),
                            ClaimStance::Disagree => Some(Stance::Dissent),
                            _ => None,
                        };
                        if let Some(stance) = stance {
                            let d = Dialogue { from, to, stance };
                            if from != to && !view.dialogue.contains(&d) {
                                view.dialogue.push(d);
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(bytes) = events {
        let text = std::str::from_utf8(&bytes).ok()?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).ok()?;
            if v["type"] != "council" {
                continue;
            }
            let subtype = v["subtype"].as_str().unwrap_or("");
            if !matches!(subtype, "member_degraded" | "turn_start" | "turn_end") {
                continue;
            }
            let who = member(v["member"].as_str()?)?;
            if let Some(phase) = v["phase"].as_str() {
                view.phase = phase.to_owned().into();
            }
            if let Some(round) = v["round"].as_u64() {
                view.round = u32::try_from(round).unwrap_or(u32::MAX);
            }
            match subtype {
                "member_degraded" => {
                    seat(
                        &mut view,
                        who,
                        Seat::Degraded {
                            status: v["status"].as_str().unwrap_or("").into(),
                            reason: v["reason"].as_str().unwrap_or("").into(),
                        },
                    );
                    if view.speaker == Some(who) {
                        view.speaker = None;
                    }
                }
                "turn_start" => {
                    if let Some(old) = view.speaker {
                        if old != who {
                            seat(&mut view, old, Seat::Done);
                        }
                    }
                    seat(&mut view, who, Seat::Speaking);
                    view.speaker = Some(who);
                }
                "turn_end" => {
                    if !view
                        .seats
                        .iter()
                        .any(|(m, s)| *m == who && matches!(s, Seat::Degraded { .. }))
                    {
                        seat(&mut view, who, Seat::Done);
                    }
                    if view.speaker == Some(who) {
                        view.speaker = None;
                    }
                }
                _ => {}
            }
        }
        view.active = !completed || view.speaker.is_some();
        if !completed {
            for m in [Member::Astra, Member::Fable, Member::Argon] {
                if !view.seats.iter().any(|(who, _)| *who == m) {
                    seat(&mut view, m, Seat::Waiting);
                }
            }
        }
    }
    Some(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    #[test]
    fn reads_complete_degraded_and_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_council(dir.path()).is_none());
        let path = dir.path().join("council-events.jsonl");
        fs::write(&path, format!("{}\n",json!({"type":"council","subtype":"turn_start","member":"fable","phase":"A","round":2}))).unwrap();
        let v = read_council(dir.path()).unwrap();
        assert!(v.active);
        assert_eq!(v.speaker, Some(Member::Fable));
        assert_eq!(v.round, 2);
        fs::write(&path, format!("{}\n",json!({"type":"council","subtype":"member_degraded","member":"fable","phase":"A","round":2,"status":"quota","reason":"limit"}))).unwrap();
        assert!(read_council(dir.path())
            .unwrap()
            .seats
            .iter()
            .any(|(m, s)| *m == Member::Fable && matches!(s, Seat::Degraded { .. })));
        let claim = |id, status| json!({"id":id,"text":"claim","evidence":"e","stances":{"fable":"agree"},"author":"astra","status":status,"ruling":null});
        let full = json!({"key":"k","question":"q","members":[{"name":"astra","model":"m","family":"f","status":"ok","tokens":0,"usd":0.1}],"rounds":[{"phase":"A","n":2,"positions":[]}],"claims":[claim("1","consensus"),claim("2","majority"),claim("3","disputed")],"cost":{"usd":0.1,"cap_usd":5.0,"cut":[]},"degraded":false});
        fs::write(dir.path().join("council.json"), full.to_string()).unwrap();
        let v = read_council(dir.path()).unwrap();
        assert!(!v.active);
        assert_eq!(v.spent_usd, Some(0.1));
        assert_eq!((v.agreed, v.majority, v.disputed), (1, 1, 1));
        assert_eq!(v.dialogue.len(), 1);
        assert!(v
            .seats
            .iter()
            .any(|(m, s)| *m == Member::Fable && matches!(s, Seat::Degraded { .. })));
        fs::write(&path, "broken\n").unwrap();
        assert!(read_council(dir.path()).is_none());
        fs::remove_file(&path).unwrap();
        fs::write(dir.path().join("council.json"), "{").unwrap();
        assert!(read_council(dir.path()).is_none());
    }
}
