//! Windows and Unix desktops register login start; macOS is a no-op in this phase.
use crate::base::{self, Failure, Res};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
pub trait LoginStart: Send + Sync {
    fn enable(&self, command: &[String]) -> Res<()>;
    fn disable(&self) -> Res<()>;
}
#[derive(Clone, Default)]
pub struct Recording {
    pub calls: Arc<Mutex<Vec<String>>>,
}
impl Recording {
    pub fn new(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { calls }
    }
}
impl LoginStart for Recording {
    fn enable(&self, command: &[String]) -> Res<()> {
        self.calls.lock().unwrap().push(format!(
            "enable({})",
            serde_json::to_string(command).unwrap()
        ));
        Ok(())
    }
    fn disable(&self) -> Res<()> {
        self.calls.lock().unwrap().push("disable".into());
        Ok(())
    }
}
pub trait Runner: Send + Sync {
    fn run(&self, argv: &[String]) -> Res<()>;
}

pub struct CommandRunner;
impl Runner for CommandRunner {
    fn run(&self, argv: &[String]) -> Res<()> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| Failure::new("invalid_argument", "empty command"))?;
        let mut command = crate::base::proc::cmd(program);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let output = command
            .output()
            .map_err(|e| Failure::new("login_start", e.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Failure::new(
                "login_start",
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }
}

#[derive(Clone, Default)]
pub struct RecordingRunner {
    pub calls: Arc<Mutex<Vec<Vec<String>>>>,
}
impl RecordingRunner {
    pub fn new(calls: Arc<Mutex<Vec<Vec<String>>>>) -> Self {
        Self { calls }
    }
}
impl Runner for RecordingRunner {
    fn run(&self, argv: &[String]) -> Res<()> {
        self.calls.lock().unwrap().push(argv.to_vec());
        Ok(())
    }
}

pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "Axiom Chronos";
pub fn quote_arg(a: &str) -> String {
    format!("\"{}\"", a.replace('"', "\\\""))
}
pub fn windows_commands(command: &[String]) -> (Vec<String>, Vec<String>) {
    let value = command
        .iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    (
        vec![
            "reg.exe".into(),
            "add".into(),
            RUN_KEY.into(),
            "/v".into(),
            VALUE_NAME.into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            value,
            "/f".into(),
        ],
        vec![
            "reg.exe".into(),
            "delete".into(),
            RUN_KEY.into(),
            "/v".into(),
            VALUE_NAME.into(),
            "/f".into(),
        ],
    )
}

pub fn xdg_entry(command: &[String]) -> String {
    let line = command
        .iter()
        .map(|arg| {
            if !arg
                .chars()
                .any(|c| matches!(c, ' ' | '\t' | '"' | '\\' | '$' | '`' | '%'))
            {
                return arg.clone();
            }
            let mut result = String::from("\"");
            for c in arg.chars() {
                match c {
                    '\\' | '"' | '$' | '`' => {
                        result.push('\\');
                        result.push(c);
                    }
                    '%' => result.push_str("%%"),
                    _ => result.push(c),
                }
            }
            result.push('"');
            result
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("[Desktop Entry]\nType=Application\nName=Axiom Chronos\nExec={line}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")
}

pub struct Windows {
    runner: Box<dyn Runner>,
}
impl Windows {
    pub fn new(runner: Box<dyn Runner>) -> Self {
        Self { runner }
    }
}
impl LoginStart for Windows {
    fn enable(&self, command: &[String]) -> Res<()> {
        if command.is_empty() {
            return Err(Failure::new("invalid_argument", "empty command"));
        }
        self.runner.run(&windows_commands(command).0)
    }
    fn disable(&self) -> Res<()> {
        let _ = self.runner.run(&windows_commands(&[]).1);
        Ok(())
    }
}

pub struct Xdg {
    dir: PathBuf,
}
impl Xdg {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    pub fn default_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .map(|p| p.join("autostart"))
    }
}
impl LoginStart for Xdg {
    fn enable(&self, command: &[String]) -> Res<()> {
        if command.is_empty() {
            return Err(Failure::new("invalid_argument", "empty command"));
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Failure::new("login_start", e.to_string()))?;
        base::atomic_write(
            &self.dir.join("axiom-chronos.desktop"),
            xdg_entry(command).as_bytes(),
        )
        .map_err(|e| Failure::new("login_start", e.to_string()))
    }
    fn disable(&self) -> Res<()> {
        match std::fs::remove_file(self.dir.join("axiom-chronos.desktop")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Failure::new("login_start", e.to_string())),
        }
    }
}

/// macOS login-start registration is not implemented in this phase.
pub struct MacNoop;
impl LoginStart for MacNoop {
    fn enable(&self, _: &[String]) -> Res<()> {
        Ok(())
    }
    fn disable(&self) -> Res<()> {
        Ok(())
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
struct Noop;
#[cfg(all(not(windows), not(target_os = "macos")))]
impl LoginStart for Noop {
    fn enable(&self, _: &[String]) -> Res<()> {
        Ok(())
    }
    fn disable(&self) -> Res<()> {
        Ok(())
    }
}
pub fn platform() -> Box<dyn LoginStart> {
    #[cfg(windows)]
    {
        Box::new(Windows::new(Box::new(CommandRunner)))
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(MacNoop)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        match Xdg::default_dir() {
            Some(dir) => Box::new(Xdg::new(dir)),
            None => Box::new(Noop),
        }
    }
}
