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
    let mut project = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return usage(e),
    };
    let mut turns = (skill == "research").then_some(40);
    let (mut no_architect, mut astra, mut creative, mut resume, mut project_set) =
        (false, false, false, None, false);
    if skill == "wish" && args.iter().any(|a| a == "--resume") {
        let mut j = 0;
        let mut seen = false;
        while j < args.len() {
            match args[j].as_str() {
                "--resume" if !seen && j + 1 < args.len() && !args[j + 1].starts_with('-') => {
                    seen = true;
                    j += 2;
                }
                "--max-turns" if j + 1 < args.len() => {
                    j += 2;
                }
                _ => return usage("--resume takes only a session id"),
            }
        }
        if !seen {
            return usage("--resume takes only a session id");
        }
    }
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => match value(args, &mut i, "--project") {
                Ok(v) => {
                    project = v.into();
                    project_set = true;
                }
                Err(e) => return usage(e),
            },
            "--max-turns" => match value(args, &mut i, "--max-turns").and_then(|s| {
                s.parse::<u32>()
                    .map_err(|_| "--max-turns requires an integer".into())
            }) {
                Ok(v) => turns = Some(v),
                Err(e) => return usage(e),
            },
            "--resume" if skill == "wish" => match value(args, &mut i, "--resume") {
                Ok(v) if resume.is_none() => resume = Some(v),
                _ => return usage("--resume takes only a session id"),
            },
            "--creative" if skill == "wish" => creative = true,
            "--no-architect" if skill == "wish" => no_architect = true,
            "--astra" if skill == "wish" => astra = true,
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => return usage(format!("unrecognized option: {s}")),
            _ => break,
        }
        i += 1;
    }
    if let Some(id) = resume {
        if i != args.len() || project_set || no_architect || astra || creative {
            return usage("--resume takes only a session id");
        }
        let config = session::Config::from_env();
        return match session::Attachment::open(
            &config,
            &serde_json::json!({"session":id,"max_turns":turns}),
        ) {
            Ok((_host, value)) => {
                println!("{}", value);
                0
            }
            Err(e) => usage(e.message),
        };
    }
    if i == args.len() && !creative {
        return usage(format!(
            "{skill}: say what you want, e.g. axiom {skill} {}",
            if skill == "wish" {
                "add a --stats flag to cli.py"
            } else {
                "how does oxide thickness affect RRAM endurance?"
            }
        ));
    }
    if !project.is_absolute() {
        project = match std::env::current_dir() {
            Ok(p) => p.join(project),
            Err(e) => return usage(e),
        };
    }
    if !project.is_dir() {
        return usage(format!(
            "--project is not a directory: {}",
            project.display()
        ));
    }
    let text = if creative {
        format!(
            "[creative]{}",
            if i == args.len() {
                String::new()
            } else {
                format!(" {}", args[i..].join(" "))
            }
        )
    } else {
        format!(
            "{}{}{}",
            if no_architect { "[no-architect] " } else { "" },
            if astra { "[astra] " } else { "" },
            args[i..].join(" ")
        )
    };
    let config = session::Config::from_env();
    match session::Attachment::open(
        &config,
        &serde_json::json!({"skill":skill,"text":text,"project":project,"max_turns":turns}),
    ) {
        Ok((_host, value)) => {
            println!("{}", value);
            0
        }
        Err(e) => usage(e.message),
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
            println!("axiom-chronos {} (beta)", env!("CARGO_PKG_VERSION"));
            0
        }
        "wish" | "research" => session_command(cmd, rest),
        "doctor" if rest.is_empty() => doctor::doctor(&mut io::stdout()),
        "bridge" if rest.is_empty() => bridge::serve(
            bridge::Config::from_env(),
            io::BufReader::new(io::stdin()),
            io::stdout(),
        ),
        "bridge" if rest.len() == 2 && rest[0] == "call" => {
            use std::io::Read;
            let mut data = String::new();
            let read = if rest[1] == "-" {
                io::stdin().read_to_string(&mut data)
            } else {
                std::fs::read_to_string(&rest[1]).map(|s| {
                    data = s;
                    data.len()
                })
            };
            if let Err(e) = read {
                return usage(e);
            }
            let input: serde_json::Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(e) => return usage(e),
            };
            let bridge = bridge::Bridge::new(bridge::Config::from_env());
            let result = bridge::call(
                &bridge,
                input["name"].as_str().unwrap_or(""),
                input.get("arguments").unwrap_or(&serde_json::Value::Null),
            );
            println!("{}", result);
            if result["isError"] == true {
                1
            } else {
                0
            }
        }
        "hook" if rest.is_empty() => hook::main_stdin(),
        "orch" => orch_command(rest),
        "memory" => memory_command(rest),
        _ => usage(format!("unknown command or arguments: {cmd}")),
    }
}
