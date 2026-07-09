//! UniFFI surface (Phase 8). Kept building from Phase 0 so the CI portability
//! guard (`cargo check --target aarch64-linux-android`) covers it from day one.

pub fn engine_version() -> String {
    engine::version().to_string()
}
