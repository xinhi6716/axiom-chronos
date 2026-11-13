fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // p8: Probe only for the acceptance-test engine; runtime discovery has no fallback.
    println!("cargo::rustc-check-cfg=cfg(paper_tectonic)");
    println!("cargo::rerun-if-env-changed=AXIOM_TECTONIC_CMD");
    println!("cargo::rerun-if-env-changed=PATH");
    if paper_tectonic_present() {
        println!("cargo::rustc-cfg=paper_tectonic");
    }
    // p8: end
    #[cfg(feature = "app")]
    embed_windows_resources();
}

// p8: Keep the probe dependency-free: extract argv[0] from the JSON command.
fn paper_tectonic_present() -> bool {
    use std::{env, path::Path};
    fn on_path(name: &str) -> bool {
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|dir| {
            if dir.join(name).is_file() {
                return true;
            }
            #[cfg(windows)]
            {
                return env::var_os("PATHEXT")
                    .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
                    .to_string_lossy()
                    .split(';')
                    .any(|ext| dir.join(format!("{name}{ext}")).is_file());
            }
            #[cfg(not(windows))]
            false
        })
    }
    let configured = env::var("AXIOM_TECTONIC_CMD")
        .ok()
        .and_then(|raw| {
            let mut chars = raw
                .trim_start()
                .strip_prefix('[')?
                .trim_start()
                .strip_prefix('"')?
                .chars();
            let mut command = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return Some(command),
                    '\\' => match chars.next()? {
                        '\\' => command.push('\\'),
                        '"' => command.push('"'),
                        '/' => command.push('/'),
                        _ => return None,
                    },
                    c => command.push(c),
                }
            }
            None
        })
        .is_some_and(|cmd| Path::new(&cmd).is_file() || on_path(&cmd));
    configured
        || on_path("tectonic")
        || Path::new("E:/codex/.toolchains/tectonic-0.17.0/tectonic.exe").is_file()
}
// p8: end

#[cfg(feature = "app")]
fn embed_windows_resources() {
    use std::env::var;
    println!("cargo:rerun-if-changed=assets/chronos.rc");
    println!("cargo:rerun-if-changed=assets/chronos.ico");
    let macros = [
        format!("VERSION_MAJOR={}", var("CARGO_PKG_VERSION_MAJOR").unwrap()),
        format!("VERSION_MINOR={}", var("CARGO_PKG_VERSION_MINOR").unwrap()),
        format!("VERSION_PATCH={}", var("CARGO_PKG_VERSION_PATCH").unwrap()),
        format!("VERSION_TEXT=\"{}\"", var("CARGO_PKG_VERSION").unwrap()),
    ];
    embed_resource::compile_for(
        "assets/chronos.rc",
        ["chronos"],
        embed_resource::ParamsMacrosAndIncludeDirs(macros, ["assets"]),
    )
    .manifest_required()
    .unwrap();
}
