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
    desktop::ux::{
        seams::{approval_outbox_for, interjection_outbox_for, unit_action_outbox_for},
        Seams,
    },
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
    let interjections = Arc::new(EngineInterjections::new(
        home.into(),
        Arc::new(ServiceTransport::new(home.into())),
    ));
    let approvals = Arc::new(EngineApprovals::new(home.into()));
    let unit_actions = Arc::new(OrchUnitActions::new(home.into()));
    Seams {
        dual_core: Some(Arc::new(EventDualCore::new(home.into()))),
        turn_policy: Some(Arc::new(CodexTurnPolicy::new(slots_home.into()))),
        interjections: Some(interjections.clone()),
        approvals: Some(approvals.clone()),
        approval_outbox: Some(approval_outbox_for(approvals)),
        interjection_outbox: Some(interjection_outbox_for(interjections)),
        unit_action_outbox: Some(unit_action_outbox_for(unit_actions.clone())),
        storage: Some(Arc::new(LedgerStorage::new(home))),
        background: Some(Arc::new(ServiceBackground::new(home))),
        modes: Some(Arc::new(ModesEngine::new(
            home,
            Arc::new(ModelProvider::new(home.into())),
            Arc::new(ModelRouter::new(home.into())),
            wish,
            Arc::new(LazyQuick { home: home.into() }),
        ))),
        unit_actions: Some(unit_actions),
        sessions: Some(Arc::new(ServiceSessions::new(home.into()))),
        optimizer: None,
        github: None,
        routines: Some(Arc::new(super::routines::StoreRoutines::new(home.into()))),
        paper: Some(Arc::new(super::paper::SessionPaper::new(home))), // p8:
    }
}
