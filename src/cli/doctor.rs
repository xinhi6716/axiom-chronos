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

pub fn doctor(out: &mut dyn Write) -> i32 {
    let codex = tool(
        "codex",
        "AXIOM_CODEX_CMD",
        "codex",
        true,
        "app-server, dynamic tools, read/interrupt and command/exec required",
        "Install and authenticate Codex CLI.",
    );
    let data = base::data_home();
    let failed = !codex.ok || data.as_ref().map_or(true, |p| p.exists() && !p.is_dir());
    let _ = writeln!(
        out,
        "{} codex        {}",
        if codex.ok { "ok " } else { "ERR" },
        codex.detail
    );
    match data {
        Ok(path) => {
            let _ = writeln!(
                out,
                "{} data         {}",
                if path.exists() && !path.is_dir() {
                    "ERR"
                } else {
                    "   "
                },
                path.display()
            );
        }
        Err(e) => {
            let _ = writeln!(out, "ERR data         {}", e.message);
        }
    }
    let home = base::axiom_home();
    let _ = writeln!(
        out,
        "    plugin       {}",
        home.join("engine")
            .join(env!("CARGO_PKG_VERSION"))
            .display()
    );
    match std::env::current_exe() {
        Ok(path) => {
            let _ = writeln!(out, "    binary       {}", path.display());
        }
        Err(e) => {
            let _ = writeln!(out, "warn binary       {e}");
        }
    }
    let _ = writeln!(out, "warn capabilities app-server protocol, dynamic tools, history, interruption and sandboxed command/exec are not verified by --version; run a loaded bridge smoke test");
    i32::from(failed)
}
