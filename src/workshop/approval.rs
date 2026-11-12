use super::{permissions::Grants, DataDirs, Manifest};
use crate::base::{self, Failure, Res};
use crate::seams::{ApprovalRequest, Decision, PersonApproval};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ApprovalReceipt {
    pub module: String,
    pub high_risk: Vec<String>,
    pub digest: String,
    pub approved_at: String,
}

pub fn receipt_path(dirs: &DataDirs) -> PathBuf {
    dirs.root.join("approval.json")
}

pub fn ensure_approved(
    dirs: &DataDirs,
    manifest: &Manifest,
    grants: &Grants,
    approval: &dyn PersonApproval,
) -> Res<Option<ApprovalReceipt>> {
    let mut high_risk: Vec<_> = grants.high_risk().iter().map(|p| p.as_string()).collect();
    high_risk.sort();
    if high_risk.is_empty() {
        return Ok(None);
    }
    let path = receipt_path(dirs);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(receipt) = serde_json::from_slice::<ApprovalReceipt>(&bytes) {
            if receipt.module == manifest.name
                && high_risk.iter().all(|p| receipt.high_risk.contains(p))
            {
                return Ok(Some(receipt));
            }
        }
    }
    let digest = grants.digest();
    let request = ApprovalRequest {
        subject: format!("workshop:{}", manifest.name),
        action: "grant_permissions".into(),
        summary: high_risk.join(", "),
        digest: digest.clone(),
    };
    if approval.decide(&request) != Decision::Approved {
        return Err(Failure::new(
            "approval_denied",
            format!("approval denied for {}", manifest.name),
        ));
    }
    let receipt = ApprovalReceipt {
        module: manifest.name.clone(),
        high_risk,
        digest,
        approved_at: base::now_iso(),
    };
    std::fs::create_dir_all(&dirs.root).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes =
        serde_json::to_vec_pretty(&receipt).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&path, &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(Some(receipt))
}
