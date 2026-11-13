use super::model::{paper_dir, PaperState, Step, StepRecord};
use crate::base::{atomic_write, now_iso, Failure, Res};
use std::{collections::BTreeMap, fs, path::Path};

pub fn load(store: &Path) -> Res<Option<PaperState>> {
    let path = paper_dir(store).join("state.json");
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Failure::new("json", e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}
pub fn save(store: &Path, state: &PaperState) -> Res<()> {
    let dir = paper_dir(store);
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|e| Failure::new("json", e.to_string()))?;
    atomic_write(&dir.join("state.json"), &bytes).map_err(|e| Failure::new("io", e.to_string()))
}
pub fn upstream(step: Step) -> &'static [Step] {
    use Step::*;
    match step {
        Evidence | Shortlist => &[],
        Journal => &[Shortlist],
        Pass1 => &[Evidence, Journal],
        Pass2 => &[Pass1],
        Pass3 => &[Pass2],
        Pass4 => &[Pass3],
        Figures => &[Pass4, Evidence],
        Citations => &[Pass4],
        Numbers => &[Pass4, Evidence, Figures],
        Compile => &[Pass4, Journal, Figures, Citations],
        Render => &[Compile],
        Disclosure => &[Journal, Pass4],
        Review => &[Compile, Render, Citations, Numbers, Disclosure],
        Ready => &[
            Evidence, Shortlist, Journal, Pass1, Pass2, Pass3, Pass4, Figures, Citations, Numbers,
            Compile, Render, Disclosure, Review,
        ],
    }
}
const ALL: [Step; 15] = [
    Step::Evidence,
    Step::Shortlist,
    Step::Journal,
    Step::Pass1,
    Step::Pass2,
    Step::Pass3,
    Step::Pass4,
    Step::Figures,
    Step::Citations,
    Step::Numbers,
    Step::Compile,
    Step::Render,
    Step::Disclosure,
    Step::Review,
    Step::Ready,
];

fn key(step: Step) -> String {
    serde_json::to_value(step)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn empty() -> PaperState {
    PaperState {
        version: 1,
        run_id: crate::base::unique(),
        steps: BTreeMap::new(),
    }
}

pub fn record(
    store: &Path,
    step: Step,
    inputs: BTreeMap<String, String>,
    output: &str,
    ok: bool,
    detail: Option<String>,
) -> Res<PaperState> {
    let mut state = load(store)?.unwrap_or_else(empty);
    state.steps.insert(
        step,
        StepRecord {
            step,
            inputs,
            output: output.into(),
            ok,
            detail,
            at: now_iso(),
        },
    );
    let mut changed = vec![(step, Some(output.to_owned()))];
    while let Some((parent, hash)) = changed.pop() {
        for child in ALL {
            if !upstream(child).contains(&parent) {
                continue;
            }
            let stale = state.steps.get(&child).is_some_and(|r| {
                hash.as_ref()
                    .is_none_or(|h| r.inputs.get(&key(parent)) != Some(h))
            });
            if stale {
                state.steps.remove(&child);
                changed.push((child, None));
            }
        }
    }
    save(store, &state)?;
    Ok(state)
}
pub fn current(state: &PaperState, step: Step) -> bool {
    let Some(record) = state.steps.get(&step) else {
        return false;
    };
    record.ok
        && upstream(step).iter().all(|&parent| {
            current(state, parent)
                && state
                    .steps
                    .get(&parent)
                    .is_some_and(|r| record.inputs.get(&key(parent)) == Some(&r.output))
        })
}
pub fn invalidate(store: &Path, step: Step) -> Res<PaperState> {
    let mut state = load(store)?.unwrap_or_else(empty);
    let mut pending = vec![step];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(parent) = pending.pop() {
        if !seen.insert(parent) {
            continue;
        }
        state.steps.remove(&parent);
        pending.extend(
            ALL.into_iter()
                .filter(|&child| upstream(child).contains(&parent)),
        );
    }
    save(store, &state)?;
    Ok(state)
}
