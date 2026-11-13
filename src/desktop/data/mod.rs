#[cfg(test)]
mod accept;
pub mod faults;
pub mod feed;
pub mod io;
pub mod outbox;

use crate::base::Res;
use crate::desktop::components::panel::PanelState;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct Snapshot<T> {
    pub value: Option<Arc<T>>,
    pub version: u64,
    pub state: PanelState,
    pub updated_ms: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor(pub u64);
pub enum Delta<T> {
    Unchanged,
    Replace(T),
    Patch(Box<dyn FnOnce(&mut T) + Send>),
}
pub trait Source<T>: Send + 'static {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<T>, Cursor)>;
    fn missing_is_empty(&self) -> bool {
        true
    }
    fn stale_after(&self) -> Option<Duration> {
        None
    }
}
pub use feed::Feed;
pub use outbox::{Outbox, Outcome, Ticket, TicketState};

/// Call once for each newly received orchestrator event, immediately before
/// `Projection::apply` in `desktop/live/feed.rs::Feed::poll`.
pub use super::perf::event_received;
