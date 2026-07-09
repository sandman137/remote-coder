#!/usr/bin/env bash
# Build the desktop libhelm_engine.so and generate foreign-language bindings
# (DESIGN.md §8, §12 Phase 8). Kotlin + Swift for the mobile clients; Python
# for the runnable on-Linux FFI proof (no emulator, no JVM toolchain needed).
#
# Output: .dev/ffi/{python,kotlin,swift}/
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.dev/ffi"
SO="$ROOT/target/debug/libhelm_engine.so"

echo "building libhelm_engine.so + uniffi-bindgen…"
(cd "$ROOT" && cargo build -p engine-ffi)

gen() {
  local lang="$1" dir="$OUT/$1"
  mkdir -p "$dir"
  (cd "$ROOT" && cargo run -q -p engine-ffi --bin uniffi-bindgen -- \
    generate --library "$SO" --language "$lang" --out-dir "$dir")
  echo "  generated $lang bindings -> $dir"
}

gen python
gen kotlin
gen swift

echo "done. FFI bindings in $OUT"
