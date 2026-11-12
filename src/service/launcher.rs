//! One start path for wishes launched by promotion and by routines, plus the
//! `Unavailable` provider/router used where no real model implementation exists.
use super::{Client, StartRequest};
use crate::{
    base::{Failure, Res},
    modes::{
        context::{self, DEFAULT_CONTEXT_BYTES},
        launch::{start_child, LaunchKind, LaunchRequest as ModeLaunch, Launched, Launcher},
        plaza::router::{Router, Routing},
        providers::{Ask, Provider, Reply},
        runs,
    },
    routines::{LaunchRequest as RoutineLaunch, WishLauncher},
};
use std::{path::PathBuf, sync::Arc};

pub trait StartPath: Send + Sync {
    fn start(&self, req: StartRequest) -> Res<String>;
}
impl StartPath for Client {
    fn start(&self, req: StartRequest) -> Res<String> {
        Client::start(self, req)
    }
}

pub struct ServiceLauncher {
    pub home: PathBuf,
    pub start: Arc<dyn StartPath>,
}
impl WishLauncher for ServiceLauncher {
    fn launch(&self, request: &RoutineLaunch) -> Res<String> {
        self.start.start(StartRequest {
            wish: request.wish.clone(),
            cwd: request.project_dir.to_string_lossy().into_owned(),
            ..Default::default()
        })
    }
}
impl Launcher for ServiceLauncher {
    fn launch(&self, req: &ModeLaunch) -> Res<Launched> {
        if req.kind != LaunchKind::Wish {
            return Err(Failure::new(
                "invalid_argument",
                "ServiceLauncher only starts wishes",
            ));
        }
        let session = runs::load_session(&self.home, &req.session)?;
        let cwd = session.project.ok_or_else(|| {
            Failure::new("invalid_argument", "wish promotion requires a project")
        })?;
        let mut wish = req.goal.clone();
        if let Some(snapshot) = &req.context {
            let background = context::bounded(&snapshot.messages, DEFAULT_CONTEXT_BYTES);
            if !background.trim().is_empty() {
                wish.push_str("\n\nBackground from the conversation:\n");
                wish.push_str(&background);
            }
        }
        let handle = self.start.start(StartRequest {
            wish,
            cwd: cwd.to_string_lossy().into_owned(),
            ..Default::default()
        })?;
        let run = start_child(&self.home, req)?;
        Ok(Launched {
            run,
            detail: serde_json::json!({ "handle": handle }),
        })
    }
}

/// Refuses before any model call: no real provider or router exists yet (D13).
pub struct Unavailable;
fn unavailable() -> Failure {
    Failure::new("unavailable", "no model provider is configured for this mode")
}
impl Provider for Unavailable {
    fn ask(&self, _: &Ask) -> Res<Reply> {
        Err(unavailable())
    }
}
impl Router for Unavailable {
    fn route(&self, _: &str, _: &str) -> Res<Routing> {
        Err(unavailable())
    }
}
