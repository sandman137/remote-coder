# HELM dev entrypoints (DESIGN.md §11).
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
    cargo run -p engine-cli -- --transport ssh --host 127.0.0.1 --port 2222 --session agents tui

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

# portability guard: engine must build for Android targets (no NDK needed for check)
check-android:
    cargo check -p engine -p engine-ffi --target aarch64-linux-android

# build desktop .so + run Kotlin FFI test on the JVM — no emulator (Phase 8)
ffi-jvm:
    scripts/build-desktop-ffi.sh && ./gradlew -p crates/engine-ffi/jvm-test test

# --- Android (Phase 9 only) ---

android-so:
    cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build --release -p engine-ffi

android-emu:
    adb reverse tcp:2222 tcp:2222 && ./gradlew -p android installDebug
