use crate::{
    base::Res,
    desktop::ux::seams::{Draft, Interjections, ReceiptStage},
    research_workspace::{
        annotations::{self, AnnotationStore, Correction, CorrectionDraft, ReceiptRef, Verdict},
        index::Index,
        manifest::Located,
    },
    seams::Clock,
};
use std::{path::Path, sync::Arc};

pub fn resolve_session(l: &Located, goals_home: Option<&Path>) -> Option<String> {
    l.manifest
        .session
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| {
            let project = l.store.as_ref()?.file_name()?.to_str()?;
            crate::goal::find_active(goals_home?, project)
                .ok()
                .flatten()
                .map(|(id, _)| id)
        })
}
pub(crate) fn who() -> String {
    ["USERNAME", "USER"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "you".into())
}
pub(crate) struct State {
    pub(crate) store: AnnotationStore,
    pub(crate) session: Option<String>,
    pub(crate) bridge: Option<Arc<dyn Interjections>>,
}
impl State {
    pub(crate) fn spot(
        &mut self,
        ix: &Index,
        target: &str,
        node: Option<&str>,
        verdict: Verdict,
        clock: &dyn Clock,
    ) -> Res<()> {
        self.store
            .spot_check(ix, target, node, verdict, &who(), clock)?;
        Ok(())
    }
    fn send(&mut self, c: &Correction, text: String) -> Res<()> {
        let (Some(session), Some(bridge)) = (&self.session, &self.bridge) else {
            return Ok(());
        };
        let (client_id, _) = annotations::reverify_request(c, session);
        let d = Draft {
            session: session.clone(),
            client_id: client_id.clone(),
            text: text.clone(),
            pause_first: false,
            replan: false,
        };
        if bridge.send(&d).is_ok() {
            self.store.set_receipt(
                &c.id,
                ReceiptRef {
                    client_id,
                    session: session.clone(),
                    stage: "sent".into(),
                    detail: None,
                },
            )?;
        } else {
            self.store.keep_draft(&c.id, &text)?;
        }
        Ok(())
    }
    pub(crate) fn correct(&mut self, ix: &Index, d: CorrectionDraft, clock: &dyn Clock) -> Res<()> {
        let c = self.store.correct(ix, d, clock)?;
        self.store
            .set_reverify(&c.id, annotations::reverify(ix, &c))?;
        if let Some(session) = &self.session {
            let (_, text) = annotations::reverify_request(&c, session);
            self.send(&c, text)?;
        }
        Ok(())
    }
    pub(crate) fn retry(&mut self, id: &str) -> Res<()> {
        if let Some(c) = self
            .store
            .corrections(None)
            .into_iter()
            .find(|c| c.id == id && c.receipt.is_none())
        {
            if let Some(text) = c.draft.clone() {
                self.send(&c, text)?;
            }
        }
        Ok(())
    }
    pub(crate) fn poll(&mut self) -> Res<()> {
        let Some(bridge) = self.bridge.clone() else {
            return Ok(());
        };
        for c in self.store.corrections(None) {
            let Some(mut r) = c.receipt else {
                continue;
            };
            if r.stage != "sent" && r.stage != "received" {
                continue;
            }
            if let Some(found) = bridge.receipt(&r.client_id) {
                let (stage, detail) = match found.stage {
                    ReceiptStage::Sent => ("sent", None),
                    ReceiptStage::Received => ("received", None),
                    ReceiptStage::Applied { first_action } => ("applied", Some(first_action)),
                    ReceiptStage::Rejected { reason } => ("rejected", Some(reason)),
                };
                if r.stage != stage || r.detail != detail {
                    r.stage = stage.into();
                    r.detail = detail;
                    self.store.set_receipt(&c.id, r)?;
                }
            }
        }
        Ok(())
    }
}
