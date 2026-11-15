use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    /// A workshop, routine, compute project, or research run identifier.
    pub subject: String,
    pub action: String,
    pub summary: String,
    pub digest: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approved,
    Denied,
}

pub trait PersonApproval: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> Decision;
}

pub struct DenyAll;

impl PersonApproval for DenyAll {
    fn decide(&self, _request: &ApprovalRequest) -> Decision {
        Decision::Denied
    }
}

pub struct TtyApproval;

impl PersonApproval for TtyApproval {
    fn decide(&self, request: &ApprovalRequest) -> Decision {
        eprintln!(
            "{}\n{}\n{}",
            request.subject, request.action, request.summary
        );
        let stdin = io::stdin();
        if !stdin.is_terminal() {
            return Decision::Denied;
        }
        let mut answer = String::new();
        if stdin.read_line(&mut answer).is_ok()
            && (answer.trim().eq_ignore_ascii_case("y")
                || answer.trim().eq_ignore_ascii_case("yes"))
        {
            Decision::Approved
        } else {
            Decision::Denied
        }
    }
}

struct FakeApprovalState {
    decisions: VecDeque<Decision>,
    fallback: Decision,
    asked: Vec<ApprovalRequest>,
}

pub struct FakeApproval {
    state: Mutex<FakeApprovalState>,
}

impl FakeApproval {
    pub fn approving() -> Self {
        Self::with_decisions(Vec::new(), Decision::Approved)
    }

    pub fn denying() -> Self {
        Self::with_decisions(Vec::new(), Decision::Denied)
    }

    pub fn scripted(decisions: Vec<Decision>) -> Self {
        Self::with_decisions(decisions, Decision::Denied)
    }

    fn with_decisions(decisions: Vec<Decision>, fallback: Decision) -> Self {
        Self {
            state: Mutex::new(FakeApprovalState {
                decisions: decisions.into(),
                fallback,
                asked: Vec::new(),
            }),
        }
    }

    pub fn asked(&self) -> Vec<ApprovalRequest> {
        self.state.lock().unwrap().asked.clone()
    }
}

impl PersonApproval for FakeApproval {
    fn decide(&self, request: &ApprovalRequest) -> Decision {
        let mut state = self.state.lock().unwrap();
        state.asked.push(request.clone());
        state.decisions.pop_front().unwrap_or(state.fallback)
    }
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
            Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
        }
    }
}

pub struct FakeClock {
    ms: AtomicI64,
}

impl FakeClock {
    pub fn new(ms: i64) -> Self {
        Self {
            ms: AtomicI64::new(ms),
        }
    }

    pub fn set(&self, ms: i64) {
        self.ms.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.ms.load(Ordering::SeqCst)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
