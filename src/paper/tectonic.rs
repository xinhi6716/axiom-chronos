use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tectonic {
    pub argv: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompileReport {
    pub pdf: Option<PathBuf>,
    pub log: PathBuf,
    pub ok: bool,
    pub unresolved_citations: Vec<String>,
    pub undefined_refs: Vec<String>,
    pub missing_files: Vec<String>,
    pub missing_chars: Vec<String>,
}

fn installed(argv: Vec<String>) -> Res<Tectonic> {
    let executable = &argv[0];
    if !Path::new(executable).is_file() && base::on_path(executable).is_none() {
        return Err(Failure::new(
            "not_installed",
            format!("Tectonic command not found: {executable}"),
        ));
    }
    Ok(Tectonic { argv })
}

pub fn find(data_home: &Path) -> Res<Tectonic> {
    if std::env::var_os("AXIOM_TECTONIC_CMD").is_some_and(|s| !s.is_empty()) {
        return installed(base::resolve_command("AXIOM_TECTONIC_CMD", "tectonic")?);
    }
    let settings = data_home.join("paper/settings.json");
    if settings.is_file() {
        let data = fs::read(&settings).map_err(|e| Failure::new("config", e.to_string()))?;
        let json: serde_json::Value =
            serde_json::from_slice(&data).map_err(|e| Failure::new("config", e.to_string()))?;
        if let Some(value) = json.get("tectonic") {
            let argv: Vec<String> = serde_json::from_value(value.clone())
                .map_err(|e| Failure::new("config", e.to_string()))?;
            if argv.is_empty() || argv.iter().any(|x| x.is_empty()) {
                return Err(Failure::new(
                    "config",
                    "tectonic must be a non-empty command array",
                ));
            }
            return installed(argv);
        }
    }
    Ok(Tectonic {
        argv: base::resolve_command("AXIOM_TECTONIC_CMD", "tectonic")?,
    })
}

pub fn find_for_tests() -> Option<Tectonic> {
    if let Ok(argv) = base::resolve_command("AXIOM_TECTONIC_CMD", "tectonic") {
        let first = &argv[0];
        if Path::new(first).is_file() || base::on_path(first).is_some() {
            return Some(Tectonic { argv });
        }
    }
    if let Some(exe) = base::on_path("tectonic") {
        return Some(Tectonic { argv: vec![exe] });
    }
    let fallback = "E:/codex/.toolchains/tectonic-0.17.0/tectonic.exe";
    Path::new(fallback).is_file().then(|| Tectonic {
        argv: vec![fallback.into()],
    })
}

pub fn compile(t: &Tectonic, main: &Path, offline: bool, timeout: Duration) -> Res<CompileReport> {
    compile_with_status(t, main, offline, timeout).map(|(report, _, _)| report)
}

fn compile_with_status(
    t: &Tectonic,
    main: &Path,
    offline: bool,
    timeout: Duration,
) -> Res<(CompileReport, i32, bool)> {
    if t.argv.is_empty() || t.argv[0].is_empty() {
        return Err(Failure::new("config", "Empty Tectonic command"));
    }
    let main = fs::canonicalize(main).map_err(|e| Failure::new("io", e.to_string()))?;
    let dir = main
        .parent()
        .ok_or_else(|| Failure::new("io", "Missing source directory"))?;
    let pdf = main.with_extension("pdf");
    let log = main.with_extension("log");
    let bib_log = main.with_extension("blg");
    for p in [&pdf, &log, &bib_log] {
        if p.exists() {
            fs::remove_file(p).map_err(|e| Failure::new("io", e.to_string()))?;
        }
    }
    let mut argv = t.argv.clone();
    if Path::new(&argv[0]).is_file() {
        argv[0] = fs::canonicalize(&argv[0])
            .map_err(|e| Failure::new("process", e.to_string()))?
            .to_string_lossy()
            .into_owned();
    }
    let mut cmd = base::proc::command(&argv);
    cmd.args(["-X", "compile", "--keep-logs", "--keep-intermediates"]);
    if offline {
        cmd.arg("--only-cached");
    }
    cmd.arg(main.file_name().unwrap())
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child =
        base::proc::spawn_group(&mut cmd).map_err(|e| Failure::new("process", e.to_string()))?;
    let output = base::proc::capture(child, timeout)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tex_log = fs::read_to_string(&log).unwrap_or_default();
    if !log.exists() {
        fs::write(&log, format!("{stdout}\n{stderr}"))
            .map_err(|e| Failure::new("io", e.to_string()))?;
    }
    let bib_diagnostics = fs::read_to_string(&bib_log).unwrap_or_default();
    let diagnostics = format!("{tex_log}\n{bib_diagnostics}\n{stdout}\n{stderr}");
    let mut citations = Vec::new();
    let mut refs = Vec::new();
    let mut missing = Vec::new();
    let mut chars = Vec::new();
    for line in diagnostics.lines() {
        let lower = line.to_ascii_lowercase();
        let push = |v: &mut Vec<String>| {
            if !v.iter().any(|s| s == line) {
                v.push(line.to_string());
            }
        };
        if lower.contains("citation")
            && (lower.contains("undefined") || lower.contains("unresolved"))
        {
            push(&mut citations);
        }
        if lower.contains("reference `")
            && (lower.contains("undefined") || lower.contains("unresolved"))
        {
            push(&mut refs);
        }
        if !lower.contains("fontconfig error")
            && (lower.contains("not found")
                || lower.contains("not available")
                || lower.contains("cannot find")
                || lower.contains("couldn't open")
                || lower.contains("can't find")
                || lower.contains("failed to open")
                || lower.contains("failed to download")
                || lower.contains("no such file"))
            && (lower.contains("file")
                || lower.contains(".bst")
                || lower.contains(".tex")
                || lower.contains(".cls"))
        {
            push(&mut missing);
        }
        if lower.contains("missing character") {
            push(&mut chars);
        }
    }
    let pdf = pdf.is_file().then_some(pdf);
    let ok = output.exit_code == 0
        && !output.timed_out
        && pdf.is_some()
        && citations.is_empty()
        && refs.is_empty()
        && missing.is_empty();
    Ok((
        CompileReport {
            pdf,
            log,
            ok,
            unresolved_citations: citations,
            undefined_refs: refs,
            missing_files: missing,
            missing_chars: chars,
        },
        output.exit_code,
        output.timed_out,
    ))
}

pub fn compile_figure(t: &Tectonic, tex: &Path, offline: bool, timeout: Duration) -> Res<PathBuf> {
    let (report, exit_code, timed_out) = compile_with_status(t, tex, offline, timeout)?;
    if report.ok {
        return report
            .pdf
            .ok_or_else(|| Failure::new("tectonic", "No figure PDF"));
    }
    let detail = if report.missing_files.is_empty() {
        let log = fs::read_to_string(&report.log).unwrap_or_default();
        let lines: Vec<_> = log.lines().collect();
        format!(
            "exit status: {exit_code}, timed out: {timed_out}; last 40 log lines:\n{}",
            lines[lines.len().saturating_sub(40)..].join("\n")
        )
    } else {
        report.missing_files.join("; ")
    };
    Err(Failure::new(
        "tectonic",
        format!(
            "Figure compilation failed; see {}: {}",
            report.log.display(),
            detail
        ),
    ))
}
