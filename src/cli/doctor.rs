use crate::{
    base, codegraph,
    codex::{self, Verdict},
};
use std::{
    io::Write,
    process::Stdio,
};

pub(crate) struct Check {
    pub label: &'static str,
    pub ok: bool,
    pub required: bool,
    pub detail: String,
}

fn resolve(env: &str, executable: &str, fix: &str) -> Result<Vec<String>, String> {
    base::resolve_command(env, executable).map_err(|e| {
        if e.kind == "not_installed" {
            format!("not on PATH. {fix}")
        } else {
            e.message
        }
    })
}

fn codex_cli() -> Check {
    let (ok, required, detail) = match resolve(
        "AXIOM_CODEX_CMD",
        "codex",
        "Install the Codex CLI and sign in.",
    ) {
        Err(detail) => (false, true, detail),
        Ok(command) => {
            let check = codex::check_installed_version(&command);
            (
                check.verdict == Verdict::Ok,
                check.verdict != Verdict::Warn,
                format!("{} (terra and sol workers)", check.message),
            )
        }
    };
    Check {
        label: "codex",
        ok,
        required,
        detail,
    }
}

fn tool(
    label: &'static str,
    env: &str,
    executable: &str,
    required: bool,
    purpose: &str,
    fix: &str,
) -> Check {
    let result = resolve(env, executable, fix).and_then(|command| {
        let output = crate::base::proc::cmd(&command[0])
            .args(&command[1..])
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{} failed: {e}", command[0]))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            let text = if stdout.is_empty() { &stderr } else { &stdout };
            Ok(format!(
                "{} ({purpose})",
                text.trim().lines().last().unwrap_or("")
            ))
        } else {
            let text = if stderr.is_empty() { &stdout } else { &stderr };
            let tail: String = text
                .trim()
                .chars()
                .rev()
                .take(200)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Err(format!("{} failed: {tail}", command[0]))
        }
    });
    match result {
        Ok(detail) => Check {
            label,
            ok: true,
            required,
            detail,
        },
        Err(detail) => Check {
            label,
            ok: false,
            required,
            detail,
        },
    }
}

fn sqlite() -> Check {
    let result = rusqlite::Connection::open_in_memory().and_then(|db| {
        db.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x, tokenize='trigram')")
    });
    let version = rusqlite::version();
    let (ok, detail) = match result {
        Ok(()) => (
            true,
            format!("SQLite {version} with trigram (memory search)"),
        ),
        Err(e) => (
            false,
            format!("SQLite {version}: {e}; memory needs SQLite 3.34+ with FTS5"),
        ),
    };
    Check {
        label: "sqlite fts5",
        ok,
        required: true,
        detail,
    }
}

