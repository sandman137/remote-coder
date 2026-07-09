# Connecting Remote Coder to your coding sessions

Remote Coder lets your phone watch and steer the AI coding agents (Claude Code,
Codex, Cursor's agent, …) running in a `tmux` session on your dev machine.

**The phone needs nothing but the app.** Install `app-release.apk`, scan a QR,
done. No VPN, no accounts. The QR carries an address your phone can dial; the
host launcher picks the best one automatically:

| Where your agents run | How the phone reaches it | You do |
|---|---|---|
| Cloud box / VPS (public IP) | directly | `scripts/rcoder-host.sh` |
| Laptop behind NAT | reverse tunnel through any box you own | `scripts/rcoder-host.sh --via user@cloudbox` |
| Tailnet (optional) | Tailscale on both ends | `scripts/rcoder-host.sh` (auto-falls back) |

---

## 1. Host — start the session + pairing QR

On the machine where your agents run:

```bash
# once: build the two small binaries
cargo build --release -p engine-cli -p broker

# every session: agents tmux session + broker + pairing QR
scripts/rcoder-host.sh
```

This will:

- create a `tmux` session named **`agents`** (if it doesn't exist),
- start a dedicated, **broker-only** SSH endpoint on port `8022` — its own
  host key and `authorized_keys` under `.rcoder/`, never touching `~/.ssh`,
- print a **QR code** and wait for one phone to pair.

Then, in another terminal, attach and launch your agent(s):

```bash
tmux attach -t agents
#   inside: run  claude   (or codex / cursor 'agent')
#   open more windows (Ctrl-b c) — each becomes a pane in the app
```

> Different session or port: `RC_SESSION=work RC_PORT=9022 scripts/rcoder-host.sh`
> Force an address (e.g. a DNS name): `RC_HOST=dev.example.com scripts/rcoder-host.sh`

### Laptop behind NAT (`--via`)

```bash
scripts/rcoder-host.sh --via me@my-vps
```

The laptop opens an outbound reverse tunnel to your cloud box; the QR carries
the cloud box's address. SSH still **terminates on the laptop** — the cloud box
only forwards encrypted bytes. One-time setup on the cloud box:
`GatewayPorts clientspecified` in `/etc/ssh/sshd_config`, then restart sshd.

---

## 2. Phone — install + pair

1. Install **Remote Coder** (`app-release.apk`) — tap the file, allow install.
2. Open it → **Scan QR** → point at your host terminal.

On pairing the app generates a hardware-backed SSH key (never leaves the
phone), enrolls its public key over the one-time token, pins the host's key
fingerprint, then connects and lists your agent panes.

Tap a pane to watch it live. The controls mirror Claude Code's own prompt:

- **Key row**: `Esc` (interrupt) · `Mode` (Shift-Tab — cycles manual /
  auto-accept / plan) · `↑ ↓` (history) · `Enter` · `Ctrl-C`. When an agent
  asks a question, the row swaps to its actual choices (**Yes / No / 1 / 2**…).
- **📎 Attachments**: pick a photo or file; it uploads to the host over the
  same SSH channel (sandboxed to `~/.rcoder/uploads`, 16 MB cap) and its path
  drops into your prompt — Claude Code reads paths natively, so images reach
  its vision.
- **Dictation**: the input line is a normal Android text field — your
  keyboard's mic (Gboard voice typing) works, plus a dedicated 🎤 button.
- **Send** = text + Enter, like Claude Code's prompt box.

---

## 3. Managing devices

```bash
scripts/rcoder-host.sh --revoke <device-name>   # unpair a phone
rcoder devices --authorized-keys .rcoder/authorized_keys
```

Revocation takes effect on the phone's next reconnect.

---

## Security model (why a public port is OK here)

- The broker sshd accepts **public keys only** — no passwords, no root, and
  every paired key is installed with a
  `command="broker --session agents",no-pty,no-port-forwarding,…` forced
  command. A paired phone can invoke **only** the broker, scoped to that one
  tmux session — no shell, no forwarding. Port scanners find a door with no
  keyhole. See `DESIGN.md §5`.
- The host key is pinned at pair time; a swapped/MITM'd endpoint fails the pin.
- The enroll listener runs **only while pairing** (single-use token, 10-min
  TTL, one device, then it exits). Treat the QR like a password while it's on
  screen. Hardening tracked: sealing enrollment to the host key so even an
  on-path attacker in that window gets nothing.
- `--via` mode: the relay box sees only SSH ciphertext; the session terminates
  on your laptop.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `Can't find an address the phone could reach` | Laptop behind NAT → use `--via user@cloudbox`, or set `RC_HOST`. |
| `--via` says port not reachable | Add `GatewayPorts clientspecified` to the cloud box's sshd_config, restart sshd. |
| App can't connect after scan | Host script still running? Cloud firewall/security-group allows TCP 8022–8023? |
| `tmux is required` | `apt install tmux` / `brew install tmux`. |
| No panes listed | `tmux attach -t agents` and start an agent; each window shows up as a pane. |
| Pairing token expired | Re-run `scripts/rcoder-host.sh` for a fresh QR. |
