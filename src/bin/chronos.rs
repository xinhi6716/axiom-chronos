// No console window when started from the desktop. Given a command, it behaves as the `axiom` CLI,
// so one program can serve as its own bridge and hooks.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let desktop = args.is_empty() || args[0].starts_with("--");
    if !desktop {
        std::process::exit(axiom::cli::main(std::env::args_os().collect()));
    }
    match axiom::desktop::parse_options(&args) {
        Ok(options) => std::process::exit(axiom::desktop::run(options)),
        Err(message) => {
            eprintln!("chronos: {message}");
            std::process::exit(2);
        }
    }
}
