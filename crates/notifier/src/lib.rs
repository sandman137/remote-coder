//! Notifier library: payload types + sinks. The privacy invariant (payloads
//! carry only `{session, pane, state, agent}` — never pane text) is enforced
//! by construction here. Implemented in Phase 7.

/// Placeholder so the crate has a testable surface in Phase 0.
pub fn crate_name() -> &'static str {
    "notifier"
}
