//! Fixed-command compilation and materialization of private Rust runners.
//!
//! `exec_check` restricts writes to scratch and disables network. It is not an
//! OS read whitelist: for example, rustc's include macros may read host files.
//! Compilation is not a product test and no runner is executed here.

use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    base::{Failure, Res},
    codex::{
        transport::{CommandResult, CommandSpec},
        AppServer,
    },
    orch::runner::{
        self, CompilerIdentity, RunnerIdentity, RunnerPhase, RunnerReceipt, RunnerSentinel,
        RunnerStatus, MAX_RUNNER_BINARY_BYTES,
    },
};

/// One version probe and at most one compile/link command per claimed attempt.
pub const RUNNER_TIMEOUT_S: u64 = 120;
const MAX_VERSION_BYTES: u64 = 16 * 1024;
const MAX_COMPILER_BYTES: u64 = 512 * 1024 * 1024;

fn blocked(message: impl Into<String>) -> Failure {
    Failure::new("evaluation_unprepared", message)
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

#[cfg(windows)]
const RUSTC_NAME: &str = "rustc.exe";
#[cfg(not(windows))]
const RUSTC_NAME: &str = "rustc";
#[cfg(windows)]
const RUNNER_NAME: &str = "runner.exe";
#[cfg(not(windows))]
const RUNNER_NAME: &str = "runner";

#[derive(Clone, Debug)]
struct CompilerSelection {
    executable: PathBuf,
    canonical_executable: PathBuf,
    sha256: String,
    safe_path: String,
}

fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn scratch_root(repo: &Path, scratch: &Path) -> Res<PathBuf> {
    if !repo.is_absolute() || !scratch.is_absolute() {
        return Err(blocked("runner roots must be absolute"));
    }
    let meta = fs::symlink_metadata(scratch).map_err(io)?;
    if is_link(&meta) || !meta.is_dir() {
        return Err(blocked("runner scratch root is not an ordinary directory"));
    }
    let root = fs::canonicalize(scratch).map_err(io)?;
    let repo = fs::canonicalize(repo).map_err(io)?;
    if root.starts_with(&repo) || repo.starts_with(&root) {
        return Err(blocked("runner scratch overlaps the product repository"));
    }
    Ok(root)
}

// Check the resolved path and each original ancestor. This also rejects a
// symlink inside a forbidden root that happens to point outside that root.
fn touches_denied(path: &Path, roots: &[&Path]) -> bool {
    if !path.is_absolute() {
        return true;
    }
    path.ancestors().any(|entry| match fs::canonicalize(entry) {
        Ok(canonical) => roots.iter().any(|root| canonical.starts_with(root)),
        Err(_) => true,
    })
}

fn resolve_from_path(repo: &Path, scratch: &Path, path: &OsStr) -> Res<Option<CompilerSelection>> {
    let repo = fs::canonicalize(repo).map_err(io)?;
    let scratch = scratch_root(&repo, scratch)?;
    let roots = [repo.as_path(), scratch.as_path()];
    // Remove relative, missing, repository and scratch directories from PATH
    // for both the compiler and the tools it launches, including the linker.
    let dirs: Vec<_> = std::env::split_paths(path)
        .filter(|p| p.is_dir() && !touches_denied(p, &roots))
        .collect();
    let safe_path = std::env::join_paths(&dirs)
        .map_err(|_| blocked("cannot construct trusted compiler PATH"))?
        .into_string()
        .map_err(|_| blocked("compiler PATH is not UTF-8"))?;
    for dir in dirs {
        let executable = dir.join(RUSTC_NAME);
        if !executable.is_file() || touches_denied(&executable, &roots) {
            continue;
        }
        let canonical_executable = fs::canonicalize(&executable).map_err(io)?;
        let sha256 = sha256_file(&canonical_executable, MAX_COMPILER_BYTES)?;
        return Ok(Some(CompilerSelection {
            executable,
            canonical_executable,
            sha256,
            safe_path,
        }));
    }
    Ok(None)
}

fn resolve_rustc(repo: &Path, scratch: &Path) -> Res<Option<CompilerSelection>> {
    match std::env::var_os("PATH") {
        Some(path) => resolve_from_path(repo, scratch, &path),
        None => Ok(None),
    }
}

// These checks detect ordinary replacement between stages, not an attacker
// racing the check/open/exec, nor changes to the rest of the toolchain.
fn verify_compiler(selection: &CompilerSelection, repo: &Path, scratch: &Path) -> Res<()> {
    let repo = fs::canonicalize(repo).map_err(io)?;
    let roots = [repo.as_path(), scratch];
    if touches_denied(&selection.executable, &roots)
        || fs::canonicalize(&selection.executable).map_err(io)? != selection.canonical_executable
        || sha256_file(&selection.canonical_executable, MAX_COMPILER_BYTES)? != selection.sha256
    {
        return Err(blocked("compiler executable changed during preparation"));
    }
    Ok(())
}

fn path_string(path: &Path) -> Res<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| blocked("runner path is not valid UTF-8"))
}

