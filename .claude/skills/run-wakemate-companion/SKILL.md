---
name: run-wakemate-companion
description: Build, run, and drive the WakeMATE Companion Rust server. Use when asked to start the companion, hit its HTTP/HTTPS API, test pairing or wake-on-LAN, run cargo tests, or check a change against the running service on macOS.
---

WakeMATE Companion is a Rust (axum) service that a phone talks to over the LAN.
On Windows it is a tray app; **on macOS it builds and runs as a headless
network service**, which is what you get here. Drive it with
`.claude/skills/run-wakemate-companion/smoke.sh` — it launches the server
against a throwaway config and HOME, asserts 21 real API behaviours, and shuts
it down. There is no GUI on this platform, so the HTTP API *is* the surface.

All paths below are relative to `WakeMATE-Companion/`.

## Prerequisites

Rust only — no system packages needed on macOS. Verified with:

```bash
cargo --version   # cargo 1.97.1
```

`openssl` and `curl` (both preinstalled on macOS) are used by the driver.

## Build

```bash
cargo build
```

~17s incremental, ~2min cold. Platform-gated dead-code warnings (`open_path`,
Windows tray helpers, …) are expected — those functions are
only called from the Windows-gated `tray.rs`.

## Run (agent path)

```bash
./.claude/skills/run-wakemate-companion/smoke.sh
```

Prints a PASS/FAIL line per assertion and exits non-zero if any fail. Last
verified run: **21 passed, 0 failed**. It covers the unauthenticated surface,
token auth (401/200), the TLS listener + SHA-256 pin, pairing refusal, WoL, and
capability gating.

Flags:

| Flag | Effect |
|---|---|
| `--keep-running` | Leaves the server up and prints the `curl` lines to poke it |
| `--input` | Sets `allow_input_commands: true` and exercises `mouse_move` — **this moves your real cursor 5px** |
| `--port N` | HTTP on N, HTTPS on N+1 (default 7787/7788) |

To poke it by hand:

```bash
./.claude/skills/run-wakemate-companion/smoke.sh --keep-running
curl -s -H 'x-wakemate-token: smoke-test-token' http://127.0.0.1:7787/v1/info
curl -sk https://127.0.0.1:7788/v1/health
kill <pid it printed>
```

Auth is the **`x-wakemate-token` header**. Putting the token in the JSON body
returns 401 — the body is only ever `device_name`/command fields.

### The endpoints

| Route | Auth | Notes |
|---|---|---|
| `GET /` | no | liveness string |
| `GET /v1/health` | no | `status`, `device_name`, `version`, `protocol_version` (4) |
| `GET /v1/info` | yes¹ | IP/MAC/subnet/broadcast + capability flags |
| `GET /v1/pairing/check` | yes | token probe |
| `GET /v1/pairing/status` | yes | `?device_id=` for per-device state |
| `POST /v1/pairing/enroll` | yes (shared token only) | **403 on macOS** |
| `POST /v1/pairing/activate` | yes | **403 on macOS** |
| `POST /v1/wake` | yes | `{"mac","broadcast?","port?"}` |
| `POST /v1/command` | yes | tagged union, `{"type": …}` |

¹ unless `require_auth_for_info: false`.

Command payloads that are easy to get wrong — these are the exact wire shapes:

```jsonc
{"type":"mouse_move","delta_x":10,"delta_y":10}          // NOT dx/dy
{"type":"mouse_button","button":"left","action":"down"}
{"type":"mouse_scroll","direction":"down","amount":2}
{"type":"key_press","key":"escape"}
{"type":"media","action":"play_pause"}                   // 400 on macOS, see Gotchas
{"type":"system","action":"sleep"}                       // DO NOT SEND, see Gotchas
{"type":"wake","mac":"2C:F0:5D:59:89:44","broadcast":"255.255.255.255"}
```

## Test

```bash
cargo test
```

51 tests, ~0.13s, all passing. These use an **in-memory fake credential store**
(`#[cfg(test)] mod backend` in `src/credential_store.rs`), so unlike the real
binary they never touch your Keychain.

## Gotchas

