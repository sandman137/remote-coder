#!/usr/bin/env bash
# Build the desktop .so + Python bindings, then drive the engine across the
# FFI boundary (DESIGN.md §12 Phase 8 acceptance). No emulator, no JVM.
#
# The design specifies a JVM/Kotlin FFI test; this box has no Kotlin/Gradle
# toolchain, so we use UniFFI's Python (ctypes) bindings as the runnable
# on-Linux proof of the identical FFI surface. The Kotlin bindings are still
# generated (Android consumes them) and a Kotlin driver is checked in at
# crates/engine-ffi/tests/FfiDriver.kt for hosts that have kotlinc.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET="rc-ffi-$$"

"$ROOT/scripts/build-desktop-ffi.sh" >/dev/null

export PYTHONPATH="$ROOT/.dev/ffi/python"
# The ctypes bindings dlopen libremotecoder_engine.so by name from the lib dir.
export LD_LIBRARY_PATH="$ROOT/target/debug:${LD_LIBRARY_PATH:-}"
# UniFFI's Python loader looks for the cdylib next to the module or on the
# path; symlink it beside the generated module.
ln -sf "$ROOT/target/debug/libremotecoder_engine.so" "$ROOT/.dev/ffi/python/libremotecoder_engine.so"

echo "running Python FFI driver…"
python3 "$ROOT/crates/engine-ffi/tests/ffi_driver.py" "$SOCKET"
