//! Real UnitActions over orch evidence, review, usage, retry and waive; owned by module engine-views-seam.
use super::session_store;
use crate::{
    base::{self, Failure, Res},
    desktop::ux::seams::unit_actions::{ActionReceipt, UnitActionKind, UnitActions, UnitEvidence},
    orch::{self, Evidence},
    usage::price::{self, CodexUsage},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

static RECEIPTS: Mutex<()> = Mutex::new(());

pub struct OrchUnitActions {
    home: PathBuf,
}
impl OrchUnitActions {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
    fn receipts_path(&self) -> PathBuf {
        self.home.join("desktop").join("unit-actions.json")
    }
    fn read_receipts(&self) -> Res<BTreeMap<String, (bool, String)>> {
        match std::fs::read(self.receipts_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Failure::new("invalid_data", format!("unit-actions.json: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(Failure::new("io", e.to_string())),
        }
    }
    fn remember(&self, receipt: &ActionReceipt) -> Res<()> {
        let mut all = self.read_receipts()?;
        all.insert(
            receipt.key.clone(),
            (receipt.accepted, receipt.message.clone()),
        );
        let path = self.receipts_path();
        std::fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| Failure::new("io", e.to_string()))?;
        let bytes =
            serde_json::to_vec_pretty(&all).map_err(|e| Failure::new("io", e.to_string()))?;
        base::atomic_write(&path, &bytes).map_err(|e| Failure::new("io", e.to_string()))
    }
}

fn require_unit(store: &Path, unit: &str) -> Res<orch::Unit> {
    orch::read_units(store)?
        .into_iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", "Unit not found"))
}
fn refuse(key: &str, message: impl Into<String>) -> ActionReceipt {
    ActionReceipt {
        key: key.into(),
        accepted: false,
        message: message.into(),
    }
}
fn orch_failure(out: &orch::Output) -> String {
    let text = if out.stderr.trim().is_empty() {
        &out.stdout
    } else {
        &out.stderr
    };
    let text = text.trim();
    if text.is_empty() {
        format!("orch exited {}", out.exit_code)
    } else {
        text.to_owned()
    }
}
fn orch_run(store: &Path, args: &[&str]) -> orch::Output {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    orch::run(store, &args, false, false)
}

fn diff_text(store: &Path, unit: &str, rows: &[Evidence]) -> Option<String> {
    let run = rows.iter().rfind(|r| r.kind == "run")?;
    let changed = std::fs::read(store.join("reports").join(format!("{unit}.changed.json")))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default();
    let mut text = format!("commit {}", run.commit);
    for path in changed {
        text.push_str(&format!("\n{path}"));
    }
    Some(text)
}
fn review_text(store: &Path, unit: &str, rows: &[Evidence]) -> Option<String> {
    let row = rows.iter().rfind(|r| r.kind == "review")?;
    let outcome = if row.exit_code == 0 { "pass" } else { "fail" };
    Some(match orch::read_verdict(store, unit) {
        Ok(Some(v)) => {
            let serious = v.findings.iter().filter(|f| f.is_serious()).count();
            let mut text = format!(
                "{outcome}: verdict {}, {} findings, {serious} serious",
                v.verdict,
                v.findings.len()
            );
            for f in v.findings.iter().filter(|f| f.is_serious()) {
                text.push_str(&format!("\n#{} {} {}", f.number, f.check, f.location));
            }
            text
        }
        _ => outcome.to_owned(),
    })
}
fn failure_text(rows: &[Evidence]) -> Option<String> {
    rows.iter()
        .rfind(|r| r.exit_code != 0)
        .map(|r| format!("{}: {} (exit {})", r.kind, r.command, r.exit_code))
}
/// Cost and tokens of the latest run report; None when the report or its prices are unknown.
fn usage_of(store: &Path, unit: &str, rows: &[Evidence]) -> (Option<f64>, Option<u64>) {
    let Some(bytes) = std::fs::read(store.join("reports").join(format!("{unit}.json"))).ok()
    else {
        return (None, None);
    };
    let Ok(report) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, None);
    };
    let usage = &report["usage"];
    let (Some(input), Some(cached), Some(output)) = (
        usage["inputTokens"].as_u64(),
        usage["cachedInputTokens"].as_u64(),
        usage["outputTokens"].as_u64(),
    ) else {
        return (None, None);
    };
    let model = rows
        .iter()
        .rfind(|r| r.kind == "run")
        .map(|r| r.command.as_str());
    let usd = model.and_then(|m| {
        price::codex_usd(
            m,
            &CodexUsage {
                input,
                cached_input: cached,
                output,
            },
        )
    });
    (usd, Some(input + output))
}