fn open_regular(path: &Path, limit: u64) -> Res<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if is_link(&metadata) || !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(blocked(
            "runner output is missing, linked, empty or too large",
        ));
    }
    Ok(file)
}

fn read_regular_bounded(path: &Path, limit: u64) -> Res<Vec<u8>> {
    let file = open_regular(path, limit)?;
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data).map_err(io)?;
    if data.is_empty() || data.len() as u64 > limit {
        return Err(blocked("runner output is empty or too large"));
    }
    Ok(data)
}

fn sha256_file(path: &Path, limit: u64) -> Res<String> {
    let mut file = open_regular(path, limit)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > limit {
            return Err(blocked("compiler executable is too large"));
        }
        hash.update(&buf[..n]);
    }
    if total == 0 {
        return Err(blocked("empty compiler executable"));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_new(path: &Path, bytes: &[u8], executable: bool) -> Res<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = options.open(path).map_err(io)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(io)
}

fn command_spec(
    compiler: &Path,
    safe_path: &str,
    args: Vec<String>,
    repo: &Path,
    scratch: &Path,
    job: &Path,
    output_dir: &Path,
) -> Res<CommandSpec> {
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), safe_path.to_owned());
    let tmp = path_string(job)?;
    for name in ["TEMP", "TMP", "TMPDIR"] {
        env.insert(name.to_owned(), tmp.clone());
    }
    Ok(CommandSpec {
        program: path_string(compiler)?,
        args,
        cwd: job.to_path_buf(),
        env,
        read_only_roots: vec![repo.to_path_buf()],
        writable_roots: vec![scratch.to_path_buf()],
        output_dir: output_dir.to_path_buf(),
        timeout_s: RUNNER_TIMEOUT_S,
    })
}

#[derive(Debug)]
pub struct HostProbe {
    compiler: CompilerIdentity,
    command: CommandResult,
}

impl HostProbe {
    pub fn compiler(&self) -> &CompilerIdentity {
        &self.compiler
    }
    pub fn command(&self) -> &CommandResult {
        &self.command
    }
}

#[derive(Debug)]
pub enum ProbeOutcome {
    Ready(HostProbe),
    Terminal {
        status: RunnerStatus,
        command: CommandResult,
    },
    Unavailable,
}

#[derive(Debug)]
pub enum PreflightOutcome {
    Unavailable,
    Terminal {
        phase: RunnerPhase,
        status: RunnerStatus,
        compiler: Option<CompilerIdentity>,
        /// The successful probe's logs should be retained as evidence too.
        probe_command: Option<CommandResult>,
        command: CommandResult,
        /// Present only after successful compile/link and a bounded, regular
        /// output-file read. The caller persists and hashes these exact bytes.
        binary: Option<Vec<u8>>,
    },
}

fn terminal_status(result: &CommandResult) -> Res<Option<RunnerStatus>> {
    if !result.policy_applied {
        return Err(blocked("runner command policy was not applied"));
    }
    if result.timed_out {
        return Ok(Some(RunnerStatus::TimedOut));
    }
    match result.exit_code {
        Some(0) => Ok(None),
        Some(_) => Ok(Some(RunnerStatus::Failed)),
        None => Err(blocked("runner command has no terminal result")),
    }
}

