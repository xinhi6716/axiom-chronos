pub mod client;
pub mod launcher;
pub mod lock;
pub mod login;
pub mod protocol;
pub mod recover;
pub mod scheduler;
pub mod server;
pub mod tray;
pub use client::{Client, Subscription};
pub use protocol::{HostedWish, Snapshot, StartRequest, WireEvent};
pub use server::{RunningService, Server, ServerOptions};

use std::{sync::mpsc, thread};

pub fn forward_tray_actions(
    rx: mpsc::Receiver<tray::TrayAction>,
    to: mpsc::Sender<tray::TrayAction>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for action in rx {
            if to.send(action).is_err() {
                break;
            }
        }
    })
}

pub fn serve_command(args: &[String]) -> i32 {
    let mut login_launch = false;
    let mut no_tray = false;
    for arg in args {
        match arg.as_str() {
            "--login" if !login_launch => login_launch = true,
            "--no-tray" if !no_tray => no_tray = true,
            _ => {
                eprintln!("axiom: serve: unrecognized argument: {arg}");
                return 2;
            }
        }
    }
    let home = crate::base::axiom_home();
    let cfg_home = home.clone();
    let (tx, rx) = mpsc::channel();
    let options = server::ServerOptions {
        home,
        config: std::sync::Arc::new(move || {
            let mut c = crate::session::Config::from_env();
            c.home = cfg_home.clone();
            c
        }),
        tray: tray::platform_tray(tx),
        desktop: tray::platform_desktop(),
        login: login::platform(),
        login_launch,
    };
    match server::Server::start(options) {
        Ok(s) => {
            let _forwarder = forward_tray_actions(rx, s.tray_actions());
            s.wait()
        }
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            if e.kind == "already_running" {
                3
            } else {
                1
            }
        }
    }
}
