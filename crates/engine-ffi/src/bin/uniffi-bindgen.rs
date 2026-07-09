//! Bindings generator (DESIGN.md §8). Invoked by scripts/build-desktop-ffi.sh:
//!   cargo run --bin uniffi-bindgen -- generate --library <so> --language <l>
fn main() {
    uniffi::uniffi_bindgen_main()
}
