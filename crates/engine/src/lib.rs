//! HELM engine — the platform-agnostic core.
//!
//! Everything security-critical and platform-independent lives here:
//! transport (local/SSH), tmux protocol, grid model, agent adapters,
//! attention detection. Native UIs (ratatui harness, Android, iOS)
//! consume this crate — directly or through `engine-ffi`.
//!
//! Phase 0: crate skeleton only. Modules land phase by phase (DESIGN.md §12).

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semverish() {
        let v = super::version();
        assert_eq!(
            v.split('.').count(),
            3,
            "expected MAJOR.MINOR.PATCH, got {v}"
        );
    }
}
