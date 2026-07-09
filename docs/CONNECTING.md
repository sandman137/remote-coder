# Connecting Remote Coder to your coding sessions

Remote Coder lets your phone watch and steer the AI coding agents (Claude Code,
Codex, Cursor's agent, …) running in a `tmux` session on your dev machine — over
your private Tailscale tailnet, with a key that can *only* reach the agent broker
and never a shell.

There are two sides: the **host** (where your agents run) and the **phone**.

---

## 1. Phone — one time

1. Install **Tailscale** from the Play Store and sign in to the same tailnet as
   your dev machine.
2. Install **Remote Coder** (`app-release.apk`) — tap the file and allow install
   from your browser/files app.

That's it on the phone until you pair (step 3).

---

## 2. Host — start the session + pairing QR

On the machine where your agents run (must be on the same tailnet):

```bash
# once: build the two small binaries
cargo build --release -p engine-cli -p broker

# every session: bring up the agents tmux session + broker + pairing QR
scripts/rcoder-host.sh
```

This will:

- create a `tmux` session named **`agents`** (if it doesn't exist),
- start a dedicated, broker-only SSH endpoint on your tailnet IP (default port
  `8022`) — it uses its **own** host key and `authorized_keys` under `.rcoder/`
  and does **not** touch your `~/.ssh`,
- print a **QR code** and wait for one phone to pair.

Then, in another terminal, attach the session and launch your agent(s):

```bash
tmux attach -t agents
#   inside: run  claude   (or codex, or your cursor/aider agent)
#   open more windows (Ctrl-b c) for more agents — each becomes a pane in the app
```

> Prefer a different name or port: `RC_SESSION=work RC_PORT=9022 scripts/rcoder-host.sh`

---

## 3. Pair

Open Remote Coder → **Scan QR** → point at the QR in your host terminal.

The app then, on-device:

- generates a hardware-backed SSH key (never leaves the phone),
- enrolls its **public** key with the host over the one-time token (valid 10 min),
- pins the host's key fingerprint,
- connects through the broker and lists your agent panes.

Tap a pane to watch it live — colors, cursor, and the agent's status. When an
agent asks a question, the app surfaces **Yes / No / Apply / Skip** buttons and
a keyboard; taps are delivered as real keystrokes into that pane.

---

## 4. Managing devices

```bash
scripts/rcoder-host.sh --revoke <device-name>   # unpair a phone
rcoder devices --authorized-keys .rcoder/authorized_keys   # list paired keys
```

Revocation takes effect on the phone's next reconnect.

---

## Security model (why this is safe to expose on a tailnet)

- The phone's key is installed into `authorized_keys` with a
  `command="broker --session agents",no-pty,no-port-forwarding,…` forced
  command. A paired phone can invoke **only** the broker, scoped to the one
  session — no shell, no forwarding, no other tmux session. See `DESIGN.md §5`.
- Transport is SSH over your tailnet; the host key is pinned at pair time, so a
  swapped endpoint fails the pin.
- Enrollment tokens are single-use and expire (default 10 min).
- Nothing listens on the public internet — only your tailnet.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `not on a tailnet` | Start Tailscale on the host (`tailscale up`). |
| App can't connect after scan | Confirm the phone is on the **same** tailnet (`tailscale status`), and the host script is still running. |
| `tmux is required` | `apt install tmux` / `brew install tmux`. |
| No panes listed | Attach `tmux -t agents` and start an agent in it; each window shows up as a pane. |
| Pairing token expired | Re-run `scripts/rcoder-host.sh` for a fresh QR. |
