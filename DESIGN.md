# Remote Coder — tmux Agent Remote

**A secure, portable remote control for tmux-hosted coding agents (Claude Code, Codex, Cursor CLI, …).**

> Codename `Remote Coder` is a placeholder — rename freely. This document is the authoritative implementation spec. It is written to be executed **phase by phase**; each phase is independently buildable, has explicit acceptance criteria, and is **fully testable on a Linux dev machine** with zero Android involvement until the very last phase.

---

## 0. TL;DR for the implementer

- **What we build:** a thin remote control for coding agents that run inside `tmux` panes on a dev host. The phone does **not** emulate a terminal — `tmux` is the terminal emulator; we render the grid it produces and send keys back.
- **Architecture:** one **Rust core engine** (transport + tmux protocol + VT grid model + agent adapters + attention detection) exposed to native UIs via **UniFFI**. Android UI (Jetpack Compose) is a thin consumer. iOS (SwiftUI) is a future consumer of the *same* engine — do **not** build it now, but do not break its portability.
- **The local-test trick:** the engine has a `Transport` abstraction with a `LocalTransport` (drives tmux on the same host, no SSH) and an `SshTransport` (russh). A desktop **`ratatui` TUI harness** drives the identical engine and renders the identical grid, so the *entire product experience* — sessions, streaming, approve/reject buttons, attention notifications, adapters — is exercised on Linux. `SshTransport` is tested against **loopback sshd** so the remote path is proven without a second machine or a phone.
- **Security posture (hard requirement):** no public ingress (Tailscale/WireGuard or SSH-behind-tailnet), key-only auth, per-device keys in hardware keystore, an SSH **forced-command broker** that whitelists tmux subcommands + scopes to an `agents:` session prefix, host-key pinning (TOFU via QR), per-device revocation, and push payloads that carry **only** `{session_id, state}` — never code.

**Non-goals (do not build):** a terminal emulator app, a file browser / editor / git UI, any cloud copy of code, iOS (yet), Windows host support (Linux/macOS host only).

---

## 1. System architecture

```
┌───────────────────────────┐         WireGuard / Tailnet          ┌────────────────────────────────────┐
│   Client (Android now,     │  (no public ingress; OS-level VPN)   │            Dev host                 │
│    iOS later, TUI for dev) │                                      │                                     │
│                            │        SSH (russh, key-only)         │  sshd ──(forced cmd)── broker ──┐   │
│  ┌──────────────────────┐  │  ───────────────────────────────►    │                                 ▼   │
│  │  Native UI (Compose /│  │                                      │                              tmux srv│
│  │  SwiftUI / ratatui)  │  │                                      │        ┌───────────────────────┐    │
│  └─────────▲────────────┘  │                                      │        │ session "agents"      │    │
│            │ UniFFI         │                                      │        │  ├─ pane: claude code │    │
│  ┌─────────┴────────────┐  │                                      │        │  ├─ pane: codex       │    │
│  │  Remote Coder ENGINE (Rust)  │◄─┼──── control-mode / capture-pane ─────┼───────►│  └─ pane: cursor cli  │    │
│  │  • Transport trait   │  │                                      │        └───────────────────────┘    │
│  │  • tmux protocol     │  │        push (session_id+state only)  │  notifier ◄── Claude Code hook /    │
│  │  • VT grid model     │  │  ◄─────────────────────────────────  │            tmux monitor-silence     │
│  │  • Adapter registry  │  │       ntfy(self-host) / FCM / APNs   │                                     │
│  │  • Attention engine  │  │                                      │                                     │
│  └──────────────────────┘  │                                      │                                     │
└───────────────────────────┘                                      └────────────────────────────────────┘
```

**Key boundary:** the security-critical and platform-agnostic code (transport, protocol parsing, grid, adapters, crypto/pinning) lives once in Rust. Platform shims (secure key storage, biometrics, push registration, foreground lifecycle) are thin and hidden behind Rust traits with a desktop impl for testing.

---

## 2. Repository layout (Cargo workspace)

