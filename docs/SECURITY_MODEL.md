# WakeMATE Security Model

## Default State

- HTTPS binds to `127.0.0.1` unless `allow_remote_connections` is explicitly enabled.
- A plaintext HTTP compatibility listener follows the same bind rule while `allow_insecure_http` is `true`. This defaults to `true` for one migration release.
- UDP discovery is disabled unless both `allow_remote_connections` and `allow_discovery` are enabled.
- Input and power commands are disabled by default, and even when enabled in config, still require the pairing approval described below before either flag can flip to `true` in the first place.
- Detailed device info requires the pairing token by default.
- The pairing token lives in the OS credential store (Windows Credential Manager; `keyring` crate) rather than a plaintext file. If the credential store is unavailable on a given machine, WakeMATE falls back to the config file and records that fact in `token_storage` so this is never silent.

## Windows Installer Behavior

- On a clean Windows install, the installer prepares the first-run config with `allow_remote_connections` and `allow_discovery` enabled so the mobile app can auto-discover the PC immediately. This is a deliberate, disclosed product tradeoff for first-run UX, not an oversight -- it widens the network attack surface (an unauthenticated UDP probe gets a device-name/IP/MAC reply) on a clean install before the user has done anything, in exchange for the mobile app being able to find the PC without manual IP entry.
- Existing configs are preserved on reinstall or upgrade.
- Input and power commands still stay disabled, and pairing still requires desktop approval, regardless of this installer default.
- Per-user config, Credential Manager, and HKCU startup preparation run as the original interactive user after a normal UAC elevation. Elevated installer work is limited to machine-scoped files, prerequisites, firewall changes, and removal of the retired SYSTEM boot task.

## Threat Model

Considered explicitly:

| Threat | Mitigation | Residual risk |
| --- | --- | --- |
| Attacker on the same LAN sniffing traffic | Current mobile builds use HTTPS and pin the self-signed leaf certificate's SHA-256 fingerprint from the visual QR channel | Traffic from a legacy client remains observable while the transitional HTTP listener is enabled. Disable `allow_insecure_http` after all phones have upgraded and re-scanned. |
| Attacker who has obtained a valid token (leak, screenshot, shoulder-surf of the QR code) | The QR normally carries a fresh single-use pairing-session token with a 10-minute lifetime, enrollment still requires explicit desktop approval, and each approved phone receives its own revocable device token | A stolen approved-device token retains that device's capabilities until it is revoked from the tray. Rotating the shared pairing credential revokes every device at once. |
| Blind token brute force | Constant-time comparison + per-IP lockout (8 failures / 60s window -> 60s lockout, shared across all authenticated endpoints) | A distributed brute force across many source IPs is not rate-limited; token entropy (a v4 UUID, ~122 bits) is the remaining defense there. |
| A malicious website in the user's browser trying to drive the API | Auth requires a custom header (`x-wakemate-token`), which a simple cross-origin form post cannot set, and which triggers a CORS preflight for `fetch`/`XHR` that the app doesn't need to explicitly allow (no `Access-Control-Allow-Origin` is returned, so the browser blocks reading the response and, for state-changing requests, blocks the request from completing for a script that doesn't already know the token) | No Origin/Host allowlist is enforced server-side as defense in depth; recommended future work. |
| Malicious/compromised paired phone silently enabling input/power control | `/v1/pairing/activate` now shows a native, always-on-top desktop dialog naming the requesting IP and requires an explicit "Yes" before flipping `allow_input_commands`/`allow_power_commands` | The dialog shows an IP, not a verified device identity; there is no cryptographic device-identity check. |
| A privileged server resolving different secrets and TLS state than the signed-in user's tray | Current installers remove the retired SYSTEM/ONSTART task, and stale `--headless-server` invocations exit before reading user state or binding a listener | Companion API status/control is unavailable until the user signs in and the normal-user tray starts. Direct Wake-on-LAN from the phone remains available before sign-in. |
| Excessive OS privilege | The running companion is the normal-user tray process; no Companion API runs as SYSTEM | The installer still needs administrator approval for Program Files, prerequisites, firewall configuration, and legacy task cleanup. |
| Tampered/malicious update package | No auto-update mechanism exists yet | N/A today; if auto-update is added, it must verify a signature before replacing the running binary. |
| Local file tampering / insecure local files | Config file no longer holds the token once migrated to the credential store. The persistent TLS certificate/private key identity is stored in the user's app-data folder and is created mode `0600` on Unix. A corrupt identity is rejected rather than silently replaced. | A local attacker with the same OS user account can still read/write the config, TLS identity, and any fallback plaintext token. On Windows the app-data file relies on the user's directory ACLs. |
| Debug functionality left on in production | No `RUST_LOG=debug`/devtools-equivalent shipped enabled; tracing defaults to `info` and never logs the token itself (only its length, e.g. on rotation) | -- |

## Transitional HTTP compatibility

The companion now generates a persistent self-signed certificate, serves HTTPS on `tls_port` (default `7778`), and places the leaf certificate's SHA-256 fingerprint in the pairing QR. The current mobile app saves the pin in secure storage and checks the exact leaf-certificate DER digest in a native Android/iOS TLS challenge handler before sending the bearer token. QR payloads that partially advertise TLS are rejected instead of downgraded.

For upgrade compatibility, `allow_insecure_http` defaults to `true` and keeps the original HTTP port available to pre-Stage-2 phones. The current mobile app never sends a QR-paired token over that listener. Operators should disable it after all paired phones have upgraded and re-scanned. This transition is the remaining transport risk, not a substitute for public-Internet exposure.

## Current Authentication

- Pairing-session and per-device bearer tokens via `x-wakemate-token`, compared in constant time; only per-device token hashes are stored in the local registry
- Shared pairing credential stored in the OS credential store, used to mint short-lived single-use QR sessions; rotation revokes all enrolled devices
- Per-device revocation from the tray without disrupting other approved phones
- Per-IP rate limiting/lockout on repeated authentication failures across all authenticated endpoints
- Explicit desktop confirmation gate before pairing can grant input/power capabilities
- No built-in account system
- Built-in self-signed TLS with mobile-side leaf-certificate fingerprint pinning

## Operational Guidance

- Keep remote access disabled unless you actively need it.
- Enable discovery only when your mobile app needs auto-discovery.
- Upgrade and re-scan every paired phone, then set `allow_insecure_http` to `false`.
- Rotate the pairing token after testing, screenshots, screen sharing, or suspected exposure -- this revokes every currently paired phone.
- Use "Reset Companion..." from the tray if you want to wipe all local pairing state and start clean; it does not touch any cloud account.
- Do not enable remote input or power actions on shared or public machines unless that risk is acceptable, and always click "Deny" on a pairing prompt you did not expect.

## Release Recommendation

Before exposing WakeMATE beyond a trusted home LAN in a production release, prioritize (in order): (1) end the HTTP migration window and default `allow_insecure_http` to `false`, and (2) add an Origin/Host allowlist as defense in depth against browser-based abuse.
