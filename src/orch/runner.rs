//! Wire contracts and pure validation for an optional prepared Rust runner.
//!
//! A successful receipt proves only that the recorded source compiled and
//! linked under the recorded command policy. It does not prove that the runner
//! works, that it has independent cases, or that compiler macros cannot read
//! files outside the scratch directory.

use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::checkpoint::ArtifactRef;

pub const RUNNER_VERSION: u32 = 1;
pub const MAX_RUNNER_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_RUNNER_BINARY_BYTES: usize = 64 * 1024 * 1024;
const RESERVED_PREFIX: &str = "@axiom:rust:";
const SENTINEL_PREFIX: &str = "@axiom:rust:v1:";

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("evaluation_unprepared", message)
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn reference_ok(value: &ArtifactRef) -> bool {
    value.path.is_absolute() && digest_ok(&value.digest)
}

fn valid_identity_field(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.contains('\0')
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerIdentity {
    pub run_id: String,
    pub epoch: usize,
    pub attempt_id: String,
    pub generation: u64,
    pub thread_id: String,
    pub turn_id: String,
}

impl RunnerIdentity {
    pub fn validate(&self) -> Res<()> {
        base::require_slug("run_id", &self.run_id)
            .and_then(|()| base::require_slug("attempt_id", &self.attempt_id))
            .map_err(|_| invalid("invalid runner attempt identity"))?;
        if !valid_identity_field(&self.thread_id) || !valid_identity_field(&self.turn_id) {
            return Err(invalid("invalid runner thread or turn identity"));
        }
        Ok(())
    }
}

/// Persist this claim before invoking the compiler. A claim without a terminal
/// receipt is an unknown outcome and must not be compiled again automatically.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerClaim {
    pub version: u32,
    pub identity: RunnerIdentity,
    pub source: ArtifactRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompilerIdentity {
    pub executable: std::path::PathBuf,
    pub canonical_executable: std::path::PathBuf,
    pub executable_sha256: String,
    pub version_verbose: String,
    pub host_triple: String,
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Ready,
    Failed,
    TimedOut,
    Unavailable,
    InvalidOutput,
    CompilerChanged,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerPhase {
    Resolve,
    Probe,
    Compile,
}

/// The caller owns artifact storage and must retain this receipt and the raw
/// output even for terminal failures. Never expose these artifacts to writers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerReceipt {
    pub version: u32,
    pub identity: RunnerIdentity,
    /// SHA-256 of the exact persisted claim bytes, not a reconstructed object.
    pub claim_digest: String,
    pub source: ArtifactRef,
    pub binary: Option<ArtifactRef>,
    pub compiler: Option<CompilerIdentity>,
    /// Exact version-probe stdout captured before compile and saved by caller.
    pub probe_stdout: Option<ArtifactRef>,
    pub status: RunnerStatus,
    pub phase: RunnerPhase,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub policy_applied: bool,
    pub stdout: Option<ArtifactRef>,
    pub stderr: Option<ArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerSentinel {
    pub source_digest: String,
    pub binary_digest: String,
}

impl RunnerSentinel {
    pub fn program(&self) -> Res<String> {
        if !digest_ok(&self.source_digest) || !digest_ok(&self.binary_digest) {
            return Err(invalid("invalid prepared runner digest"));
        }
        Ok(format!(
            "{SENTINEL_PREFIX}{}:{}",
            self.source_digest, self.binary_digest
        ))
    }
}

/// Ordinary program names return None; the reserved namespace fails closed.
pub fn parse_sentinel(program: &str) -> Res<Option<RunnerSentinel>> {
    if !program.starts_with(RESERVED_PREFIX) {
        return Ok(None);
    }
    let rest = program
        .strip_prefix(SENTINEL_PREFIX)
        .ok_or_else(|| invalid("unsupported prepared runner sentinel"))?;
    let (source, binary) = rest
        .split_once(':')
        .ok_or_else(|| invalid("malformed prepared runner sentinel"))?;
    if !digest_ok(source) || !digest_ok(binary) {
        return Err(invalid("malformed prepared runner sentinel digest"));
    }
    Ok(Some(RunnerSentinel {
        source_digest: source.into(),
        binary_digest: binary.into(),
    }))
}

/// Check the exact serialized claim and source before invoking the compiler.
/// The caller is responsible for durably recording the claim first.
pub fn validate_claim(
    claim_bytes: &[u8],
    expected_identity: &RunnerIdentity,
    source: &[u8],
) -> Res<RunnerClaim> {
    expected_identity.validate()?;
    let claim: RunnerClaim = serde_json::from_slice(claim_bytes)
        .map_err(|_| invalid("malformed prepared runner claim"))?;
    if claim.version != RUNNER_VERSION
        || &claim.identity != expected_identity
        || !reference_ok(&claim.source)
        || source.is_empty()
        || source.len() > MAX_RUNNER_SOURCE_BYTES
        || std::str::from_utf8(source).is_err()
        || sha256(source) != claim.source.digest
    {
        return Err(invalid("invalid prepared runner claim or source"));
    }
    Ok(claim)
}

/// Extract the unique host line from the actual bounded version-probe bytes.
pub fn host_from_probe(bytes: &[u8]) -> Res<&str> {
    if bytes.is_empty() || bytes.len() > 16 * 1024 || bytes.contains(&0) {
        return Err(invalid("invalid compiler probe output"));
    }
    let version = std::str::from_utf8(bytes).map_err(|_| invalid("compiler probe is not UTF-8"))?;
    let mut hosts = version
        .lines()
        .filter_map(|line| line.strip_prefix("host: "));
    let host = hosts
        .next()
        .filter(|host| valid_identity_field(host))
        .ok_or_else(|| invalid("compiler probe has no host"))?;
    if hosts.next().is_some() {
        return Err(invalid("compiler probe has ambiguous host"));
    }
    Ok(host)
}

/// Validate receipt identity, terminal state and exact persisted claim bytes.
/// The caller must also establish that these artifacts belong to this run and
/// that the claim was committed before compilation; hashes alone cannot do so.
pub fn validate_receipt(
    claim_bytes: &[u8],
    receipt: &RunnerReceipt,
    expected_identity: &RunnerIdentity,
    probe_stdout_bytes: Option<&[u8]>,
) -> Res<RunnerClaim> {
    expected_identity.validate()?;
    let claim: RunnerClaim = serde_json::from_slice(claim_bytes)
        .map_err(|_| invalid("malformed prepared runner claim"))?;
    if claim.version != RUNNER_VERSION
        || receipt.version != RUNNER_VERSION
        || &claim.identity != expected_identity
        || &receipt.identity != expected_identity
        || !reference_ok(&claim.source)
        || receipt.source != claim.source
        || !digest_ok(&receipt.claim_digest)
        || sha256(claim_bytes) != receipt.claim_digest
    {
        return Err(invalid("prepared runner claim or identity mismatch"));
    }
    if receipt.binary.as_ref().is_some_and(|r| !reference_ok(r))
        || receipt.stdout.as_ref().is_some_and(|r| !reference_ok(r))
        || receipt.stderr.as_ref().is_some_and(|r| !reference_ok(r))
        || receipt
            .probe_stdout
            .as_ref()
            .is_some_and(|r| !reference_ok(r))
    {
        return Err(invalid("invalid prepared runner artifact reference"));
    }
    if let Some(compiler) = &receipt.compiler {
        let probe = receipt
            .probe_stdout
            .as_ref()
            .ok_or_else(|| invalid("missing compiler probe artifact"))?;
        let probe_bytes =
            probe_stdout_bytes.ok_or_else(|| invalid("missing compiler probe bytes"))?;
        if !compiler.executable.is_absolute()
            || !compiler.canonical_executable.is_absolute()
            || !digest_ok(&compiler.executable_sha256)
            || !valid_identity_field(&compiler.host_triple)
            || !valid_identity_field(&compiler.os)
            || !valid_identity_field(&compiler.arch)
            || compiler.version_verbose.is_empty()
            || compiler.version_verbose.len() > 16 * 1024
            || compiler.version_verbose.as_bytes() != probe_bytes
            || sha256(probe_bytes) != probe.digest
            || host_from_probe(probe_bytes)? != compiler.host_triple
        {
            return Err(invalid("invalid prepared runner compiler identity"));
        }
    } else if receipt.probe_stdout.is_some() || probe_stdout_bytes.is_some() {
        return Err(invalid("unexpected prepared runner probe artifact"));
    }
    let have_logs = receipt.stdout.is_some() && receipt.stderr.is_some();
    let compiler_for_phase = match receipt.phase {
        RunnerPhase::Compile => receipt.compiler.is_some(),
        RunnerPhase::Resolve | RunnerPhase::Probe => receipt.compiler.is_none(),
    };
    let state_ok = match receipt.status {
        RunnerStatus::Ready => {
            receipt.phase == RunnerPhase::Compile
                && receipt.binary.is_some()
                && receipt.compiler.is_some()
                && receipt.exit_code == Some(0)
                && !receipt.timed_out
                && receipt.policy_applied
                && have_logs
        }
        RunnerStatus::Failed => {
            matches!(receipt.phase, RunnerPhase::Probe | RunnerPhase::Compile)
                && compiler_for_phase
                && receipt.binary.is_none()
                && receipt.exit_code.is_some_and(|x| x != 0)
                && !receipt.timed_out
                && receipt.policy_applied
                && have_logs
        }
        RunnerStatus::TimedOut => {
            matches!(receipt.phase, RunnerPhase::Probe | RunnerPhase::Compile)
                && compiler_for_phase
                && receipt.binary.is_none()
                && receipt.timed_out
                && receipt.policy_applied
                && have_logs
        }
        RunnerStatus::Unavailable => {
            receipt.phase == RunnerPhase::Resolve
                && receipt.binary.is_none()
                && receipt.compiler.is_none()
                && receipt.exit_code.is_none()
                && !receipt.timed_out
                && !receipt.policy_applied
                && receipt.stdout.is_none()
                && receipt.stderr.is_none()
        }
        RunnerStatus::InvalidOutput => {
            matches!(receipt.phase, RunnerPhase::Probe | RunnerPhase::Compile)
                && compiler_for_phase
                && receipt.binary.is_none()
                && receipt.exit_code == Some(0)
                && !receipt.timed_out
                && receipt.policy_applied
                && have_logs
        }
        RunnerStatus::CompilerChanged => {
            matches!(receipt.phase, RunnerPhase::Probe | RunnerPhase::Compile)
                && compiler_for_phase
                && receipt.binary.is_none()
                && receipt.policy_applied
                && have_logs
        }
    };
    if !state_ok {
        return Err(invalid("inconsistent prepared runner result"));
    }
    Ok(claim)
}

/// Verify the bytes to materialize, not merely their artifact path or metadata.
/// `observed_probe_stdout` must come from a fresh sandboxed rustc probe.
pub fn verify_material(
    receipt: &RunnerReceipt,
    sentinel: &RunnerSentinel,
    source: &[u8],
    binary: &[u8],
    observed_probe_stdout: &[u8],
) -> Res<()> {
    let compiler = receipt
        .compiler
        .as_ref()
        .ok_or_else(|| invalid("missing compiler identity"))?;
    let binary_ref = receipt
        .binary
        .as_ref()
        .ok_or_else(|| invalid("missing prepared binary"))?;
    if receipt.status != RunnerStatus::Ready
        || source.is_empty()
        || source.len() > MAX_RUNNER_SOURCE_BYTES
        || std::str::from_utf8(source).is_err()
        || binary.is_empty()
        || binary.len() > MAX_RUNNER_BINARY_BYTES
        || !digest_ok(&sentinel.source_digest)
        || !digest_ok(&sentinel.binary_digest)
        || sha256(source) != receipt.source.digest
        || sha256(binary) != binary_ref.digest
        || sentinel.source_digest != receipt.source.digest
        || sentinel.binary_digest != binary_ref.digest
        || compiler.host_triple != host_from_probe(observed_probe_stdout)?
        || compiler.os != std::env::consts::OS
        || compiler.arch != std::env::consts::ARCH
    {
        return Err(invalid("prepared runner material or host mismatch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    const PROBE: &[u8] = b"rustc test\nhost: test-target\n";

    fn checked(
        claim: &[u8],
        receipt: &RunnerReceipt,
        identity: &RunnerIdentity,
    ) -> Res<RunnerClaim> {
        super::validate_receipt(
            claim,
            receipt,
            identity,
            receipt.compiler.as_ref().map(|_| PROBE),
        )
    }

    fn artifact(dir: &Path, name: &str, bytes: &[u8]) -> ArtifactRef {
        ArtifactRef {
            path: dir.join(name),
            digest: sha256(bytes),
        }
    }

    fn fixture(dir: &Path) -> (Vec<u8>, RunnerIdentity, RunnerReceipt, RunnerSentinel) {
        let source = b"fn main() {}";
        let binary = b"binary bytes";
        let identity = RunnerIdentity {
            run_id: "run-1".into(),
            epoch: 2,
            attempt_id: "attempt-1".into(),
            generation: 3,
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
        };
        let claim = RunnerClaim {
            version: RUNNER_VERSION,
            identity: identity.clone(),
            source: artifact(dir, "source", source),
        };
        let claim_bytes = serde_json::to_vec(&claim).unwrap();
        let receipt = RunnerReceipt {
            version: RUNNER_VERSION,
            identity: identity.clone(),
            claim_digest: sha256(&claim_bytes),
            source: claim.source.clone(),
            binary: Some(artifact(dir, "binary", binary)),
            compiler: Some(CompilerIdentity {
                executable: dir.join("rustc"),
                canonical_executable: dir.join("rustc-real"),
                executable_sha256: sha256(b"rustc executable"),
                version_verbose: "rustc test\nhost: test-target\n".into(),
                host_triple: "test-target".into(),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            }),
            probe_stdout: Some(artifact(dir, "probe-stdout", PROBE)),
            status: RunnerStatus::Ready,
            phase: RunnerPhase::Compile,
            exit_code: Some(0),
            timed_out: false,
            policy_applied: true,
            stdout: Some(artifact(dir, "stdout", b"")),
            stderr: Some(artifact(dir, "stderr", b"")),
        };
        let sentinel = RunnerSentinel {
            source_digest: claim.source.digest,
            binary_digest: receipt.binary.as_ref().unwrap().digest.clone(),
        };
        (claim_bytes, identity, receipt, sentinel)
    }

    #[test]
    fn sentinel_is_strict_and_generic_programs_are_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (_, _, _, sentinel) = fixture(dir.path());
        let program = sentinel.program().unwrap();
        assert_eq!(parse_sentinel(&program).unwrap(), Some(sentinel));
        assert_eq!(parse_sentinel("python").unwrap(), None);
        assert_eq!(parse_sentinel("rustc").unwrap(), None);
        for bad in [
            "@axiom:rust:v2:a:b",
            "@axiom:rust:v1:a:b",
            "@axiom:rust:v1:",
            "@axiom:rust:garbage",
        ] {
            assert!(parse_sentinel(bad).is_err(), "{bad}");
        }
        assert!(parse_sentinel(
            &program
                .to_uppercase()
                .replace("@AXIOM:RUST:", "@axiom:rust:")
                .replace("V1:", "v1:")
        )
        .is_err());
        assert!(parse_sentinel(&(program + ":extra")).is_err());
    }

    #[test]
    fn claim_receipt_and_material_are_bound_to_identity_and_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (claim, identity, receipt, sentinel) = fixture(dir.path());
        assert!(validate_claim(&claim, &identity, b"fn main() {}").is_ok());
        assert!(checked(&claim, &receipt, &identity).is_ok());
        assert!(
            verify_material(&receipt, &sentinel, b"fn main() {}", b"binary bytes", PROBE).is_ok()
        );
        assert!(validate_claim(&claim, &identity, b"fn main() { }").is_err());
        assert!(verify_material(&receipt, &sentinel, b"fn main() {}", b"modified", PROBE).is_err());
        assert!(verify_material(
            &receipt,
            &sentinel,
            b"fn main() {}",
            b"binary bytes",
            b"rustc test\nhost: other-target\n"
        )
        .is_err());

        let mut modified = identity.clone();
        modified.run_id.push('x');
        assert!(checked(&claim, &receipt, &modified).is_err());
        modified = identity.clone();
        modified.epoch += 1;
        assert!(checked(&claim, &receipt, &modified).is_err());
        modified = identity.clone();
        modified.attempt_id.push('x');
        assert!(checked(&claim, &receipt, &modified).is_err());
        modified = identity.clone();
        modified.generation += 1;
        assert!(checked(&claim, &receipt, &modified).is_err());
        modified = identity.clone();
        modified.thread_id.push('x');
        assert!(checked(&claim, &receipt, &modified).is_err());
        modified = identity.clone();
        modified.turn_id.push('x');
        assert!(checked(&claim, &receipt, &modified).is_err());

        let mut changed_claim = claim.clone();
        changed_claim.push(b' ');
        assert!(checked(&changed_claim, &receipt, &identity).is_err());
        let mut changed_receipt = receipt.clone();
        changed_receipt.binary.as_mut().unwrap().digest = "0".repeat(64);
        assert!(verify_material(
            &changed_receipt,
            &sentinel,
            b"fn main() {}",
            b"binary bytes",
            PROBE
        )
        .is_err());
        changed_receipt = receipt.clone();
        changed_receipt.policy_applied = false;
        assert!(checked(&claim, &changed_receipt, &identity).is_err());
        changed_receipt = receipt.clone();
        changed_receipt.timed_out = true;
        assert!(checked(&claim, &changed_receipt, &identity).is_err());
        changed_receipt = receipt.clone();
        changed_receipt.compiler.as_mut().unwrap().arch = "wrong".into();
        assert!(verify_material(
            &changed_receipt,
            &sentinel,
            b"fn main() {}",
            b"binary bytes",
            PROBE
        )
        .is_err());
    }

    #[test]
    fn terminal_states_cannot_masquerade_as_ready() {
        let dir = tempfile::tempdir().unwrap();
        let (claim, identity, receipt, _) = fixture(dir.path());
        let mut failed = receipt.clone();
        failed.status = RunnerStatus::Failed;
        failed.binary = None;
        failed.exit_code = Some(1);
        assert!(checked(&claim, &failed, &identity).is_ok());
        failed.status = RunnerStatus::Ready;
        assert!(checked(&claim, &failed, &identity).is_err());
        failed.status = RunnerStatus::TimedOut;
        failed.timed_out = true;
        failed.exit_code = None;
        assert!(checked(&claim, &failed, &identity).is_ok());
        failed.status = RunnerStatus::Unavailable;
        failed.phase = RunnerPhase::Resolve;
        failed.compiler = None;
        failed.probe_stdout = None;
        failed.timed_out = false;
        failed.policy_applied = false;
        failed.stdout = None;
        failed.stderr = None;
        assert!(checked(&claim, &failed, &identity).is_ok());
        failed.status = RunnerStatus::Ready;
        assert!(checked(&claim, &failed, &identity).is_err());
        let mut unknown: serde_json::Value = serde_json::from_slice(&claim).unwrap();
        unknown["unexpected"] = true.into();
        let unknown = serde_json::to_vec(&unknown).unwrap();
        assert!(validate_claim(&unknown, &identity, b"fn main() {}").is_err());
    }

    #[test]
    fn compiler_host_must_match_the_exact_recorded_probe() {
        let dir = tempfile::tempdir().unwrap();
        let (claim, identity, receipt, _) = fixture(dir.path());
        assert!(super::validate_receipt(&claim, &receipt, &identity, Some(PROBE)).is_ok());
        assert!(super::validate_receipt(&claim, &receipt, &identity, None).is_err());
        assert!(super::validate_receipt(
            &claim,
            &receipt,
            &identity,
            Some(b"rustc test\nhost: forged\n")
        )
        .is_err());
        let mut changed = receipt.clone();
        changed.compiler.as_mut().unwrap().host_triple = "forged".into();
        assert!(super::validate_receipt(&claim, &changed, &identity, Some(PROBE)).is_err());
        changed = receipt.clone();
        changed.probe_stdout.as_mut().unwrap().digest = sha256(b"different probe");
        assert!(super::validate_receipt(&claim, &changed, &identity, Some(PROBE)).is_err());
        assert!(host_from_probe(b"host: one\nhost: two\n").is_err());
        assert!(host_from_probe(b"host: test\n\0").is_err());
    }
}
