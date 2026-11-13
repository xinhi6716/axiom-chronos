//! Production desktop seam adapters. Service connections are made on demand.
use super::{
    approvals::EngineApprovals,
    background::ServiceBackground,
    dual_core::EventDualCore,
    extreme::CodexTurnPolicy,
    interjections::{EngineInterjections, ServiceTransport},
    modes::{LazyQuick, ModesEngine},
    sessions::ServiceSessions,
    storage::LedgerStorage,
    unit_actions::OrchUnitActions,
};
use crate::{
    base::Res,
    desktop::ux::Seams,
    modes::runners::{ModelProvider, ModelRouter},
    service::{
        launcher::{ServiceLauncher, StartPath},
        Client, StartRequest,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct LazyStart(pub(super) PathBuf);
impl StartPath for LazyStart {
    fn start(&self, req: StartRequest) -> Res<String> {
        Client::connect(&self.0)?.start(req)
    }
}

pub fn production(home: &Path, slots_home: &Path) -> Seams {
    let wish = Arc::new(ServiceLauncher {
        home: home.into(),
        start: Arc::new(LazyStart(home.into())),
    });
    Seams {
        dual_core: Some(Arc::new(EventDualCore::new(home.into()))),
        turn_policy: Some(Arc::new(CodexTurnPolicy::new(slots_home.into()))),
        interjections: Some(Arc::new(EngineInterjections::new(
            home.into(),
            Arc::new(ServiceTransport::new(home.into())),
        ))),
        approvals: Some(Arc::new(EngineApprovals::new(home.into()))),
        storage: Some(Arc::new(LedgerStorage::new(home))),
        background: Some(Arc::new(ServiceBackground::new(home))),
        modes: Some(Arc::new(ModesEngine::new(
            home,
            Arc::new(ModelProvider::new(home.into())),
            Arc::new(ModelRouter::new(home.into())),
            wish,
            Arc::new(LazyQuick { home: home.into() }),
        ))),
        unit_actions: Some(Arc::new(OrchUnitActions::new(home.into()))),
        sessions: Some(Arc::new(ServiceSessions::new(home.into()))),
        optimizer: None,
        github: None,
        routines: Some(Arc::new(super::routines::StoreRoutines::new(home.into()))),
        paper: Some(Arc::new(super::paper::SessionPaper::new(home))), // p8:
    }
}
