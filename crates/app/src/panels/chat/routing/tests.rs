//! Routing decisions for a changed launch signature.

use super::{RoutingAction, routing_action};
use thinkrail_core::SessionPhase;

#[test]
fn unchanged_signature_keeps_running_session() {
    assert_eq!(
        routing_action(Some("same"), Some("same"), SessionPhase::Running),
        RoutingAction::Unchanged
    );
}

#[test]
fn changed_idle_session_restarts_now() {
    assert_eq!(
        routing_action(Some("old"), Some("new"), SessionPhase::Idle),
        RoutingAction::RestartNow
    );
}

#[test]
fn changed_running_session_waits_until_idle() {
    assert_eq!(
        routing_action(Some("old"), Some("new"), SessionPhase::Running),
        RoutingAction::Pending
    );
}