fn retry(store: &Path, unit: &orch::Unit, key: &str) -> Res<ActionReceipt> {
    if orch::read_claims(store)?.iter().any(|c| c.unit == unit.id) {
        return Ok(refuse(key, format!("busy: unit {} is claimed", unit.id)));
    }
    if unit.brief.is_empty() {
        return Ok(refuse(
            key,
            format!("unit {} has no brief to retry", unit.id),
        ));
    }
    let existing = orch::read_units(store)?;
    let new = (1..)
        .map(|n| format!("{}-retry{n}", unit.id))
        .find(|id| !existing.iter().any(|u| u.id == *id))
        .unwrap();
    let out = orch_run(
        store,
        &[
            "unit",
            "add",
            &new,
            "--track",
            &unit.track,
            "--brief",
            &unit.brief,
            "--fixes",
            &unit.id,
        ],
    );
    if out.exit_code != 0 {
        return Ok(refuse(key, orch_failure(&out)));
    }
    Ok(ActionReceipt {
        key: key.into(),
        accepted: true,
        message: format!("retry accepted: {new}"),
    })
}

fn waive(
    store: &Path,
    unit: &str,
    reason: &str,
    finding: Option<usize>,
    key: &str,
) -> Res<ActionReceipt> {
    let rows = orch::evidence(store, unit)?;
    let Some(review_at) = rows.iter().rposition(|r| r.kind == "review") else {
        return Ok(refuse(key, "no review to waive"));
    };
    if rows[review_at + 1..].iter().any(|r| r.kind == "run") {
        return Ok(refuse(key, "stale review: the unit ran again after it"));
    }
    let Some(verdict) = orch::read_verdict(store, unit)? else {
        return Ok(refuse(key, "no verdict file to waive"));
    };
    let number = match finding {
        Some(n) => n,
        None => {
            let waived: Vec<usize> = orch::waivers(&rows).iter().map(|w| w.number).collect();
            let open: Vec<usize> = verdict
                .findings
                .iter()
                .filter(|f| f.is_serious() && !waived.contains(&f.number))
                .map(|f| f.number)
                .collect();
            match open.as_slice() {
                [n] => *n,
                _ => return Ok(refuse(key, "choose a finding")),
            }
        }
    };
    let out = orch_run(
        store,
        &["waive", unit, &number.to_string(), "--reason", reason],
    );
    if out.exit_code != 0 {
        return Ok(refuse(key, orch_failure(&out)));
    }
    Ok(ActionReceipt {
        key: key.into(),
        accepted: true,
        message: format!("waive accepted: #{number}"),
    })
}

impl UnitActions for OrchUnitActions {
    fn evidence(&self, session: &str, unit: &str) -> Res<UnitEvidence> {
        let store = session_store(&self.home, session)?;
        require_unit(&store, unit)?;
        let rows = orch::evidence(&store, unit)?;
        let (cost_usd, tokens) = usage_of(&store, unit, &rows);
        Ok(UnitEvidence {
            diff: diff_text(&store, unit, &rows),
            review: review_text(&store, unit, &rows),
            cost_usd,
            tokens,
            failure: failure_text(&rows),
        })
    }
    fn act(
        &self,
        session: &str,
        unit: &str,
        kind: &UnitActionKind,
        key: &str,
    ) -> Res<ActionReceipt> {
        let _guard = RECEIPTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((accepted, message)) = self.read_receipts()?.get(key) {
            return Ok(ActionReceipt {
                key: key.into(),
                accepted: *accepted,
                message: message.clone(),
            });
        }
        let store = session_store(&self.home, session)?;
        let row = require_unit(&store, unit)?;
        let receipt = match kind {
            UnitActionKind::Retry => retry(&store, &row, key)?,
            UnitActionKind::Waive { reason, .. } if reason.trim().is_empty() => {
                return Err(Failure::new("reason_required", "A reason is required"))
            }
            UnitActionKind::Waive { reason, finding } => {
                waive(&store, unit, reason.trim(), *finding, key)?
            }
        };
        self.remember(&receipt)?;
        Ok(receipt)
    }
}
