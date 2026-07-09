# Remote Coder dev entrypoints (DESIGN.md §11).
# Daily loop: `just fake-session` once, then iterate with `just tui`.

# --- local, no SSH, no Android ---

# start tmux `agents` session with fake agents in panes
fake-session:
    scripts/dev-tmux.sh

# THE dev surface: full product UX over LocalTransport (Phase 2+)
tui:
    cargo run -p engine-cli -- --transport local --session agents tui

# scriptable engine CLI (list/snapshot/send) (Phase 1+)
headless *ARGS:
    cargo run -p engine-cli -- --transport local {{ARGS}}

# --- prove the remote path on ONE machine ---

# loopback sshd on 127.0.0.1:2222 with a test key (add --broker for Phase 6)
sshd *ARGS:
    scripts/dev-sshd.sh {{ARGS}}

# same TUI, but over SshTransport -> loopback -> tmux (Phase 5+)
tui-ssh:
    cargo run -p engine-cli -- --transport ssh --host 127.0.0.1 --port 2222 \
        --key .dev/sshd/client_ed25519 --session agents tui

# --- notifications ---

ntfy:
    scripts/dev-ntfy.sh

test-notify:
    cargo test -p notifier

# --- tests ---

test:
    cargo test --workspace

golden:
    cargo test -p engine --test golden

# record a control-mode fixture from a fake agent (Phase 3)
record-fixture NAME:
    scripts/record-fixture.sh {{NAME}}

# portability guard: the core must cross-compile to Android (DESIGN.md §13).
# Needs the NDK for russh's `ring` crypto backend — set ANDROID_NDK_HOME and
# use cargo-ndk (same toolchain as the Phase-9 build).
check-android:
    cargo ndk -t arm64-v8a check -p engine -p engine-ffi

# generate bindings (python/kotlin/swift) from the desktop .so (Phase 8)
ffi-bindings:
    scripts/build-desktop-ffi.sh

# build desktop .so + drive the engine across the FFI boundary — no emulator
# (Phase 8). Python (ctypes) bindings are the runnable on-Linux proof of the
# same FFI surface Android's Kotlin bindings expose.
ffi-jvm:
    scripts/run-ffi-test.sh

# --- Android (Phase 9 only) ---

android-so:
    cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build --release -p engine-ffi

android-emu:
    adb reverse tcp:2222 tcp:2222 && ./gradlew -p android installDebug
