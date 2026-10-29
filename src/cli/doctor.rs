use crate::base;
use std::{
    io::Write,
    process::{Command, Stdio},
};

struct Check {
    label: &'static str,
    ok: bool,
    required: bool,
    detail: String,
}

fn tool(
    label: &'static str,
    env: &str,
    executable: &str,
    required: bool,
    purpose: &str,
    fix: &str,
) -> Check {
    let result = base::resolve_command(env, executable)
        .map_err(|e| {
            if e.kind == "not_installed" {
                format!("not on PATH. {fix}")
            } else {
                e.message
            }
        })
        .and_then(|command| {
            let output = Command::new(&command[0])
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
        let status = Command::new(&command[0]).args(&command[1..]).args(["-e", "sh", "-c", "test -x \"$HOME/.axiom/linux-venv/bin/python\""]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| format!("WSL did not answer: {e}"))?;
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

pub fn doctor(out: &mut dyn Write) -> i32 {
    let mut checks = vec![
        tool(
            "claude",
            "AXIOM_CLAUDE_CMD",
            "claude",
            true,
            "runs every session",
            "Install Claude Code and sign in with `claude`.",
        ),
        tool(
            "codex",
            "AXIOM_CODEX_CMD",
            "codex",
            true,
            "terra and sol workers",
            "Install the Codex CLI and sign in.",
        ),
        tool(
            "agy",
            "AXIOM_AGY_CMD",
            "agy",
            false,
            "plain-language reports",
            "Install Antigravity; without it reports are shown as written.",
        ),
        sqlite(),
        state(),
    ];
    #[cfg(windows)]
    {
        checks.extend([windows_shell(), linux()]);
    }
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
