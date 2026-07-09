# HELM — tmux Agent Remote

A secure, portable remote control for tmux-hosted coding agents (Claude Code,
Codex, Cursor CLI, …). The phone does **not** emulate a terminal — tmux is the
terminal emulator; we render the grid it produces and send keys back.

**[DESIGN.md](DESIGN.md) is the authoritative spec.** One Rust core engine
(transport + tmux protocol + VT grid + agent adapters + attention detection)
exposed to native UIs via UniFFI; Android (Jetpack Compose) is a thin consumer,
with a desktop ratatui TUI as the dev harness so the entire product is
exercised on Linux.

## Layout

| Path | What |
|---|---|
| `crates/engine` | The core library: Transport (Local/SSH), tmux protocol, grid, adapters, attention |
| `crates/engine-cli` | Desktop harness: ratatui TUI + headless CLI |
| `crates/broker` | SSH forced-command broker (host-side least privilege) |
| `crates/notifier` | Host-side notify daemon + hook scripts |
| `crates/engine-ffi` | UniFFI wrapper → `libhelm_engine.so` + Kotlin/Swift bindings |
| `adapters/` | Declarative agent profiles (TOML) |
| `fixtures/` | Fake agents + recorded control-mode streams for golden tests |
| `scripts/` | Dev entrypoints: local tmux session, loopback sshd, local ntfy |

## Quickstart (Linux)

```sh
just fake-session   # tmux session "agents" with fake agents in panes
just tui            # the product experience, in your terminal (Phase 2+)
just test           # unit + golden + integration tests
```

Prove the remote path on one machine:

```sh
just sshd           # loopback sshd on 127.0.0.1:2222
just tui-ssh        # same TUI over SSH -> localhost
```

## Status

Built phase by phase per [DESIGN.md §12](DESIGN.md). Phases 0–8 are complete
and fully tested on Linux (**115 Rust tests + a Python FFI driver**, all
green); Phase 9's Android source is complete and builds with the Android
toolchain.

| Phase | Scope | Status |
|---|---|---|
| 0 | Workspace scaffold, dev scripts, CI | ✅ |
| 1 | LocalTransport + snapshot mode | ✅ |
| 2 | Desktop TUI harness | ✅ |
| 3 | Control-mode streaming + VT grid | ✅ |
| 4 | Adapters + attention | ✅ |
| 5 | SshTransport + loopback proof | ✅ |
| 6 | Broker + pairing + keystore | ✅ |
| 7 | Notifications | ✅ |
| 8 | UniFFI + FFI driver (Python; Kotlin/Swift generated) | ✅ |
| 9 | Android app (Compose) | ✅ source; needs Android toolchain to build |

The entire product experience runs on Linux today: `just tui` (local) and
`just tui-ssh` (over SSH → loopback → broker) deliver sessions, live
streaming, approve/reject buttons, attention badges, and metadata chips. Per
DESIGN.md §3, a green engine over `SshTransport`-to-loopback ≈ a green Android
app over the tailnet — and the [Android client](android/) consumes the exact
same engine through the generated UniFFI bindings.