- **The macOS Keychain silently overrides the config file's `api_token`.**
  `hydrate_token()` reads the OS credential store *first*. Run the binary
  against your real `$HOME` once and it moves the token into your login
  keychain, rewrites the config with `"api_token": ""` and
  `"token_storage": "keyring"`, and from then on **whatever token you write
  into the config JSON is ignored**. Verified: seeded a config with
  `SECOND-token-xyz`, and the *previous* run's token still authenticated (200)
  while the new one got 401.

- **The fix — and the reason `smoke.sh` sets a fake `HOME`.** With
  `HOME=<scratch>`, the keyring backend fails with *"A default keychain could
  not be found"*, the app falls back to `TokenStorage::File`, and the config's
  token is used verbatim. Do this in anything that needs a predictable token.

- **Don't read the token back with the `security` CLI.** An earlier version of
  this driver used `security find-generic-password -w` and it popped a *"security
  wants to use your confidential information stored in WakeMATE Companion"*
  password dialog — `security` isn't the binary that created the item, so macOS
  challenges it. The fake-HOME approach avoids needing to read it at all.

- **`--config-path` does not isolate everything.** Only the config JSON follows
  that flag. `TlsIdentity` and `DeviceRegistry` go through `AppConfig::data_dir()`
  → `~/Library/Application Support/WakeMATE Companion/` regardless. A test run
  against your real HOME leaves `wakemate.tls.json` behind there. Overriding
  `HOME` relocates those too.

- **Pairing cannot be completed on macOS, by design.** `main.rs` builds a
  `PairingCoordinator::unavailable()` on non-Windows because there is no tray to
  show an approval dialog, so both `/v1/pairing/enroll` and
  `/v1/pairing/activate` return 403 *"the WakeMATE tray app is not running"*.
  This is correct behaviour, not a broken setup — do not chase it. Full
  enroll→approve→command flows can only be verified on Windows.

- **Media and transport keys 400 on macOS.** `parse_key` gates
  `MediaPlayPause`/`MediaNextTrack`/`MediaPrevTrack` behind
  `#[cfg(any(target_os = "windows", target_os = "linux"))]` because enigo only
  defines them there. `{"type":"media","action":"play_pause"}` returns
  `unsupported key: playpause`. `volumeup`/`volumedown`/`mute` **do** work.
  A media-command failure here is a platform limit, not a regression.

- **Never send `{"type":"system","action":"sleep"}` with power commands
  enabled.** On macOS `perform_system_action` shells out to `pmset sleepnow` —
  it will suspend the machine you are working on. `restart`/`shutdown` map to
  `shutdown -r|-h now`. The driver only ever sends these with
  `allow_power_commands: false`, where they are a safe 403.

- **`--input` drives the real desktop.** `mouse_move` worked without any extra
  Accessibility grant, but it moves the actual cursor, and `mouse_button`/
  `key_press` deliver real clicks and keystrokes wherever focus happens to be.
  Keep those out of unattended runs.

- **The rate limiter will lock you out.** 8 failed auth attempts from one IP
  inside 60s ⇒ a 60s lockout for that IP (`src/security.rs`). A loop probing
  bad tokens locks the smoke test out of its own server; `smoke.sh` deliberately
  sends only 3.

- **`cargo check --target x86_64-pc-windows-msvc` still fails** — `ring` needs a
  C toolchain for that target. To typecheck Windows-only code, copy the module
  into a throwaway crate depending only on `windows-sys` (same version/features)
  and check that against the Windows target.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Keychain password dialog appears | Something is reading the token via the `security` CLI. Use the fake-`HOME` launch instead. |
| Auth returns 401 with the token you just set | The login keychain holds an older token and wins. Relaunch with `HOME` pointed at a scratch dir. |
| `unauthorized` on `/v1/pairing/enroll` with a valid token | Token was sent in the JSON body. It goes in the `x-wakemate-token` header. |
| `missing field 'delta_x'` (422) | Sent `dx`/`dy`. The wire names are `delta_x`/`delta_y`. |
| `unknown variant` (422) | Bad `type` tag on `/v1/command`; the tag is snake_case. |
| Every request 401s for ~a minute | Tripped the 8-failures/60s per-IP lockout. Wait it out. |
| `Address already in use` | A previous run is still up: `pkill -f "wakemate-companion --config-path"`. |
| `server died` from the driver | Read the printed `server.log`; the launch banner names the bind address and ports. |