fn state() -> Check {
    let home = base::axiom_home();
    let result = std::fs::create_dir_all(&home).and_then(|()| {
        let probe = home.join(format!(".write-probe-{}", base::unique()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        let written = file.write_all(b"ok");
        drop(file);
        let removed = std::fs::remove_file(probe);
        written.and(removed)
    });
    let (ok, detail) = match result {
        Ok(()) => (true, home.display().to_string()),
        Err(e) => (false, format!("{} is not writable: {e}", home.display())),
    };
    Check {
        label: "state",
        ok,
        required: true,
        detail,
    }
}

fn graph() -> Check {
    let detail = match base::on_path(codegraph::NAME) {
        None => "not on PATH (optional; runs work without it)".to_string(),
        Some(path) if std::env::current_dir().is_ok_and(|dir| codegraph::indexed(&dir)) => {
            format!("{path} (this directory has .codegraph/, so runs here use it)")
        }
        Some(path) => format!("{path} (no .codegraph/ here; `codegraph init` turns it on)"),
    };
    Check {
        label: "codegraph",
        ok: true,
        required: false,
        detail,
    }
}

fn agy() -> Check {
    let detail = match base::resolve_command("AXIOM_AGY_CMD", "agy") {
        Ok(command) => format!(
            "{} (optional; `axiom review` falls back to it when Codex is out of quota or logged out)",
            command[0]
        ),
        Err(e) if e.kind == "not_installed" => {
            "not on PATH (optional; `axiom review` then has Codex only)".to_string()
        }
        Err(e) => {
            return Check {
                label: "agy",
                ok: false,
                required: false,
                detail: e.message,
            }
        }
    };
    Check {
        label: "agy",
        ok: true,
        required: false,
        detail,
    }
}

#[cfg(windows)]
fn windows_shell() -> Check {
    match base::resolve_command("AXIOM_PWSH_CMD", "pwsh") {
        Ok(command) if command[0].to_ascii_lowercase().contains("windowsapps") => Check { label: "pwsh", ok: false, required: true, detail: "Store build; the Codex sandbox cannot start it, so workers cannot run tests. Remove it, then: winget install --id Microsoft.PowerShell --exact --installer-type wix".into() },
        Ok(command) => Check { label: "pwsh", ok: true, required: false, detail: command[0].clone() },
        Err(e) if e.kind == "not_installed" => Check { label: "pwsh", ok: true, required: false, detail: "not found; Codex falls back to Windows PowerShell, which its sandbox can start".into() },
        Err(e) => Check { label: "pwsh", ok: false, required: true, detail: e.message },
    }
}

#[cfg(windows)]
fn linux() -> Check {
    let result = base::resolve_command("AXIOM_WSL_CMD", "wsl.exe").map_err(|e| if e.kind == "not_installed" { "no WSL; full suites run on Windows only, where POSIX-only tests are skipped".into() } else { e.message }).and_then(|command| {
        let status = crate::base::proc::cmd(&command[0]).args(&command[1..]).args(["-e", "sh", "-c", "test -x \"$HOME/.axiom/linux-venv/bin/python\""]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| format!("WSL did not answer: {e}"))?;
        if status.success() { Ok("WSL ~/.axiom/linux-venv (full suites also run on Linux)".into()) } else { Err("WSL has no ~/.axiom/linux-venv. Inside WSL: uv venv ~/.axiom/linux-venv, then install the project test dependencies into it".into()) }
    });
    match result {
        Ok(detail) => Check {
            label: "linux",
            ok: true,
            required: false,
            detail,
        },
        Err(detail) => Check {
            label: "linux",
            ok: false,
            required: false,
            detail,
        },
    }
}

fn plugin() -> Vec<Check> {
    let check = |ok, required, detail| Check {
        label: "plugin",
        ok,
        required,
        detail,
    };
    let listed = base::resolve_command("AXIOM_CLAUDE_CMD", "claude")
        .ok()
        .and_then(|mut command| {
            command.extend(["plugin".to_owned(), "list".to_owned(), "--json".to_owned()]);
            crate::provenance::run_bounded(&command, None, std::time::Duration::from_secs(30))
        })
        .filter(|output| output.success)
        .and_then(|output| serde_json::from_str::<Vec<serde_json::Value>>(&output.stdout).ok());
    let Some(listed) = listed else {
        return vec![check(
            false,
            false,
            "`claude plugin list --json` gave no usable answer, so the plugin behind /chronos:wish was not checked".into(),
        )];
    };
    let installed: Vec<_> = listed
        .iter()
        .filter(|p| {
            p["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("chronos@"))
                && p["enabled"] != false
        })
        .collect();
    if installed.is_empty() {
        return vec![check(
            true,
            false,
            "not installed in Claude Code; `axiom wish` runs on this build's own engine".into(),
        )];
    }
    installed
        .into_iter()
        .map(|p| {
            let Some(root) = p["readFromFolder"].as_str().or(p["installPath"].as_str()) else {
                return check(false, false, "Claude Code did not say where the plugin loads from".into());
            };
            match crate::engine::check_plugin(std::path::Path::new(root)) {
                Err(reason) => check(false, true, reason),
                Ok(found) if found.differing.is_empty() => check(
                    true,
                    false,
                    format!("chronos {} matches this axiom binary (engine {})", found.version, crate::engine::build_id()),
                ),
                Ok(found) => check(
                    false,
                    false,
                    format!(
                        "chronos {} has the version of this axiom binary, but these files differ from the ones built into it: {}",
                        found.version,
                        found.differing.join(", ")
                    ),
                ),
            }
        })
        .collect()
}

/// Every environment check. This starts programs, so never call it on a UI thread.
pub(crate) fn checks() -> Vec<Check> {
    let mut checks = vec![
        tool(
            "claude",
            "AXIOM_CLAUDE_CMD",
            "claude",
            true,
            "runs every session",
            "Install Claude Code and sign in with `claude`.",
        ),
        codex_cli(),
        sqlite(),
        state(),
        graph(),
        agy(),
    ];
    checks.extend(plugin());
    #[cfg(windows)]
    {
        checks.extend([windows_shell(), linux()]);
    }
    checks
}

pub fn doctor(out: &mut dyn Write) -> i32 {
    let checks = checks();
    let failed = checks.iter().any(|c| !c.ok && c.required);
    for c in checks {
        let mark = if c.ok {
            "ok "
        } else if c.required {
            "ERR"
        } else {
            "warn"
        };
        let _ = writeln!(out, "{mark:4} {:12} {}", c.label, c.detail);
    }
    i32::from(failed)
}
