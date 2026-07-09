# Remote Coder — Android client (Phase 9)

A thin Jetpack Compose consumer of the Rust `engine` via the UniFFI Kotlin
bindings. **All product logic lives in Rust** (transport, tmux protocol, VT
grid, adapters, attention, security); this module is UI + platform shims
(camera/QR, biometrics, FCM, foreground service, deep links).

## What this module contains

| Area | File(s) |
|---|---|
| Engine wrapper (owns the Rust `RemoteCoderEngine`, republishes events as Flow) | `engine/RemoteCoderRepository.kt` |
| Biometric gate (§8.4) | `engine/DeviceKeystore.kt` |
| UI state + event handling | `ui/RemoteCoderViewModel.kt` |
| Canvas grid renderer (256/truecolor, cursor, reflow → viewport) | `ui/GridView.kt` |
| Session list / pane / pairing screens | `ui/Screens.kt`, `ui/PairingScreen.kt` |
| QR pairing (CameraX + ML Kit) → `pair_enroll` FFI | `ui/PairingScreen.kt` |
| Foreground service + persistent notification (Live-Activity equiv.) | `notify/SessionForegroundService.kt` |
| FCM receiver → deep-link on tap, code-free payloads | `notify/FcmService.kt` |
| Single-activity host, deep links, STT input | `MainActivity.kt` |

The Rust cdylib and Kotlin bindings are produced automatically by Gradle:
`app/build.gradle.kts` wires `:cargoNdkBuild` (→ `jniLibs/<abi>/libremotecoder_engine.so`)
and `:uniffiBindgen` (→ generated `uniffi/remotecoder_engine`) into `preBuild`.

## Build & run

Prerequisites: Android SDK (API 34), NDK (r26+), `cargo-ndk`
(`cargo install cargo-ndk`), the `x86_64-linux-android` +
`aarch64-linux-android` Rust targets, `ANDROID_NDK_HOME` set. A
`local.properties` with `sdk.dir` / `ndk.dir` (gitignored — machine paths).

```sh
cd android
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
./gradlew :app:assembleDebug            # full APK, both ABIs (~364 MB debug)
```

`preBuild` runs cargo-ndk (→ jniLibs/<abi>/libremotecoder_engine.so) and
uniffi-bindgen (→ generated Kotlin) automatically.

**Lean build for the emulator** — a single ABI + release-stripped native lib
(~77 MB, the `.so` drops from ~150 MB debug to ~8.5 MB):

```sh
./gradlew :app:assembleDebug -Prcoder.rustAbis=x86_64 -Prelease
```

Emulator flow (DESIGN.md §9): the app reaches the host over the emulator's
`10.0.2.2` alias (host loopback). Pair with
`rcoder pair --pair-host 10.0.2.2 --ssh-port 2222` on the host and scan the QR;
the device key is generated on-device, the host key pinned, and a
forced-command key installed host-side (`broker` scopes to the `agents`
session). A debug-only `remotecoder://connect` deep link drives the same
pair+connect path without the camera, for automated testing:

```sh
adb shell am start -a android.intent.action.VIEW -d remotecoder://connect \
  --es payload '<pair-json>' --es device emu -n com.remotecoder/.MainActivity
```

## Verification status

- **Toolchain + build: verified on this machine.** SDK 34 + NDK r26 +
  cargo-ndk + Gradle 8.9 installed; `./gradlew :app:assembleDebug` produces a
  working APK that packages `libremotecoder_engine.so` (the real Rust engine) + JNA
  for arm64-v8a and x86_64, launchable as `com.remotecoder/.MainActivity`. The
  engine cross-compiles to both Android ABIs (ring crypto included) — the
  §13 portability guard is green with the real NDK. Building the APK caught
  three real defects (FfiError `message`↔`Throwable.message` clash, a Compose
  `Card`/`weight` misuse, the missing x86_64 Rust target), all fixed.
- **Engine behavior: fully green on Linux** — 115 Rust tests + the Python FFI
  driver exercising the identical FFI surface these Kotlin bindings expose
  (connect, snapshot with color, streaming via poll + callback, send-keys,
  press-button). Per DESIGN.md §3, a green engine over
  `SshTransport`-to-loopback ≈ a green Android app over the tailnet.
- **Emulator run: verified end to end on an Apple Silicon Mac.** The original
  Linux authoring host has no hardware virtualization (`/dev/kvm` absent, no
  `vmx`/`svm` flags), so its emulator can't boot usably (QEMU TCG is too slow;
  the modern emulator also refuses ARM64 images on x86_64 hosts). On an
  Apple Silicon Mac (arm64 image, Hypervisor.framework) the emulator boots in
  seconds and the full flow runs:
  1. `rcoder pair --pair-host <linux-tailnet-ip> --ssh-port <port>` on the dev
     host (the broker sshd + `agents` tmux session);
  2. `remotecoder://connect` on the emulator → the app generates its device key,
     enrolls over the tailnet, and pins the host key;
  3. the app connects via russh → the SSH forced command runs
     `broker --session agents` (scoped, least-privilege) → the app lists the
     live panes, streams a pane (colors, cursor, `tokens:` chip, adapter
     Yes/No buttons, "⚠ waiting" attention), and **Yes** round-trips a
     keystroke through the broker into tmux (the fake agent advances,
     tokens 137→274). This is the exact Remote Coder topology: phone (emulator) →
     remote dev host over the tailnet, through the broker.

## Remaining hardening (tracked)

- **Hardware-backed SSH key.** The device key is generated by the Rust
  `FileKeyStore` into app-private storage (non-exportable off-device;
  `allowBackup=false`). Moving it into Android Keystore/StrongBox with
  `setUserAuthenticationRequired(true)` requires a russh custom-signer
  callback across the FFI so the SSH handshake signs in hardware.
  `DeviceKeystore.kt` provides the biometric gate that plugs into this.
- **FCM registration handshake.** `FcmService.onNewToken` persists the token;
  delivering it to the host notifier over the secure channel during pairing
  is the last wiring step (the payload/deep-link path is complete).
- **google-services.json.** Add it and enable the `com.google.gms.google-services`
  plugin (commented in `app/build.gradle.kts`) to activate real FCM; until
  then the notifier's ntfy sink drives notifications.
