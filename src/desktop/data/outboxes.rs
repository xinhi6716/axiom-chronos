//! The outboxes in front of the approval, interjection and unit-action services: each runs its
//! service's call in the background and turns the reply into an outbox outcome.
use super::outbox::{Outbox, Outcome};
use crate::base::Failure;
use crate::desktop::ux::seams::{
    ApprovalService, Decision, Draft, Interjections, UnitAction, UnitActions,
};
use std::sync::Arc;

fn outbox_failure(error: Failure) -> Outcome {
    match error.kind.as_str() {
        "transient"
        | "offline"
        | "unreachable"
        | "unavailable"
        | "service_unavailable"
        | "timeout" => Outcome::Retry(error.to_string()),
        _ => Outcome::Refused(error.to_string()),
    }
}

pub fn approval_outbox_for(service: Arc<dyn ApprovalService>) -> Outbox<Decision> {
    Outbox::new(move |decision: &Decision, key| {
        if decision.key != key {
            return Outcome::Refused("idempotency key does not match decision".into());
        }
        match service.decide(decision) {
            Ok(_) => Outcome::Applied,
            Err(error) => outbox_failure(error),
        }
    })
}

pub fn interjection_outbox_for(service: Arc<dyn Interjections>) -> Outbox<Draft> {
    Outbox::new(move |draft: &Draft, key| {
        if draft.client_id != key {
            return Outcome::Refused("idempotency key does not match client id".into());
        }
        match service.send(draft) {
            Ok(_) => Outcome::Applied,
            Err(error) => outbox_failure(error),
        }
    })
}

pub fn unit_action_outbox_for(service: Arc<dyn UnitActions>) -> Outbox<UnitAction> {
    Outbox::new(move |action: &UnitAction, key| {
        if action.key != key {
            return Outcome::Refused("idempotency key does not match unit action".into());
        }
        match service.act(&action.session, &action.unit, &action.kind, key) {
            Ok(receipt) if receipt.accepted => Outcome::Applied,
            Ok(receipt) => Outcome::Refused(receipt.message),
            Err(error) => outbox_failure(error),
        }
    })
}