```
rcoder/
├── Cargo.toml                  # workspace
├── justfile                    # dev entrypoints (see §11)
├── DESIGN.md                   # this file
├── rust-toolchain.toml
├── crates/
│   ├── engine/                 # THE core library (no I/O to native; pure-ish)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── transport/      # Transport trait + Local + Ssh impls
│   │   │   ├── tmux/           # control-mode parser, capture-pane, command builder
│   │   │   ├── grid/           # VT grid model (vte-based) + SGR-only fast path
│   │   │   ├── adapter/        # AgentAdapter model, TOML loader, built-ins
│   │   │   ├── attention/      # tiered attention detection
│   │   │   ├── security/       # host-key pinning, keystore trait, pairing types
│   │   │   ├── event.rs        # EngineEvent enum + event bus
│   │   │   └── engine.rs       # Engine facade (public API)
│   │   └── tests/              # golden + integration tests
│   ├── engine-cli/             # DESKTOP HARNESS: ratatui TUI + headless CLI
│   ├── broker/                 # SSH forced-command broker (host-side bin)
│   ├── notifier/               # host-side notify daemon + hook scripts
│   └── engine-ffi/             # UniFFI wrapper -> libremotecoder_engine.so + Kotlin/Swift
├── android/                    # Jetpack Compose app (Phase 9 only)
├── fixtures/
│   ├── control-mode/           # recorded tmux -C byte streams for golden tests
│   └── agents/                 # fake-agent shell scripts (yn, numbered, streaming)
├── scripts/
│   ├── dev-tmux.sh             # spin up a local "agents" session w/ fake agents
│   ├── dev-sshd.sh             # loopback sshd on 127.0.0.1:2222 with a test key
│   └── dev-ntfy.sh             # local ntfy for notification tests
└── adapters/                   # shipped .toml agent profiles (also embedded via include_str!)
    ├── claude-code.toml
    ├── codex.toml
    └── cursor.toml
```

**Toolchain / crates (pin the current versions at scaffold time — do not trust these numbers blindly):**

| Concern | Crate | Notes |
|---|---|---|
| Async runtime | `tokio` | full features |
| SSH client | `russh` + `russh-keys` | pure-Rust, cross-compiles to Android/iOS cleanly; implements host-key pinning in `check_server_key` |
| VT parsing | `vte` (lean) **or** `alacritty_terminal` (full grid) | start with `vte` + a small custom `Grid`; escalate to `alacritty_terminal` only if TUIs misrender |
| Desktop TUI | `ratatui` + `crossterm` | dev harness only, not shipped to mobile |
| Config | `serde`, `toml` | adapter profiles |
| Regex | `regex` | attention patterns, metadata extractors |
| FFI | `uniffi` (>= async-capable release) | async fns + callback interfaces; verify async support in the version chosen |
| Errors/log | `thiserror`, `anyhow`, `tracing`, `tracing-subscriber` | |
| Desktop keystore (test impl) | `keyring` | stands in for Android Keystore in tests |
| QR (host side) | `qrcode` | render pairing payload |
| Android Rust build | `cargo-ndk` (tooling) | Phase 9 |

> **Version discipline:** at Phase 0, run `cargo add` for each and record the resolved versions in `Cargo.toml`. Confirm `uniffi`'s async + callback-interface support in the chosen version; if unavailable, fall back to the callback-only event model in §7.4.

---

## 3. The Transport abstraction (this is what makes local testing work)

Everything the engine does is expressed as either (a) one-shot tmux commands, or (b) one long-lived control-mode channel. Both go through `Transport`, which has two implementations selected at connect time.

```rust
// crates/engine/src/transport/mod.rs
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Run a single tmux invocation, return combined stdout (already newline-normalized).
    async fn exec(&self, argv: &[String]) -> Result<Vec<u8>, TransportError>;

    /// Open the streaming control-mode channel: `tmux -C attach-session -t <session>`.
    /// Returns a duplex: we write tmux commands as lines, we read %-notification lines.
    async fn open_control(&self, session: &str, size: (u16, u16))
        -> Result<Box<dyn ControlChannel>, TransportError>;
}

#[async_trait::async_trait]
pub trait ControlChannel: Send {
    async fn write_line(&mut self, line: &str) -> Result<(), TransportError>;
    /// Yields raw control-mode lines (still %-prefixed, octal-escaped payloads intact).
    async fn read_line(&mut self) -> Result<Option<Vec<u8>>, TransportError>;
}
```

### 3.1 `LocalTransport` — no SSH, same host

`exec` spawns `tmux <argv>` via `tokio::process::Command`. `open_control` spawns `tmux -C attach-session -t <session>` with piped stdin/stdout. **This is the default transport for all local tests and the TUI harness.** It exercises 100% of the tmux protocol, grid, adapter, and attention code paths with no network, no SSH, no keys.

### 3.2 `SshTransport` — russh, the real remote path

`exec` opens an SSH session/exec channel and runs the tmux argv (or the broker forced-command). `open_control` opens a channel and runs `tmux -C attach …`; the channel's stdout/stdin become the `ControlChannel`. Host-key pinning is enforced in `russh`'s `check_server_key` against the fingerprint captured during pairing (§8.3).

> **Local test for SshTransport without a phone or second box:** point it at **loopback sshd** (`scripts/dev-sshd.sh` → `127.0.0.1:2222`). The desktop TUI can run over `SshTransport` to localhost and behave exactly as it will over the tailnet. This proves the entire remote stack — auth, pinning, control-mode-over-SSH, broker — on one Linux machine.

**Because the engine only ever sees `Transport`, the TUI, the JVM FFI test, and the Android app all run the same code. A green TUI over `SshTransport`-to-loopback ≈ a green Android app over the tailnet.**

---

## 4. tmux protocol layer

Two interaction modes; both required.

