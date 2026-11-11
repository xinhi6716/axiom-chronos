pub(crate) mod doctor;
use crate::{base, bridge, evolve, hook, memory::Memory, orch, session};
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
    let (mut no_architect, mut creative, mut resume, mut project_set) =
        (false, false, None, false);
    let mut launch = session::Launch::default();
    let mut caps = std::collections::BTreeMap::new();
    const RESUME_USAGE: &str = "--resume takes a session id and an optional message";
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
                "--" if seen => break,
                s if seen && !s.starts_with('-') => break,
                _ => return usage(RESUME_USAGE),
            }
        }
        if !seen {
            return usage(RESUME_USAGE);
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
                _ => return usage(RESUME_USAGE),
            },
            "--creative" if skill == "wish" => creative = true,
            "--no-architect" if skill == "wish" => no_architect = true,
            "--advanced-research" => launch.advanced_research = true,
            flag @ ("--fable-cap" | "--astra-cap" | "--council-rounds") => {
                let key = match flag { "--fable-cap" => crate::council::budgets::FABLE_CAP, "--astra-cap" => crate::council::budgets::ASTRA_CAP, _ => crate::council::budgets::ROUNDS };
                match value(args, &mut i, flag) { Ok(v) => { caps.insert(key, v); }, Err(e) => return usage(e) }
            },
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => return usage(format!("unrecognized option: {s}")),
            _ => break,
        }
        i += 1;
    }
    if !caps.is_empty() && !launch.advanced_research { return usage("budget options require --advanced-research"); }
    if launch.advanced_research {
        launch.council_budgets = match crate::council::Budgets::parse(|k| caps.get(k).cloned().or_else(|| crate::council::budgets::env_value(k))) {
            Ok(b) => Some(b), Err(e) => return usage(e),
        };
    }
    if let Some(id) = resume {
        if project_set || no_architect || creative {
            return usage(RESUME_USAGE);
        }
        let message = (i < args.len()).then(|| args[i..].join(" "));
        return match session::resume(
            &session::Config::from_env(),
            &id,
            turns,
            message.as_deref(),
            Box::new(io::BufReader::new(io::stdin())),
            &mut io::stdout(),
        ) {
            Ok(code) => code,
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
    let text = session::wish_text(&args[i..].join(" "), creative, no_architect);
    match session::run_with(
        &session::Config::from_env(),
        skill,
        &text,
        &project,
        turns,
        Box::new(io::BufReader::new(io::stdin())),
        &mut io::stdout(),
        &launch,
    ) {
        Ok(code) => code,
        Err(e) => usage(e.message),
    }
}
fn wait_command(args: &[String]) -> i32 {
    let [id] = args else {
        return usage("wait takes only a session id");
    };
    match session::wait_and_resume(
        &session::Config::from_env(),
        id,
        Box::new(io::BufReader::new(io::stdin())),
        &mut io::stdout(),
    ) {
        Ok(code) => code,
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
fn lint_command(args: &[String]) -> i32 {
    let mut project = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return usage(e),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => match value(args, &mut i, "--project") {
                Ok(v) => project = v.into(),
                Err(e) => return usage(e),
            },
            s => return usage(format!("lint: unrecognized argument: {s}")),
        }
        i += 1;
    }
    if !project.is_absolute() {
        project = match std::env::current_dir() {
            Ok(p) => p.join(project),
            Err(e) => return usage(e),
        };
    }
    if !project.is_dir() {
        return usage(format!("--project is not a directory: {}", project.display()));
    }
    let slug = base::slug_for_dir(&project);
    match Memory::open(&base::axiom_home()).and_then(|m| m.lint(&slug, &project)) {
        Ok(problems) if problems.is_empty() => {
            println!("axiom lint: no problems ({slug})");
            0
        }
        Ok(problems) => {
            for p in &problems {
                println!("{}: {}", p.kind, p.message);
            }
            eprintln!("axiom lint: {} problem(s) in {slug}", problems.len());
            1
        }
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
fn usage_command(args: &[String]) -> i32 {
    let (mut session, mut as_json, mut i) = (None, false, 0);
    while i < args.len() {
        match args[i].as_str() {
            "--session" => match value(args, &mut i, "--session") {
                Ok(v) => session = Some(v),
                Err(e) => return usage(e),
            },
            "--json" => as_json = true,
            s => return usage(format!("unrecognized option: {s}")),
        }
        i += 1;
    }
    match crate::usage::ledger(&crate::usage::Locations::from_env(), session.as_deref()) {
        Ok(ledger) if as_json => {
            println!("{}", serde_json::to_string_pretty(&ledger).unwrap());
            0
        }
        Ok(ledger) => {
            print!("{}", crate::usage::render(&ledger));
            0
        }
        Err(e) if e.kind == "invalid_argument" => usage(e.message),
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
fn evolve_command(args: &[String]) -> i32 {
    let (mut since, mut repo, mut dry_run, mut i) = (evolve::DEFAULT_SINCE_DAYS, None, false, 0);
    while i < args.len() {
        match args[i].as_str() {
            "--since" => match value(args, &mut i, "--since") {
                Ok(v) => match v.parse::<u32>() {
                    Ok(days) if (1..=evolve::MAX_SINCE_DAYS).contains(&days) => since = days,
                    _ => {
                        return usage(format!(
                            "--since must be an integer between 1 and {}",
                            evolve::MAX_SINCE_DAYS
                        ))
                    }
                },
                Err(e) => return usage(e),
            },
            "--repo" => match value(args, &mut i, "--repo") {
                Ok(v) => repo = Some(PathBuf::from(v)),
                Err(e) => return usage(e),
            },
            "--dry-run" => dry_run = true,
            s => return usage(format!("evolve: unrecognized argument: {s}")),
        }
        i += 1;
    }
    let inputs = evolve::Inputs::from_env(since);
    if dry_run {
        return match evolve::preview(&inputs) {
            Ok(preview) => {
                print!("{}", preview.text);
                0
            }
            Err(e) if e.kind == "invalid_argument" => usage(e.message),
            Err(e) => {
                eprintln!("axiom: {}", e.message);
                1
            }
        };
    }
    if let Some(path) = &mut repo {
        if !path.is_absolute() {
            *path = match std::env::current_dir() {
                Ok(cwd) => cwd.join(&*path),
                Err(e) => {
                    eprintln!("axiom: {e}");
                    return 1;
                }
            };
        }
    }
    match evolve::start(
        &inputs,
        repo.as_deref(),
        &session::Config::from_env(),
        Box::new(io::BufReader::new(io::stdin())),
        &mut io::stdout(),
    ) {
        Ok(evolve::StartOutcome::NoOp { reason }) => {
            println!("{reason}");
            0
        }
        Ok(evolve::StartOutcome::Ran { exit_code, .. }) => exit_code,
        Err(e) if e.kind == "invalid_argument" => usage(e.message),
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
        "wait" => wait_command(rest),
        #[cfg(feature = "app")]
        "app" => match crate::desktop::parse_options(rest) {
            Ok(options) => crate::desktop::run(options),
            Err(message) => usage(message),
        },
        #[cfg(not(feature = "app"))]
        "app" => usage("this build has no desktop app; build with --features app"),
        "doctor" if rest.is_empty() => doctor::doctor(&mut io::stdout()),
        "bridge" if rest.is_empty() => bridge::serve(
            bridge::Config::from_env(),
            io::BufReader::new(io::stdin()),
            io::stdout(),
        ),
        "hook" if rest.is_empty() => hook::main_stdin(),
        "orch" => orch_command(rest),
        "memory" => memory_command(rest),
        "lint" => lint_command(rest),
        "usage" => usage_command(rest),
        "evolve" => evolve_command(rest),
        "review" => crate::review::command(rest),
        _ => usage(format!("unknown command or arguments: {cmd}")),
    }
}
