mod doctor;
use crate::{base, bridge, hook, memory::Memory, orch, session};
use std::{ffi::OsString, io, path::PathBuf};

fn usage(message: impl std::fmt::Display) -> i32 {
    eprintln!("axiom: {message}");
    2
}
fn value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{name} requires a value"))
}
fn session_command(skill: &str, args: &[String]) -> i32 {
    use crate::bridge::dispatch::{DispatchAction, RecoveryDecision};
    let mut project = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return usage(e),
    };
    let (mut action, mut request_id, mut digest, mut verdict, mut reason, mut recovery) =
        (None, None, None, None, None, None);
    let mut words = Vec::new();
    let (mut creative, mut astra, mut no_architect) = (false, false, false);
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if !words.is_empty() {
            words.extend_from_slice(&args[i..]);
            break;
        }
        match flag {
            "--creative" => creative = true,
            "--astra" => astra = true,
            "--no-architect" => no_architect = true,
            "--max-turns" => return usage(
                "--max-turns is unavailable with checkpoint dispatch; no turn limit was applied",
            ),
            "--project" | "--request-id" | "--research-digest" | "--verdict" | "--reason"
            | "--recovery-file" | "--resume" | "--status" | "--cancel" | "--decide" => {
                let v = match value(args, &mut i, flag) {
                    Ok(v) if !v.is_empty() => v,
                    Ok(_) => return usage(format!("{flag} cannot be empty")),
                    Err(e) => return usage(e),
                };
                match flag {
                    "--project" => project = v.into(),
                    "--request-id" => request_id = Some(v),
                    "--research-digest" => digest = Some(v),
                    "--verdict" => verdict = Some(v),
                    "--reason" => reason = Some(v),
                    "--recovery-file" => recovery = Some(v),
                    _ => {
                        if action.is_some() {
                            return usage("only one action is allowed");
                        }
                        action = Some((flag.to_owned(), v));
                    }
                }
            }
            "--" => {
                words.extend_from_slice(&args[i + 1..]);
                break;
            }
            x if x.starts_with('-') => return usage(format!("unrecognized option: {x}")),
            _ => words.push(args[i].clone()),
        }
        i += 1;
    }
    let action = match action {
        None if (!words.is_empty() || creative)
            && digest.is_none()
            && verdict.is_none()
            && reason.is_none()
            && recovery.is_none() =>
        {
            let mut intent = String::new();
            if skill == "research" {
                intent.push_str("Research only; stop at the research review boundary. ");
            }
            if creative {
                intent.push_str("[creative] ");
            }
            if !creative && no_architect {
                intent.push_str("[no-architect] ");
            }
            if !creative && astra {
                intent.push_str("[astra] ");
            }
            intent.push_str(&words.join(" "));
            DispatchAction::Start {
                intent: intent.trim_end().to_owned(),
            }
        }
        Some((flag, id)) if words.is_empty() && !creative && !astra && !no_architect => {
            match flag.as_str() {
                "--resume" if digest.is_none() && verdict.is_none() && reason.is_none() => {
                    let recovery = match recovery {
                        Some(path) => match std::fs::read(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|b| {
                                serde_json::from_slice::<RecoveryDecision>(&b)
                                    .map_err(|e| e.to_string())
                            }) {
                            Ok(v) => Some(v),
                            Err(e) => return usage(format!("invalid recovery file: {e}")),
                        },
                        None => None,
                    };
                    if let Some(ref decision) = recovery {
                        use sha2::{Digest, Sha256};
                        let refs: Vec<&crate::bridge::dispatch::ArtifactRef> = match decision {
                            RecoveryDecision::AcceptTerminal { report, .. } => vec![report],
                            RecoveryDecision::RetryAfterRepair {
                                stopped_evidence,
                                repair_evidence,
                                ..
                            } => vec![stopped_evidence, repair_evidence],
                        };
                        for artifact in refs {
                            let bytes = match std::fs::read(&artifact.path) {
                                Ok(bytes) => bytes,
                                Err(e) => return usage(format!("invalid recovery artifact: {e}")),
                            };
                            if artifact.digest != format!("{:x}", Sha256::digest(&bytes)) {
                                return usage("recovery artifact digest mismatch");
                            }
                        }
                    }
                    DispatchAction::Resume {
                        run_id: id,
                        recovery,
                    }
                }
                "--status"
                    if digest.is_none()
                        && verdict.is_none()
                        && reason.is_none()
                        && recovery.is_none() =>
                {
                    DispatchAction::Status { run_id: id }
                }
                "--cancel" if digest.is_none() && verdict.is_none() && recovery.is_none() => {
                    DispatchAction::Cancel {
                        run_id: id,
                        reason: reason.unwrap_or_else(|| "user requested cancellation".into()),
                    }
                }
                "--decide" if recovery.is_none() => match (digest, verdict.as_deref(), reason) {
                    (Some(research_digest), Some(v @ ("pass" | "fail")), Some(reason)) => {
                        DispatchAction::Decide {
                            run_id: id,
                            research_digest,
                            pass: v == "pass",
                            reason,
                        }
                    }
                    _ => return usage(
                        "--decide requires --research-digest, --verdict pass|fail, and --reason",
                    ),
                },
                _ => return usage("invalid action options"),
            }
        }
        _ => return usage("provide an intent or one action"),
    };
    match session::dispatch_action(
        &session::Config::from_env(),
        &project,
        action,
        request_id.unwrap_or_else(base::unique),
        &mut io::stdout(),
    ) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("axiom: {e}");
            1
        }
    }
}
fn orch_command(args: &[String]) -> i32 {
    let mut store = std::env::var_os("ORCH_STORE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let (mut json, mut force, mut i) = (false, false, 0);
    while i < args.len() {
        match args[i].as_str() {
            "--store" => match value(args, &mut i, "--store") {
                Ok(v) => store = Some(v.into()),
                Err(e) => return usage(e),
            },
            "--json" => json = true,
            "--force" => force = true,
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => return usage(format!("unrecognized option: {s}")),
            _ => break,
        }
        i += 1;
    }
    let Some(store) = store else {
        return usage("orch: --store or ORCH_STORE is required");
    };
    if i == args.len() {
        return usage("orch: missing command");
    }
    let out = orch::run(&store, &args[i..], json, force);
    print!("{}", out.stdout);
    eprint!("{}", out.stderr);
    out.exit_code
}
fn memory_command(args: &[String]) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        return usage("memory: missing command");
    };
    let mut query = Vec::new();
    let mut project = None;
    let mut limit = 10i64;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--project" if command == "search" => match value(args, &mut i, "--project") {
                Ok(v) => project = Some(v),
                Err(e) => return usage(e),
            },
            "--limit" if command == "search" => match value(args, &mut i, "--limit").and_then(|v| {
                v.parse::<i64>()
                    .map_err(|_| "--limit requires an integer".into())
            }) {
                Ok(v) => limit = v,
                Err(e) => return usage(e),
            },
            s if s.starts_with('-') => return usage(format!("unrecognized option: {s}")),
            _ => query.push(args[i].clone()),
        }
        i += 1;
    }
    if query.is_empty() || (command != "search" && query.len() != 1) {
        return usage("memory: invalid arguments");
    }
    let id = if command == "get" {
        match query[0].parse::<i64>() {
            Ok(n) => Some(n),
            Err(_) => return usage("memory get: id must be an integer"),
        }
    } else {
        None
    };
    if !matches!(command, "search" | "get" | "digest") {
        return usage(format!("memory: unknown command {command}"));
    }
    let result = Memory::open(&base::axiom_home()).and_then(|m| match command {
        "search" => m.search(&query.join(" "), project.as_deref(), limit),
        "get" => m.get(id.unwrap()),
        _ => m.digest(&query[0], None),
    });
    match result {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            0
        }
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
pub fn main(args: Vec<OsString>) -> i32 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
        SetConsoleCP(65001);
        SetConsoleOutputCP(65001);
    }
    let args: Vec<String> = match args
        .into_iter()
        .skip(1)
        .map(|s| s.into_string().map_err(|_| ()))
        .collect()
    {
        Ok(a) => a,
        Err(_) => return usage("arguments must be UTF-8"),
    };
    let Some(cmd) = args.first().map(String::as_str) else {
        return usage("missing command");
    };
    let rest = &args[1..];
    match cmd {
        "--version" if rest.is_empty() => {
            println!("axiom-chronos {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "wish" | "research" => session_command(cmd, rest),
        "doctor" if rest.is_empty() => doctor::doctor(&mut io::stdout()),
        "bridge" if rest.is_empty() => bridge::serve(
            bridge::Config::from_env(),
            io::BufReader::new(io::stdin()),
            io::stdout(),
        ),
        "hook" if rest.is_empty() => hook::main_stdin(),
        "orch" => orch_command(rest),
        "memory" => memory_command(rest),
        _ => usage(format!("unknown command or arguments: {cmd}")),
    }
}