### 4.1 Snapshot mode (Phase 1 MVP) — `capture-pane` + `send-keys`

- **Render:** `tmux capture-pane -t <pane> -p -e` for the visible screen; `-S -<N>` prepended for scrollback. `-e` includes SGR (color/attr) escapes; **cursor positioning is already resolved into a grid by tmux**, so this path needs only an **SGR parser**, never a VT state machine. This is the literal realization of "no terminal emulator."
- **Input:** `tmux send-keys -t <pane> …`. Keys are sent as tmux key names for specials (`Enter`, `Escape`, `C-c`, `Up`, `BSpace`, `Tab`) and as `-l` literal for text.
- **Enumeration:** `list-sessions -F …`, `list-windows -t <s> -F …`, `list-panes -t <w> -F …` with explicit `-F` format strings (see §4.4).

Polling cadence: on-demand (user opens a pane) + a light timer (e.g. 500 ms) while a pane is focused. Snapshot mode is enough to be a *usable* remote for agent approve/reject and is the fastest path to something real.

### 4.2 Streaming mode (Phase 3) — control mode `tmux -C`

- Enter with `tmux -C attach-session -t agents` (iTerm's `-CC` disables echo; `-C` is sufficient for a programmatic client — verify against local tmux ≥ 3.2).
- **Notifications to parse:** `%begin/%end/%error` (wrap command replies, tagged by command number), `%output %<pane> <data>`, `%window-add`, `%window-close`, `%unlinked-window-add`, `%window-renamed`, `%session-changed`, `%layout-change <win-id> <layout>`, `%pane-mode-changed`, `%exit`, `%client-detached`.
- **`%output` payloads are OCTAL-ESCAPED**: non-printable bytes appear as `\ooo` and backslash is doubled. **The parser MUST unescape** before feeding the VT model. This is the single most common correctness bug — cover it with a golden test.
- **`%layout-change`** carries the tmux layout string (e.g. `bd5b,80x24,0,0,3`): `checksum,WxH,x,y,paneid`, nested with `[` `]` (vertical splits) and `{` `}` (horizontal). Parse it to a pane geometry tree for multi-pane rendering.
- Because `%output` is the **raw pane byte stream** (cursor moves, clears, alt-screen), streaming mode **does require a VT state machine** — use the `grid` module (§5). Do **not** hand-roll the VT parser; drive `vte`/`alacritty_terminal`.

### 4.3 Mobile reflow (important UX detail)

Agent TUIs (Claude Code boxes, Cursor menus) assume ~80–120 cols; a phone is ~40–55 cols. Solution: set the **control client size** to the client viewport so tmux reflows the pane for us:

- Set client size on attach and on device rotation: `refresh-client -C <cols>x<rows>` (or `-C -t <client> WxH` depending on tmux version — verify).
- Set the window option so tmux honors the latest client rather than the smallest: `set-window-option -t <win> window-size latest` (or `aggressive-resize on`). Document the tradeoff: if the desktop is *also* attached to the same window, sizes fight; recommend the phone attach to the `agents` session while the desktop uses a *different* session, or use `window-size latest`.

### 4.4 Format strings (stable enumeration, avoid scraping)

Use explicit `-F` so parsing is trivial and version-stable, e.g.:

```
list-panes -a -F '#{session_name}\t#{window_index}\t#{pane_id}\t#{pane_title}\t#{pane_current_command}\t#{pane_active}\t#{pane_width}x#{pane_height}'
```

Filter to the `agents` session prefix in the broker (§8.2), not just client-side.

---

## 5. Grid model (`crates/engine/src/grid`)

- **Public type:** `GridSnapshot { cols, rows, cells: Vec<Cell>, cursor: (u16,u16) }`, `Cell { ch: char, fg: Color, bg: Color, attrs: CellAttrs }`. This is what the UI renders and what UniFFI exposes (as a flattened, FFI-friendly struct — see §7.3).
- **Two producers:**
  - **SGR fast path** (snapshot mode): parse `capture-pane -e` lines → grid. Only `\e[…m` SGR handling; positions come from line/column in the captured text.
  - **VT path** (streaming mode): feed unescaped `%output` bytes into a `vte::Parser` (or `alacritty_terminal::Term`) that maintains the grid; snapshot it on each flush. Handles alt-screen, scroll regions, clears, cursor moves.
- **Diffing:** emit `EngineEvent::Grid` only on change; compute a cheap dirty-row set to keep FFI traffic and mobile redraw small.
- **Colors:** support 16 / 256 / truecolor. Map to `Color::{Indexed(u8), Rgb(u8,u8,u8), Default}`.

---

## 6. Agent adapters (`crates/engine/src/adapter`) — the generic plugin mechanism

The mechanical layer (transport/grid/keys) is agent-agnostic and needs **no** per-agent code. Adapters add only the *semantic sugar*: how to launch an agent, how to know it's waiting, which buttons to show, and what metadata to surface. Adapters are **declarative TOML** (shipped + user-overridable), so adding a new agent later is a config drop, not an app release.

### 6.1 Adapter schema

```toml
# adapters/cursor.toml
id      = "cursor"
name    = "Cursor CLI"
launch  = { cmd = "agent", args = [], cwd = "picker" }   # runs `agent` in a fresh pane

# tier-3 attention: regexes matched against recent pane text
attention = [
  '\(y/n\)',
  'Apply this edit\?',
  'Run command\?',
]

# quick-action buttons: label -> literal keys sent via send-keys
[[buttons]]
label = "Yes"
keys  = "y\n"
[[buttons]]
label = "No"
keys  = "n\n"
[[buttons]]
label = "Always"
keys  = "a\n"

# optional metadata extractors for notifications / header chips
[[metadata]]
field = "cost"
regex = 'tokens:\s*(\d+)'
[[metadata]]
field = "tool"
regex = '●\s*(\w+)'

# optional tier-1 hook script name (host-side); empty => rely on tier-2/3
hook  = ""

# transport kind: "tmux" today; "acp" reserved for structured agents (§6.4)
transport = "tmux"
```

Built-ins (`claude-code.toml`, `codex.toml`, `cursor.toml`) are embedded via `include_str!` **and** overridable from a config dir (`$XDG_CONFIG_HOME/remote-coder/adapters/*.toml`). User files win on `id` collision.

### 6.2 Loader + model

```rust
pub struct AgentAdapter {
    pub id: String,
    pub name: String,
    pub launch: LaunchSpec,                 // cmd, args, cwd policy
    pub attention: Vec<regex::Regex>,       // tier-3
    pub buttons: Vec<Button>,               // label -> keys
    pub metadata: Vec<MetaExtractor>,       // field <- regex capture 1
    pub hook: Option<String>,               // tier-1 script name
    pub transport: AdapterTransport,        // Tmux | Acp
}

pub struct Registry { adapters: HashMap<String, AgentAdapter> }
impl Registry {
    pub fn load_builtins_and_overrides() -> Result<Self>;
    pub fn get(&self, id: &str) -> Option<&AgentAdapter>;
    /// Best-effort auto-detect from `pane_current_command` (e.g. "claude", "codex", "agent").
    pub fn detect(&self, current_command: &str, recent_text: &str) -> Option<&AgentAdapter>;
}
```

### 6.3 Confirmed scope

Any TUI/CLI that runs in a pane is drivable with **zero** adapter work (a plain shell just has no attention rules/buttons). Claude Code, Codex, and **Cursor CLI** (its interactive `agent` command with a slash-command menu; also a headless `-p` mode) all run in a pane and are in scope. Adapters exist only to make attention detection + buttons *nice*, not to make them *work*.

### 6.4 ACP (future, `transport = "acp"`)

Some agents (e.g. Cursor) speak the **Agent Client Protocol** — a structured host↔agent protocol carrying turn/permission/tool events. Reserve an `AdapterTransport::Acp` variant so a future adapter can subscribe to *structured* approval events instead of regex-scraping, while everything else falls back to the tmux path. **Do not implement ACP now** — just don't design it out. Attention detection and buttons for ACP agents would be exact rather than heuristic.

---

## 7. Public engine API + event model (`engine.rs`, `event.rs`)

### 7.1 Facade

```rust
pub struct Engine { /* transport, registry, grid caches, attention, event bus */ }

impl Engine {
    pub async fn connect(cfg: ConnConfig) -> Result<Engine>;      // picks Local vs Ssh
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    pub async fn list_panes(&self, session: &str) -> Result<Vec<PaneInfo>>;
    pub async fn attach(&self, pane: PaneId, size: (u16,u16)) -> Result<()>; // begins streaming
    pub async fn snapshot(&self, pane: PaneId, scrollback: u32) -> Result<GridSnapshot>;
    pub async fn send_keys(&self, pane: PaneId, keys: KeyInput) -> Result<()>;
    pub async fn press_button(&self, pane: PaneId, button_label: &str) -> Result<()>; // adapter map
    pub async fn resize(&self, pane: PaneId, cols: u16, rows: u16) -> Result<()>;
    pub async fn launch_agent(&self, session: &str, adapter_id: &str, cwd: Option<String>) -> Result<PaneId>;
    pub fn subscribe(&self) -> EventStream;                        // broadcast
}

pub enum ConnConfig {
    Local,                                    // LocalTransport
    Ssh { host: String, port: u16, user: String,
          key: KeyRef, hostkey_fp: String },  // SshTransport + pinning
}
```

### 7.2 Events

```rust
pub enum EngineEvent {
    Sessions(Vec<SessionInfo>),
    Panes { session: String, panes: Vec<PaneInfo> },
    Grid { pane: PaneId, snapshot: GridSnapshot, dirty_rows: Vec<u16> },
    Attention { pane: PaneId, agent: String, kind: PromptKind, buttons: Vec<Button> },
    Metadata { pane: PaneId, fields: HashMap<String,String> },
    Exited { pane: PaneId, status: Option<i32> },
    Reconnecting, Connected, Error(String),
}
```

### 7.3 FFI-shaped types

UniFFI can't export generics or trait objects. Keep the FFI surface **concrete and flat**: `GridSnapshot` uses `Vec<CellFfi>` with primitive fields; colors are `enum ColorFfi { Default, Indexed(u8), Rgb(u8,u8,u8) }`; no lifetimes, no `&`. The `Transport` trait stays fully internal.

### 7.4 Event delivery across FFI

Prefer UniFFI **async methods** + a **callback interface** `EngineListener { fn on_event(e: EngineEventFfi) }`. Native side registers a listener; engine pushes events. If the chosen UniFFI version lacks async, expose `poll_events() -> Vec<EngineEventFfi>` and drive it from a native coroutine/loop. Decide in Phase 8 based on the resolved version.

---

## 8. Security design (hard requirements)

### 8.1 Transport security

- **No public ingress.** Primary: Tailscale/WireGuard; the engine just dials a tailnet IP. Fallback: SSH reachable only through the tailnet. Never expose sshd to the public internet in docs or defaults.
- **Key-only SSH**, ed25519. No passwords, no keyboard-interactive.
- **Host-key pinning (TOFU):** the fingerprint is delivered out-of-band via the pairing QR; `russh` `check_server_key` rejects mismatches → MITM protection even on a compromised network.

### 8.2 Forced-command broker (`crates/broker`) — least privilege

The phone's key is installed with a forced command so it can **never** open a general shell:

```
# ~/.ssh/authorized_keys on the host
command="/opt/rcoder/broker",no-port-forwarding,no-x11-forwarding,no-agent-forwarding,no-pty ssh-ed25519 AAAA... rc-phone-<deviceid>
```

`broker` reads `$SSH_ORIGINAL_COMMAND`, tokenizes, and allows only:

- `list-sessions`, `list-windows`, `list-panes` (with `-F`), **scoped** so any `-t` target must match the `agents:` (configurable) session prefix.
- `capture-pane -t agents:… …` (read-only flags only).
- `send-keys -t agents:… …`.
- `-C attach-session -t agents…` (control mode, scoped).
- `resize-window` / `refresh-client -C` (size only).

Everything else → exit non-zero, log, deny. The session-prefix scope is enforced by the broker, not trusted from the client.

> **Honest caveat to document in-code:** `send-keys` can type arbitrary text into whatever pane it targets, so if an agent pane is sitting at a shell prompt, keys reach the shell. Containment therefore = (a) broker scoping to the `agents:` session, (b) keeping agents in dedicated panes, and (c) revocable per-device keys. The broker prevents *shell channels and out-of-scope sessions*; it does not claim to sandbox agent behavior itself. State this plainly; don't oversell.

`broker` is a normal Rust bin → **unit-testable on Linux** by setting `SSH_ORIGINAL_COMMAND` and asserting allow/deny (§10.4). No SSH needed for those tests.

### 8.3 Pairing + revocation

QR payload (JSON, shown by a host-side `rcoder pair` command):

```json
{ "v":1, "host":"100.x.y.z", "port":22, "user":"dev",
  "hostkey_fp":"SHA256:…", "enroll":"<one-time-token>", "ttl":600 }
```

Flow: app scans → generates ed25519 keypair **on device** (private key never leaves the secure element) → connects using the one-time enroll token → host appends the device pubkey to `authorized_keys` with the forced command → token invalidated. **Revoke** = remove that line (or `rcoder revoke <deviceid>`). Store `hostkey_fp` on device for pinning.

### 8.4 Key storage (trait + platform impls)

```rust
pub trait KeyStore: Send + Sync {
    fn generate_device_key(&self, alias: &str) -> Result<PublicKey>;   // private stays in HW
    fn sign(&self, alias: &str, data: &[u8]) -> Result<Signature>;     // biometric-gated on mobile
    fn pinned_hostkey(&self, host: &str) -> Result<Option<String>>;
    fn pin_hostkey(&self, host: &str, fp: &str) -> Result<()>;
}
```

- Android impl: Android Keystore / StrongBox, `setUserAuthenticationRequired(true)` (biometric), key non-exportable.
- iOS impl (future): Keychain + Secure Enclave.
- **Desktop test impl:** `keyring` crate + a temp file for pins — lets the whole pairing/pinning/signing path be tested on Linux.

### 8.5 Notification privacy

Push payload = `{ session_id, pane_id, state:"waiting"|"done"|"error", agent }` **only**. Never include pane text, code, prompts, or file paths — those go through FCM/APNs/Google/Apple servers. The client fetches any detail over the secure channel on tap. This preserves the "no cloud copy" property.

---

## 9. Notifications (`crates/notifier`)

Tiered "agent needs attention" detection; the client is pushed only when it isn't actively attached.

- **Tier 1 — native hook (most reliable).** Claude Code `Notification` / permission (`PreToolUse`) hooks in `~/.claude/settings.json` invoke `notifier notify --session agents --state waiting`. `notifier` posts to the configured sink. For Codex/Cursor, use a hook if/when available; otherwise fall back.
- **Tier 2 — tmux signal.** `set-hook`/`monitor-silence` (output stalls ⇒ likely waiting) and `monitor-bell`. Engine subscribes via control-mode `%pane-mode-changed`/alerts, or a small host daemon runs `tmux wait-for`/pipe-pane.
- **Tier 3 — regex.** Adapter `attention` patterns matched against recent pane text (works for *any* agent with zero host integration).

**Sinks:** self-hosted **ntfy** for local dev (`scripts/dev-ntfy.sh`), **FCM** for Android, **APNs** for iOS later. `notifier` is sink-pluggable. Local test: run ntfy on localhost, fire the hook, assert the received message body contains **no** code and only the allowed fields (§10.5).

Client-side: Android **foreground service** + persistent notification mirroring `{agent, state}` (the equivalent of an iOS Live Activity). Tapping deep-links to the pane.

---

## 10. Testing strategy (everything green on Linux)

### 10.1 Layers

1. **Unit tests** — SGR parser, octal unescape, layout-string parser, adapter TOML loader, attention matcher, broker allow/deny, key names for `send-keys`.
2. **Golden tests** — recorded control-mode byte streams in `fixtures/control-mode/*.bin` replayed through the parser + VT grid; assert `GridSnapshot`. Deterministic, no tmux needed → CI-safe.
3. **Integration tests** — spawn a **real tmux server on a private socket** (`tmux -L rc-test -f /dev/null`), run a **fake agent** script in a pane, drive it through `LocalTransport`, assert grid + attention + button behavior. Requires tmux on the dev box only.
4. **Loopback SSH tests** — `SshTransport` → `127.0.0.1:2222` (dev sshd) → broker → tmux. Proves auth, pinning, control-mode-over-SSH, and broker scoping on one machine.
5. **FFI tests** — a **JVM/Kotlin test on desktop** loads the desktop-built `libremotecoder_engine.so` via the generated UniFFI Kotlin bindings (JNA-based, runs on desktop JVM) and drives the engine. Proves the FFI boundary **without an Android emulator**.

### 10.2 Fake agents (`fixtures/agents/`)

- `fake-yn.sh`: prints work lines, then `Proceed? (y/n) ` and blocks on a char; loops.
- `fake-numbered.sh`: prints `1) apply  2) skip  3) abort` and reads a number.
- `fake-stream.sh`: emits colored, alt-screen, cursor-moving output to exercise the VT path.
- These stand in for real agents so tests are deterministic and offline. Real `claude` / `codex` / `agent` binaries are used in optional, non-CI "smoke" runs.

### 10.3 Golden capture helper

`just record-fixture <name>` runs a fake agent under `tmux -C`, tees the raw control-mode stream to `fixtures/control-mode/<name>.bin`, so new scenarios become regression tests.

### 10.4 Broker tests (pure, no SSH)

```rust
#[test] fn broker_denies_shell() {
    assert!(!broker::authorize("bash -i", "agents").allowed());
}
#[test] fn broker_scopes_session() {
    assert!( broker::authorize("capture-pane -t agents:0.0 -p", "agents").allowed());
    assert!(!broker::authorize("capture-pane -t other:0.0 -p", "agents").allowed());
}
```

### 10.5 Notification privacy test

Fire the hook against local ntfy, capture the delivered payload, assert it equals the whitelist of fields and contains none of the known code markers from the fake agent's output.

---

## 11. Dev entrypoints (`justfile`)

```make
# --- local, no SSH, no Android ---
fake-session:      # start tmux `agents` with fake agents in panes
	scripts/dev-tmux.sh

tui:               # THE dev surface: full product UX over LocalTransport
	cargo run -p engine-cli -- --transport local --session agents

headless:          # scriptable engine CLI (list/snapshot/send)
	cargo run -p engine-cli -- --transport local {{ARGS}}

# --- prove the remote path on ONE machine ---
sshd:              # loopback sshd on 127.0.0.1:2222 w/ test key + broker
	scripts/dev-sshd.sh
tui-ssh:           # same TUI, but over SshTransport -> loopback -> broker -> tmux
	cargo run -p engine-cli -- --transport ssh --host 127.0.0.1 --port 2222 --session agents

# --- notifications ---
ntfy:              scripts/dev-ntfy.sh
test-notify:       cargo test -p notifier --features local-ntfy

# --- tests ---
test:              cargo test --workspace
golden:            cargo test -p engine --test golden
ffi-jvm:           # build desktop .so + run Kotlin FFI test on the JVM (no emulator)
	scripts/build-desktop-ffi.sh && ./gradlew -p engine-ffi/jvm-test test

# --- Android (Phase 9 only) ---
android-so:        cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build --release -p engine-ffi
android-emu:       # x86_64 emulator on the Linux box; reach host tmux via adb reverse
	adb reverse tcp:2222 tcp:2222 && ./gradlew -p android installDebug
```

**Daily loop for you:** `just fake-session` once, then iterate with `just tui` (instant, local). Periodically `just tui-ssh` to confirm the remote/broker path, and `just test`. Android never enters the loop until Phase 9, and even then runs on the emulator against `adb reverse` loopback.

---

## 12. Phase plan

Each phase: **Goal → Deliverables → Acceptance (automated) → Local verification (manual)**. Do not start a phase until the prior phase's acceptance is green. Every phase through 8 is Linux-only.

### Phase 0 — Scaffold
- **Goal:** workspace compiles; dev scripts exist.
- **Deliverables:** Cargo workspace (§2), `justfile`, `scripts/dev-tmux.sh`, fake-agent scripts, `rust-toolchain.toml`, CI running `cargo test --workspace`.
- **Acceptance:** `cargo build --workspace` and `cargo test --workspace` pass (empty tests OK). `just fake-session` creates a tmux `agents` session with 2 fake-agent panes.
- **Local verify:** `tmux -L default attach -t agents` shows the fake agents.

### Phase 1 — Engine core: LocalTransport + snapshot mode
- **Goal:** enumerate sessions/panes, render a pane via `capture-pane`, send keys — same host.
- **Deliverables:** `Transport` trait, `LocalTransport`, tmux command builder + `-F` parsers, **SGR parser → GridSnapshot**, `send_keys`, `Engine::{connect(Local), list_sessions, list_panes, snapshot, send_keys}`.
- **Acceptance:** unit tests for SGR parser + key-name mapping; integration test drives `fake-yn.sh` and asserts the snapshot contains the prompt text; `send_keys("y\n")` advances it.
- **Local verify:** `just headless -- list` prints panes; `just headless -- snapshot agents:0.0` prints the grid.

### Phase 2 — Desktop TUI harness (the app-minus-Android)
- **Goal:** a real, usable remote in the terminal that drives the engine.
- **Deliverables:** `engine-cli` ratatui TUI: session/pane list, grid view (renders `GridSnapshot`), an approve/reject/enter/ctrl-c button row (keybinds), a text-input line, scrollback view. Polls snapshots while focused.
- **Acceptance:** headless snapshot golden test still green; a scripted TUI test (feed keystrokes, assert the fake agent advanced) passes.
- **Local verify:** `just tui` → navigate to the `fake-yn` pane, hit the **Yes** button, watch it proceed. **This is the product experience, on Linux.**

### Phase 3 — Control-mode streaming + VT grid
- **Goal:** live, low-latency updates; correct rendering of alt-screen agent TUIs.
- **Deliverables:** control-mode client in `LocalTransport::open_control`, `%`-notification parser, **octal unescape**, layout-string parser, VT grid via `vte`/`alacritty_terminal`, `Engine::attach` emitting `EngineEvent::Grid` with dirty rows, mobile reflow via client-size set.
- **Acceptance:** golden tests over `fixtures/control-mode/*.bin` (including an octal-escaped payload and an alt-screen sequence) assert exact grids; integration test attaches to `fake-stream.sh` and observes streaming updates.
- **Local verify:** `just tui` streams the fake agent live; resizing the terminal reflows the pane.

### Phase 4 — Adapter registry + attention detection
- **Goal:** per-agent buttons + "is waiting" events, declaratively.
- **Deliverables:** `AgentAdapter` model + TOML loader + built-ins (claude-code, codex, cursor) via `include_str!` + user overrides; tiered attention (regex tier-3 now; monitor-silence tier-2; hook tier-1 wired but may be inert until Phase 7); `press_button`, `Attention`/`Metadata` events.
- **Acceptance:** loader tests (built-in + override precedence); attention tests using `fake-yn`/`fake-numbered` assert correct `Attention` events + button sets; detect-by-`pane_current_command` test.
- **Local verify:** `just tui` shows agent-specific buttons; the header chip shows extracted metadata; the pane flips to an "attention" state when the fake agent waits.

### Phase 5 — SshTransport (russh) + loopback proof
- **Goal:** identical behavior over SSH.
- **Deliverables:** `SshTransport` (`exec` + `open_control`), ed25519 key auth, **host-key pinning** in `check_server_key`, `ConnConfig::Ssh`.
- **Acceptance:** loopback integration test: `SshTransport`→`127.0.0.1:2222`→tmux runs the Phase-1 and Phase-3 assertions unchanged; a pinning test rejects a wrong host key.
- **Local verify:** `just sshd` then `just tui-ssh` → the TUI behaves exactly as `just tui`. **Remote path proven on one machine.**

### Phase 6 — Broker + pairing + keystore
- **Goal:** least-privilege access, revocable devices, hardware-key abstraction.
- **Deliverables:** `broker` bin (whitelist + session scoping), `rcoder pair` (QR generate) + enroll flow, revocation, `KeyStore` trait + desktop impl (`keyring`).
- **Acceptance:** pure broker allow/deny tests (§10.4); an end-to-end loopback test where the phone key is installed with `command="broker"` and a denied command fails while an allowed one succeeds; a pairing round-trip test using the desktop keystore.
- **Local verify:** `just sshd` with the broker as forced command; `just tui-ssh` still works; attempting an out-of-scope `capture-pane -t other:…` is rejected (add a `--raw` debug flag to the CLI to try it).

### Phase 7 — Notifications
- **Goal:** get pinged when an agent waits; zero code in payloads.
- **Deliverables:** `notifier` bin, Claude Code hook script (tier-1), tmux monitor-silence daemon (tier-2), ntfy sink for dev, FCM sink stub for Phase 9, privacy-filtered payloads.
- **Acceptance:** privacy test (§10.5); a test that a tier-3 regex match produces a notify call when no client is attached; hook-script smoke test against local ntfy.
- **Local verify:** `just ntfy`; trigger `fake-yn` to wait; receive an ntfy push containing only `{session,pane,state,agent}`.

### Phase 8 — UniFFI bindings + JVM FFI test
- **Goal:** the engine is callable from Kotlin/Swift; the FFI boundary is proven on Linux.
- **Deliverables:** `engine-ffi` crate: UniFFI scaffolding, FFI-shaped types (§7.3), `EngineListener` callback interface (or `poll_events`), desktop `.so` build, generated Kotlin bindings, a **JVM test** driving the engine over LocalTransport.
- **Acceptance:** `just ffi-jvm` runs a Kotlin test that connects, snapshots, sends keys, and receives events through the FFI — **no emulator**.
- **Local verify:** JVM test green; Swift binding *generation* checked in CI (no build) to keep iOS portability honest.

### Phase 9 — Android app (only now touch a device/emulator)
- **Goal:** ship the Android client.
- **Deliverables:** Compose UI (session/pane list, grid renderer, button row, text+STT input, scrollback), `cargo-ndk` build of `libremotecoder_engine.so` into `jniLibs`, Android `KeyStore` impl (StrongBox + biometric), FCM registration + foreground service + persistent notification, QR scanner for pairing, deep-link on notification tap.
- **Acceptance:** app builds; on the **x86_64 emulator** it connects via `adb reverse tcp:2222 tcp:2222` to the loopback sshd/broker/tmux and reproduces the TUI flows; biometric-gated signing works; a waiting fake agent produces an FCM (or ntfy) notification with no code.
- **Local verify:** everything runs on the Linux box's emulator against loopback — no physical device, no repeated "download to Android."

### Phase 10 — Future (do not build now, do not design out)
- iOS SwiftUI client reusing `engine-ffi` (Swift bindings) + iOS `KeyStore`/APNs shims.
- `AdapterTransport::Acp` for structured agents (exact approval events).
- Multi-host / multi-session dashboard; Live Activity; watch complications.

---

## 13. Cross-cutting requirements & edge cases (bake into code + tests)

- **Reconnect/resume:** control-mode channel drops (network blip, phone sleep) must resume: re-attach, re-request layout, re-snapshot; emit `Reconnecting`/`Connected`. Test by killing the loopback SSH channel mid-stream.
- **Octal unescape** in `%output` — dedicated golden fixture.
- **Reflow contention** when desktop + phone attach the same window — document the `window-size latest` recommendation; test that setting client size changes the reported pane width.
- **Large scrollback** — cap `capture-pane -S` and paginate; never stream unbounded history to mobile.
- **Truecolor + 256-color** SGR — fixtures for each.
- **Unicode / wide chars / combining** — grid cells must handle width; add a CJK/emoji fixture.
- **Send-keys safety** — literal text via `-l`; never shell-interpolate user input into the tmux argv (build argv arrays, never format a shell string).
- **Broker is the trust boundary** — all scoping enforced host-side; client requests are untrusted.
- **No code in logs at info level** — pane bytes only at `trace`, gated; default subscriber must not print pane content.
- **Portability guard in CI** — build `engine`/`engine-ffi` for an Android target (and generate Swift bindings) every CI run so a JVM-only or Linux-only dependency can't sneak into the core.

---

## 14. Definition of done (whole project, pre-iOS)

- `just test` green (unit + golden + integration + loopback + broker + notify + JVM-FFI).
- `just tui` and `just tui-ssh` deliver the full remote experience on Linux.
- Android emulator app reproduces the flows over loopback with biometric-gated keys and code-free push.
- Adding a new agent = drop a `.toml`; no engine changes.
- Security invariants hold: no public ingress, key-only, forced-command scoping, host-key pinning, revocable devices, payloads carry no code.
- `engine` core has **no** platform-specific code; all platform behavior sits behind traits with a desktop impl.
