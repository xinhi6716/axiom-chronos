use crate::base::{self, Failure, Res};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

fn arg<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("")
}
fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
pub(super) fn path(cwd: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    if p.is_absolute() {
        p.into()
    } else {
        cwd.join(p)
    }
}

pub(super) fn resolved(target: &Path) -> Res<PathBuf> {
    if target
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Failure::new(
            "permission_denied",
            "parent traversal is not allowed",
        ));
    }
    let mut ancestor = target;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(error(e)),
        }
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| Failure::new("invalid_argument", "invalid file path"))?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Failure::new("invalid_argument", "invalid file path"))?;
    }
    let mut resolved = ancestor.canonicalize().map_err(error)?;
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    // Canonical containment also rejects symlinks and lexical parent traversal.
    if resolved
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Failure::new(
            "permission_denied",
            "parent traversal is not allowed",
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn unix_write_rejects_dangling_file_symlink_outside_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let outside = temp.path().join("outside.txt");
        let link = home.join("link.txt");
        std::os::unix::fs::symlink(&outside, &link).expect("creating test symlink failed");

        assert!(resolved(&link).is_err());

        let result = write(
            &home,
            &home,
            &json!({"file_path": "link.txt", "content": "escape"}),
            &[],
        );
        assert!(result.is_err());
        assert!(!outside.exists());
    }

    #[test]
    #[cfg(windows)]
    fn windows_rejects_dangling_directory_junction_outside_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let outside = temp.path().join("outside");
        let link = home.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("running mklink /J failed");
        assert!(
            output.status.success(),
            "creating test junction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(
            resolved(&link).is_err(),
            "dangling junction must be rejected"
        );
        let result = write(
            &home,
            &home,
            &json!({"file_path": "junction/file.txt", "content": "escape"}),
            &[],
        );
        assert!(result.is_err());
        assert!(!outside.exists());
        assert!(!outside.join("file.txt").exists());
    }

    #[test]
    fn write_preserves_missing_parent_and_exact_output_authorization() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let inside = home.join("new").join("file.txt");
        write(
            &home,
            &home,
            &json!({"file_path": inside, "content": "inside"}),
            &[],
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&inside).unwrap(), "inside");

        let outside = temp.path().join("output").join("file.txt");
        let authorized = resolved(&outside).unwrap();
        write(
            &home,
            &home,
            &json!({"file_path": outside, "content": "output"}),
            &[authorized],
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "output");
    }
}
pub(super) fn write(home: &Path, cwd: &Path, a: &Value, outputs: &[PathBuf]) -> Res<Value> {
    let resolved = resolved(&path(cwd, arg(a, "file_path")))?;
    let home = home.canonicalize().map_err(error)?;
    let authorized = outputs.iter().any(|p| p == &resolved);
    if !resolved.starts_with(home) && !authorized {
        return Err(Failure::new(
            "permission_denied",
            "host writes require data home or an explicitly authorized content output",
        ));
    }
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(error)?;
    }
    std::fs::write(&resolved, arg(a, "content")).map_err(error)?;
    Ok(json!({"file_path":resolved,"bytes":arg(a,"content").len()}))
}

pub(super) fn dispatch(
    home: &Path,
    cwd: &Path,
    name: &str,
    a: &Value,
    outputs: &[PathBuf],
    cancelled: &AtomicBool,
) -> Res<Value> {
    match name {
        "Read" => {
            let path = path(cwd, arg(a, "file_path"));
            let bytes = std::fs::read(&path).map_err(error)?;
            let text = String::from_utf8(bytes).map_err(|_| {
                Failure::new(
                    "unsupported_format",
                    "Read supports UTF-8 documents; use native view_image to inspect images",
                )
            })?;
            let offset = a["offset"].as_u64().unwrap_or(1).saturating_sub(1) as usize;
            let limit = a["limit"].as_u64().unwrap_or(2000) as usize;
            let content = text
                .lines()
                .enumerate()
                .skip(offset)
                .take(limit)
                .map(|(i, s)| format!("{}\t{}", i + 1, s))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(json!({"file_path":path,"content":content}))
        }
        "Write" => write(home, cwd, a, outputs),
        "Glob" | "Grep" => {
            let mut command = std::process::Command::new("rg");

            if name == "Glob" {
                command.args(["--files", "--hidden", "-g", arg(a, "pattern")]);
            } else {
                command.args(["--line-number", "--no-heading", "--color", "never"]);
                if a["-i"] == true {
                    command.arg("--ignore-case");
                }
                if let Some(glob) = a["glob"].as_str() {
                    command.args(["-g", glob]);
                }
                if a["output_mode"] == "files_with_matches" {
                    command.arg("--files-with-matches");
                }
                command.arg("--").arg(arg(a, "pattern"));
            }
            if let Some(p) = a["path"].as_str() {
                command.arg(path(cwd, p));
            }
            let result = base::proc::run_command(command, cwd, Duration::from_secs(30))?;
            if result.exit_code != 0 && result.exit_code != 1 {
                return Err(error(result.output));
            }
            Ok(
                json!({"exit_code":result.exit_code,"output":result.output,"timed_out":result.timed_out}),
            )
        }
        "Bash" => shell(cwd, a, cancelled),
        _ => Err(Failure::new("unknown_tool", name)),
    }
}

fn shell(cwd: &Path, a: &Value, _cancelled: &AtomicBool) -> Res<Value> {
    let text = arg(a, "command");
    if !crate::hook::read_only_bash(text) || text.chars().any(|c| "$`{}()<>".contains(c)) {
        return Err(Failure::new(
            "permission_denied",
            "host shell only accepts read-only commands without shell expansion",
        ));
    }
    #[cfg(windows)]
    let argv = {
        let mut v = base::resolve_command("AXIOM_PWSH_CMD", "pwsh")
            .unwrap_or_else(|_| vec!["powershell.exe".into()]);
        v.extend([
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            text.into(),
        ]);
        v
    };
    #[cfg(unix)]
    let argv = vec!["sh".into(), "-c".into(), text.into()];
    let result = base::proc::run_command(
        base::proc::command(&argv),
        cwd,
        Duration::from_millis(a["timeout"].as_u64().unwrap_or(120000).clamp(1, 120000)),
    )?;
    Ok(json!({"exit_code":result.exit_code,"output":result.output,"timed_out":result.timed_out}))
}
