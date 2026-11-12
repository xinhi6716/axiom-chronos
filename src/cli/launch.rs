use crate::{
    base,
    session::launch::{self, LaunchSpec, Launched},
    storage::{
        preflight::{self, Kind},
        Class,
    },
};
use std::path::PathBuf;

pub fn command(args: &[String]) -> i32 {
    let (mut kind, mut need_bytes, mut session, mut progress_file, mut marker_file) =
        (None, None, None, None, None);
    let mut register = Vec::new();
    let mut i = 0;
    let mut separator = false;
    while i < args.len() {
        let flag = args[i].as_str();
        if flag == "--" {
            i += 1;
            separator = true;
            break;
        }
        if !matches!(
            flag,
            "--kind" | "--need" | "--session" | "--progress" | "--marker" | "--register"
        ) {
            return super::usage(format!("launch: unrecognized argument: {flag}"));
        }
        let v = match super::value(args, &mut i, flag) {
            Ok(v) => v,
            Err(e) => return super::usage(e),
        };
        match flag {
            "--kind" => {
                kind = Some(match v.as_str() {
                    "batch" => Kind::Batch,
                    "unpack" => Kind::Unpack,
                    "cache" => Kind::CacheBuild,
                    _ => return super::usage("--kind must be batch, unpack or cache"),
                })
            }
            "--need" => {
                need_bytes = match preflight::parse_size(&v) {
                    Some(n) => Some(n),
                    None => return super::usage("--need requires a valid size"),
                }
            }
            "--session" => session = Some(v),
            "--progress" => progress_file = Some(PathBuf::from(v)),
            "--marker" => marker_file = Some(PathBuf::from(v)),
            "--register" => {
                let Some((path, class)) = v.rsplit_once('=') else {
                    return super::usage("--register requires <dir>=<class>");
                };
                let Some(class) = Class::parse(class) else {
                    return super::usage("--register requires a valid class");
                };
                if path.is_empty() {
                    return super::usage("--register requires a directory");
                }
                register.push((PathBuf::from(path), class));
            }
            _ => unreachable!(),
        }
        i += 1;
    }
    let Some(kind) = kind else {
        return super::usage("launch requires --kind");
    };
    if !separator || i == args.len() || args[i].is_empty() {
        return super::usage("launch requires -- <argv...>");
    }
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axiom: {e}");
            return 1;
        }
    };
    let spec = LaunchSpec {
        kind,
        argv: args[i..].to_vec(),
        cwd,
        need_bytes,
        session,
        progress_file,
        marker_file,
        register,
    };
    match launch::launch(&base::axiom_home(), &*preflight::os_probe(), &spec) {
        Ok(Launched::Started { pid, .. }) => {
            println!(
                "{}",
                serde_json::json!({"pid": pid, "progress_file": spec.progress_file, "marker_file": spec.marker_file})
            );
            0
        }
        Ok(Launched::Paused { check }) => {
            println!("{}", check.line());
            75
        }
        Err(e) if e.kind == "invalid_argument" => super::usage(e.message),
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