fn parse_compiler(selection: &CompilerSelection, result: &CommandResult) -> Res<CompilerIdentity> {
    let bytes = read_regular_bounded(&result.stdout_path, MAX_VERSION_BYTES)?;
    let version =
        std::str::from_utf8(&bytes).map_err(|_| blocked("rustc version output is not UTF-8"))?;
    let host = runner::host_from_probe(&bytes)?;
    Ok(CompilerIdentity {
        executable: selection.executable.clone(),
        canonical_executable: selection.canonical_executable.clone(),
        executable_sha256: selection.sha256.clone(),
        version_verbose: version.to_owned(),
        host_triple: host.to_owned(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
    })
}

fn probe_with<F>(
    compiler: &CompilerSelection,
    repo: &Path,
    scratch: &Path,
    job: &Path,
    output: &Path,
    exec: &mut F,
) -> Res<ProbeOutcome>
where
    F: FnMut(&CommandSpec) -> Res<CommandResult>,
{
    verify_compiler(compiler, repo, scratch)?;
    let spec = command_spec(
        &compiler.executable,
        &compiler.safe_path,
        vec!["-vV".into()],
        repo,
        scratch,
        job,
        output,
    )?;
    let command = exec(&spec)?;
    let status = terminal_status(&command)?;
    if verify_compiler(compiler, repo, scratch).is_err() {
        return Ok(ProbeOutcome::Terminal {
            status: RunnerStatus::CompilerChanged,
            command,
        });
    }
    if let Some(status) = status {
        return Ok(ProbeOutcome::Terminal { status, command });
    }
    match parse_compiler(compiler, &command) {
        Ok(compiler) => Ok(ProbeOutcome::Ready(HostProbe { compiler, command })),
        Err(_) => Ok(ProbeOutcome::Terminal {
            status: RunnerStatus::InvalidOutput,
            command,
        }),
    }
}

/// Probe the local host in the existing scratch-only, network-disabled runner.
/// A caller reusing a binary should probe again and compare the host triple.
pub fn probe_host(server: &AppServer, repo: &Path, scratch: &Path) -> Res<ProbeOutcome> {
    let scratch = scratch_root(repo, scratch)?;
    let Some(compiler) = resolve_rustc(repo, &scratch)? else {
        return Ok(ProbeOutcome::Unavailable);
    };
    let job = scratch.join("prepared-runner-host-probe");
    fs::create_dir(&job).map_err(io)?;
    let output = job.join("output");
    fs::create_dir(&output).map_err(io)?;
    probe_with(&compiler, repo, &scratch, &job, &output, &mut |spec| {
        server.exec_check(spec)
    })
}

/// Compile and link the claimed source exactly once, with no runner execution.
/// The caller MUST persist the exact claim bytes before calling, and must not
/// retry an outstanding claim after an uncertain transport outcome.
pub fn preflight(
    server: &AppServer,
    repo: &Path,
    scratch: &Path,
    claim_bytes: &[u8],
    identity: &RunnerIdentity,
    source: &[u8],
) -> Res<PreflightOutcome> {
    preflight_with(
        repo,
        scratch,
        claim_bytes,
        identity,
        source,
        resolve_rustc(repo, scratch)?,
        &mut |spec| server.exec_check(spec),
    )
}

fn preflight_with<F>(
    repo: &Path,
    scratch: &Path,
    claim_bytes: &[u8],
    identity: &RunnerIdentity,
    source: &[u8],
    compiler: Option<CompilerSelection>,
    exec: &mut F,
) -> Res<PreflightOutcome>
where
    F: FnMut(&CommandSpec) -> Res<CommandResult>,
{
    runner::validate_claim(claim_bytes, identity, source)?;
    let scratch = scratch_root(repo, scratch)?;
    let Some(selection) = compiler else {
        return Ok(PreflightOutcome::Unavailable);
    };
    // create_dir prevents a duplicate invocation in the same scratch root.
    let job = scratch.join("prepared-runner-preflight");
    fs::create_dir(&job).map_err(io)?;
    let probe_output = job.join("probe");
    let compile_output = job.join("compile");
    fs::create_dir(&probe_output)
        .and_then(|()| fs::create_dir(&compile_output))
        .map_err(io)?;
    write_new(&job.join("runner.rs"), source, false)?;
    let (compiler, probe_command) =
        match probe_with(&selection, repo, &scratch, &job, &probe_output, exec)? {
            ProbeOutcome::Unavailable => return Ok(PreflightOutcome::Unavailable),
            ProbeOutcome::Terminal { status, command } => {
                return Ok(PreflightOutcome::Terminal {
                    phase: RunnerPhase::Probe,
                    status,
                    compiler: None,
                    probe_command: None,
                    command,
                    binary: None,
                });
            }
            ProbeOutcome::Ready(HostProbe { compiler, command }) => (compiler, command),
        };
    if verify_compiler(&selection, repo, &scratch).is_err() {
        return Ok(PreflightOutcome::Terminal {
            phase: RunnerPhase::Probe,
            status: RunnerStatus::CompilerChanged,
            compiler: None,
            probe_command: None,
            command: probe_command,
            binary: None,
        });
    }
    let spec = command_spec(
        &compiler.executable,
        &selection.safe_path,
        vec![
            "--edition=2021".into(),
            "--crate-type=bin".into(),
            "--emit=link".into(),
            "--crate-name".into(),
            "axiom_private_runner".into(),
            "-o".into(),
            RUNNER_NAME.into(),
            "runner.rs".into(),
        ],
        repo,
        &scratch,
        &job,
        &compile_output,
    )?;
    let command = exec(&spec)?;
    let status = terminal_status(&command)?;
    if verify_compiler(&selection, repo, &scratch).is_err() {
        return Ok(PreflightOutcome::Terminal {
            phase: RunnerPhase::Compile,
            status: RunnerStatus::CompilerChanged,
            compiler: Some(compiler),
            probe_command: Some(probe_command),
            command,
            binary: None,
        });
    }
    if let Some(status) = status {
        return Ok(PreflightOutcome::Terminal {
            phase: RunnerPhase::Compile,
            status,
            compiler: Some(compiler),
            probe_command: Some(probe_command),
            command,
            binary: None,
        });
    }
    let binary = read_regular_bounded(&job.join(RUNNER_NAME), MAX_RUNNER_BINARY_BYTES as u64).ok();
    let status = if binary.is_some() {
        RunnerStatus::Ready
    } else {
        RunnerStatus::InvalidOutput
    };
    Ok(PreflightOutcome::Terminal {
        phase: RunnerPhase::Compile,
        status,
        compiler: Some(compiler),
        probe_command: Some(probe_command),
        command,
        binary,
    })
}

/// Materialize only the verified bytes into an exclusive per-check directory.
/// The caller must execute the returned path directly with its sandbox; a shell
/// or any other executable would not enforce the frozen sentinel.
pub fn materialize(
    repo: &Path,
    scratch: &Path,
    claim_bytes: &[u8],
    receipt: &RunnerReceipt,
    expected_identity: &RunnerIdentity,
    sentinel: &RunnerSentinel,
    source: &[u8],
    binary: &[u8],
    persisted_probe_stdout: &[u8],
    observed_probe: &HostProbe,
) -> Res<PathBuf> {
    runner::validate_receipt(
        claim_bytes,
        receipt,
        expected_identity,
        Some(persisted_probe_stdout),
    )?;
    runner::validate_claim(claim_bytes, expected_identity, source)?;
    runner::verify_material(
        receipt,
        sentinel,
        source,
        binary,
        observed_probe.compiler.version_verbose.as_bytes(),
    )?;
    let scratch = scratch_root(repo, scratch)?;
    let job = scratch.join("prepared-runner-exec");
    fs::create_dir(&job).map_err(io)?;
    let path = job.join(RUNNER_NAME);
    write_new(&path, binary, true)?;
    let actual = read_regular_bounded(&path, MAX_RUNNER_BINARY_BYTES as u64)?;
    if runner::sha256(&actual) != sentinel.binary_digest {
        return Err(blocked("materialized prepared runner changed"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orch::{
        checkpoint::ArtifactRef,
        runner::{RunnerClaim, RUNNER_VERSION},
    };
    use std::process::Command;

    struct Fixture {
        _dir: tempfile::TempDir,
        repo: PathBuf,
        scratch: PathBuf,
        identity: RunnerIdentity,
        claim_bytes: Vec<u8>,
        source: Vec<u8>,
    }

    fn fixture(source: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let scratch = dir.path().join("scratch");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&scratch).unwrap();
        let identity = RunnerIdentity {
            run_id: "run-1".into(),
            epoch: 1,
            attempt_id: "attempt-1".into(),
            generation: 4,
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
        };
        let source = source.as_bytes().to_vec();
        let claim = RunnerClaim {
            version: RUNNER_VERSION,
            identity: identity.clone(),
            source: ArtifactRef {
                path: dir.path().join("saved-source"),
                digest: runner::sha256(&source),
            },
        };
        Fixture {
            _dir: dir,
            repo,
            scratch,
            identity,
            claim_bytes: serde_json::to_vec(&claim).unwrap(),
            source,
        }
    }

    fn result(
        spec: &CommandSpec,
        exit_code: Option<i32>,
        timed_out: bool,
        stdout: &[u8],
        stderr: &[u8],
    ) -> CommandResult {
        let stdout_path = spec.output_dir.join("stdout.txt");
        let stderr_path = spec.output_dir.join("stderr.txt");
        fs::write(&stdout_path, stdout).unwrap();
        fs::write(&stderr_path, stderr).unwrap();
        CommandResult {
            exit_code,
            timed_out,
            policy_applied: true,
            stdout_path,
            stderr_path,
            duration_ms: None,
        }
    }

    // This exercises the real local compiler/linker. It is deliberately not a
    // sandbox test: the production entry point always uses AppServer.exec_check.
    fn direct_test_exec(spec: &CommandSpec) -> Res<CommandResult> {
        let output = Command::new(&spec.program)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .envs(&spec.env)
            .output()
            .map_err(io)?;
        Ok(result(
            spec,
            output.status.code(),
            false,
            &output.stdout,
            &output.stderr,
        ))
    }

    fn artifact(path: PathBuf) -> ArtifactRef {
        ArtifactRef {
            digest: runner::sha256(&fs::read(&path).unwrap()),
            path,
        }
    }

    #[test]
    fn rustc_links_without_running_then_materializes_the_same_binary() {
        let f = fixture("fn main() { std::fs::write(\"executed-marker\", b\"ran\").unwrap(); }");
        let compiler = resolve_rustc(&f.repo, &f.scratch)
            .unwrap()
            .expect("Rust build environment must have rustc on PATH");
        let mut seen = Vec::new();
        let outcome = preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            Some(compiler),
            &mut |spec| {
                seen.push(spec.clone());
                direct_test_exec(spec)
            },
        )
        .unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].args, vec!["-vV"]);
        assert_eq!(
            seen[1].args,
            vec![
                "--edition=2021",
                "--crate-type=bin",
                "--emit=link",
                "--crate-name",
                "axiom_private_runner",
                "-o",
                RUNNER_NAME,
                "runner.rs"
            ]
        );
        for spec in &seen {
            assert_eq!(spec.timeout_s, RUNNER_TIMEOUT_S);
            assert_eq!(spec.read_only_roots, vec![f.repo.clone()]);
            assert_eq!(spec.writable_roots.len(), 1);
            assert!(spec.cwd.starts_with(&spec.writable_roots[0]));
            assert_eq!(spec.env["TMP"], path_string(&spec.cwd).unwrap());
        }
        assert!(!f
            .scratch
            .join("prepared-runner-preflight")
            .join("executed-marker")
            .exists());
        let (compiler, probe_command, command, binary) = match outcome {
            PreflightOutcome::Terminal {
                phase: RunnerPhase::Compile,
                status: RunnerStatus::Ready,
                compiler: Some(compiler),
                probe_command: Some(probe_command),
                command,
                binary: Some(binary),
            } => (compiler, probe_command, command, binary),
            other => panic!("expected linked executable, got {other:?}"),
        };
        let claim: RunnerClaim = serde_json::from_slice(&f.claim_bytes).unwrap();
        let binary_ref = ArtifactRef {
            path: f._dir.path().join("saved-binary"),
            digest: runner::sha256(&binary),
        };
        let sentinel = RunnerSentinel {
            source_digest: claim.source.digest.clone(),
            binary_digest: binary_ref.digest.clone(),
        };
        let receipt = RunnerReceipt {
            version: RUNNER_VERSION,
            identity: f.identity.clone(),
            claim_digest: runner::sha256(&f.claim_bytes),
            source: claim.source,
            binary: Some(binary_ref),
            compiler: Some(compiler.clone()),
            probe_stdout: Some(artifact(probe_command.stdout_path.clone())),
            status: RunnerStatus::Ready,
            phase: RunnerPhase::Compile,
            exit_code: command.exit_code,
            timed_out: command.timed_out,
            policy_applied: command.policy_applied,
            stdout: Some(artifact(command.stdout_path)),
            stderr: Some(artifact(command.stderr_path)),
        };
        let check_scratch = f._dir.path().join("check-scratch");
        fs::create_dir(&check_scratch).unwrap();
        let observed = HostProbe {
            compiler: compiler.clone(),
            command: probe_command,
        };
        let exe = materialize(
            &f.repo,
            &check_scratch,
            &f.claim_bytes,
            &receipt,
            &f.identity,
            &sentinel,
            &f.source,
            &binary,
            compiler.version_verbose.as_bytes(),
            &observed,
        )
        .unwrap();
        assert_eq!(
            runner::sha256(&fs::read(&exe).unwrap()),
            sentinel.binary_digest
        );
        assert!(!check_scratch.join("executed-marker").exists());
        assert!(Command::new(exe)
            .current_dir(&check_scratch)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            fs::read(check_scratch.join("executed-marker")).unwrap(),
            b"ran"
        );
        assert!(materialize(
            &f.repo,
            &check_scratch,
            &f.claim_bytes,
            &receipt,
            &f.identity,
            &sentinel,
            &f.source,
            &binary,
            compiler.version_verbose.as_bytes(),
            &observed
        )
        .is_err());
    }

    #[test]
    fn compiler_rejects_syntax_and_real_unresolved_linker_symbol() {
        for source in ["fn main( {", "extern \"C\" { fn axiom_missing_symbol_638251(); } fn main() { unsafe { axiom_missing_symbol_638251(); } }"] {
            let f = fixture(source);
            let mut count = 0;
            let outcome = preflight_with(&f.repo, &f.scratch, &f.claim_bytes, &f.identity, &f.source,
                resolve_rustc(&f.repo, &f.scratch).unwrap(), &mut |spec| { count += 1; direct_test_exec(spec) }).unwrap();
            match outcome {
                PreflightOutcome::Terminal { phase: RunnerPhase::Compile, status: RunnerStatus::Failed, command, binary: None, .. } => {
                    let stderr = fs::read_to_string(command.stderr_path).unwrap();
                    assert!(!stderr.is_empty());
                    if source.contains("axiom_missing_symbol") { assert!(stderr.contains("axiom_missing_symbol_638251"), "{stderr}"); }
                }
                other => panic!("expected compile or link failure, got {other:?}"),
            }
            assert_eq!(count, 2);
        }
    }

    #[test]
    fn missing_tool_timeout_and_invalid_output_fail_closed_without_retry() {
        let f = fixture("fn main() {}");
        let mut never =
            |_: &CommandSpec| -> Res<CommandResult> { panic!("no compiler should run") };
        assert!(matches!(
            preflight_with(
                &f.repo,
                &f.scratch,
                &f.claim_bytes,
                &f.identity,
                &f.source,
                None,
                &mut never
            )
            .unwrap(),
            PreflightOutcome::Unavailable
        ));
        let compiler = resolve_rustc(&f.repo, &f.scratch).unwrap().unwrap();
        let mut commands = 0;
        let outcome = preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            Some(compiler.clone()),
            &mut |spec| {
                commands += 1;
                if spec.args == ["-vV"] {
                    Ok(result(
                        spec,
                        Some(0),
                        false,
                        b"rustc test\nhost: test-target\n",
                        b"",
                    ))
                } else {
                    Ok(result(spec, None, true, b"", b"timed out"))
                }
            },
        )
        .unwrap();
        assert!(matches!(
            outcome,
            PreflightOutcome::Terminal {
                phase: RunnerPhase::Compile,
                status: RunnerStatus::TimedOut,
                binary: None,
                ..
            }
        ));
        assert_eq!(commands, 2);
        assert!(preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            Some(compiler.clone()),
            &mut never
        )
        .is_err());

        let f = fixture("fn main() {}");
        let mut count = 0;
        let outcome = preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            Some(compiler),
            &mut |spec| {
                count += 1;
                if spec.args == ["-vV"] {
                    Ok(result(
                        spec,
                        Some(0),
                        false,
                        b"rustc test\nhost: test-target\n",
                        b"",
                    ))
                } else {
                    Ok(result(spec, Some(0), false, b"", b""))
                }
            },
        )
        .unwrap();
        assert!(matches!(
            outcome,
            PreflightOutcome::Terminal {
                phase: RunnerPhase::Compile,
                status: RunnerStatus::InvalidOutput,
                binary: None,
                ..
            }
        ));
        assert_eq!(count, 2);
    }

    #[test]
    fn source_identity_and_scratch_overlap_fail_before_any_command() {
        let f = fixture("fn main() {}");
        let mut never = |_: &CommandSpec| -> Res<CommandResult> { panic!("command must not run") };
        let compiler = resolve_rustc(&f.repo, &f.scratch).unwrap();
        let mut changed = f.identity.clone();
        changed.turn_id.push('x');
        assert!(preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &changed,
            &f.source,
            compiler.clone(),
            &mut never
        )
        .is_err());
        assert!(preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            b"different",
            compiler.clone(),
            &mut never
        )
        .is_err());
        assert!(preflight_with(
            &f.repo,
            &f.repo,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            compiler,
            &mut never
        )
        .is_err());
    }

    #[test]
    fn path_excludes_repo_and_scratch_and_changed_compiler_is_blocked() {
        let f = fixture("fn main() {}");
        let repo_bin = f.repo.join("bin");
        let scratch_bin = f.scratch.join("bin");
        let external = f._dir.path().join("external-bin");
        for path in [&repo_bin, &scratch_bin, &external] {
            fs::create_dir(path).unwrap();
            fs::write(path.join(RUSTC_NAME), b"test compiler").unwrap();
        }
        let only_untrusted = std::env::join_paths([&repo_bin, &scratch_bin]).unwrap();
        assert!(resolve_from_path(&f.repo, &f.scratch, &only_untrusted)
            .unwrap()
            .is_none());
        let candidates = std::env::join_paths([&repo_bin, &scratch_bin, &external]).unwrap();
        let selection = resolve_from_path(&f.repo, &f.scratch, &candidates)
            .unwrap()
            .unwrap();
        assert_eq!(selection.executable, external.join(RUSTC_NAME));
        let retained: Vec<_> = std::env::split_paths(OsStr::new(&selection.safe_path)).collect();
        assert_eq!(retained, vec![external.clone()]);
        let mut count = 0;
        let outcome = preflight_with(
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
            Some(selection),
            &mut |spec| {
                count += 1;
                fs::write(external.join(RUSTC_NAME), b"changed compiler").unwrap();
                Ok(result(
                    spec,
                    Some(0),
                    false,
                    b"rustc test\nhost: test-target\n",
                    b"",
                ))
            },
        )
        .unwrap();
        assert!(matches!(
            outcome,
            PreflightOutcome::Terminal {
                phase: RunnerPhase::Probe,
                status: RunnerStatus::CompilerChanged,
                binary: None,
                ..
            }
        ));
        assert_eq!(count, 1);
    }

    #[test]
    fn appserver_entry_point_requests_fixed_sandbox_policy() {
        // Protocol fake validates the request and invokes local rustc; it does
        // not pretend to provide operating-system sandbox enforcement.
        let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
            let candidates = if cfg!(windows) {
                ["python", "python3"]
            } else {
                ["python3", "python"]
            };
            candidates
                .into_iter()
                .find(|p| {
                    Command::new(p)
                        .arg("--version")
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
                .expect("Python is required by the existing fake-server test suite")
                .to_owned()
        });
        let script = r#"
import json, os, subprocess, sys
sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')
for line in sys.stdin:
    m = json.loads(line)
    method = m.get('method')
    if method == 'initialized':
        continue
    if method == 'initialize':
        response = {'id': m['id'], 'result': {}}
    elif method == 'command/exec':
        p = m['params']
        policy = p['sandboxPolicy']
        valid = (policy.get('type') == 'workspaceWrite' and policy.get('networkAccess') is False
                 and policy.get('excludeTmpdirEnvVar') is True and policy.get('excludeSlashTmp') is True
                 and len(policy.get('writableRoots', [])) == 1 and p['timeoutMs'] == 120000
                 and os.path.commonpath([os.path.normcase(p['cwd']), os.path.normcase(policy['writableRoots'][0])])
                     == os.path.normcase(policy['writableRoots'][0]))
        if not valid:
            response = {'id': m['id'], 'error': {'code': -1, 'message': 'invalid sandbox request'}}
        else:
            r = subprocess.run(p['command'], cwd=p['cwd'], env=dict(os.environ, **p['env']),
                               capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=120)
            response = {'id': m['id'], 'result': {'exitCode': r.returncode, 'stdout': r.stdout,
                                                   'stderr': r.stderr, 'timedOut': False}}
    else:
        response = {'id': m.get('id'), 'error': {'code': -2, 'message': 'unexpected method'}}
    print(json.dumps(response), flush=True)
"#;
        let f = fixture("fn main() { std::fs::write(\"marker\", b\"run\").unwrap(); }");
        let server = AppServer::new(vec![
            python,
            "-I".into(),
            "-u".into(),
            "-c".into(),
            script.into(),
        ]);
        let result = preflight(
            &server,
            &f.repo,
            &f.scratch,
            &f.claim_bytes,
            &f.identity,
            &f.source,
        )
        .unwrap();
        let compiler = match result {
            PreflightOutcome::Terminal {
                phase: RunnerPhase::Compile,
                status: RunnerStatus::Ready,
                compiler: Some(compiler),
                binary: Some(_),
                ..
            } => compiler,
            other => panic!("expected sandbox-protocol compile, got {other:?}"),
        };
        assert!(!f
            .scratch
            .join("prepared-runner-preflight")
            .join("marker")
            .exists());
        let host_scratch = f._dir.path().join("host-scratch");
        fs::create_dir(&host_scratch).unwrap();
        match probe_host(&server, &f.repo, &host_scratch).unwrap() {
            ProbeOutcome::Ready(observed) => {
                assert_eq!(observed.compiler().host_triple, compiler.host_triple)
            }
            other => panic!("expected host probe, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn binary_symlink_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"target contents").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(target, &link).unwrap();
        assert!(read_regular_bounded(&link, 1024).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn junction_scratch_and_optional_file_symlink_are_rejected() {
        let f = fixture("fn main() {}");
        let junction = f._dir.path().join("scratch-junction");
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&f.scratch)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(scratch_root(&f.repo, &junction).is_err());

        let file = f._dir.path().join("target");
        let symlink = f._dir.path().join("file-link");
        fs::write(&file, b"target contents").unwrap();
        // File symlinks require an OS setting or privilege on Windows; when
        // available, also exercise the no-follow file handle path.
        if std::os::windows::fs::symlink_file(&file, &symlink).is_ok() {
            assert!(read_regular_bounded(&symlink, 1024).is_err());
        }
    }
}
