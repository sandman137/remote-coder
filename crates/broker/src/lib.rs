//! Forced-command broker library. The allow/deny logic lives here (pure,
//! unit-testable without SSH — DESIGN.md §10.4); `main.rs` is a thin shell
//! around it. Implemented in Phase 6.

/// Placeholder so the crate has a testable surface in Phase 0.
pub fn crate_name() -> &'static str {
    "broker"
}
