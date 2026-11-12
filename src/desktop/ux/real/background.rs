//! Real BackgroundService over the 2a service client; owned by module service-storage-seam.
use crate::{
    base::{Failure, Res},
    desktop::ux::seams::{BackgroundService, KeepAlive},
    service::client::Client,
};
use std::path::PathBuf;

/// Wishes live in the service. The window never owns them, so detaching only drops the window's
/// own subscription and never touches the service.
pub struct ServiceBackground {
    pub home: PathBuf,
}

impl ServiceBackground {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    fn client(&self) -> Res<Client> {
        Client::connect(&self.home)
    }
}

impl BackgroundService for ServiceBackground {
    fn status(&self) -> KeepAlive {
        match self.client() {
            Ok(_) => KeepAlive::Available,
            Err(error) => KeepAlive::Unavailable {
                reason: error.message,
            },
        }
    }

    fn detach(&self, _sessions: &[String]) -> Res<()> {
        // The service keeps the wishes running; only prove it is still there.
        self.client()
            .map(|_| ())
            .map_err(|e| Failure::new("unavailable", e.message))
    }

    fn reattach(&self) -> Res<Vec<String>> {
        Ok(self
            .client()?
            .snapshot()?
            .wishes
            .into_iter()
            .map(|wish| wish.session.unwrap_or(wish.handle))
            .collect())
    }

    fn quit(&self, stop_running: bool) -> Res<()> {
        self.client()?.quit(stop_running)
    }
}
