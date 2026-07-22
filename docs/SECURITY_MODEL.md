# WakeMATE Security Model

## Default State

- HTTP binds to `127.0.0.1` unless `allow_remote_connections` is explicitly enabled.
- UDP discovery is disabled unless both `allow_remote_connections` and `allow_discovery` are enabled.
- Input and power commands are disabled by default, and even when enabled in config, still require the pairing approval described below before either flag can flip to `true` in the first place.
- Detailed device info requires the pairing token by default.
- The pairing token lives in the OS credential store (Windows Credential Manager; `keyring` crate) rather than a plaintext file. If the credential store is unavailable on a given machine, WakeMATE falls back to the config file and records that fact in `token_storage` so this is never silent.

## Windows Installer Behavior

- On a clean Windows install, the installer prepares the first-run config with `allow_remote_connections` and `allow_discovery` enabled so the mobile app can auto-discover the PC immediately. This is a deliberate, disclosed product tradeoff for first-run UX, not an oversight -- it widens the network attack surface (an unauthenticated UDP probe gets a device-name/IP/MAC reply) on a clean install before the user has done anything, in exchange for the mobile app being able to find the PC without manual IP entry.
- Existing configs are preserved on reinstall or upgrade.
- Input and power commands still stay disabled, and pairing still requires desktop approval, regardless of this installer default.

## Threat Model

Considered explicitly:

| Threat | Mitigation | Residual risk |
| --- | --- | --- |
| Attacker on the same LAN sniffing traffic | None -- HTTP is plaintext | **Not mitigated.** See "Known limitation: no transport encryption" below. |
| Attacker who has obtained a valid token (leak, screenshot, shoulder-surf of the QR code) | Rate limiting slows brute force; pairing activation now requires an explicit desktop click, not just a valid token | Once the token is known, `/v1/wake`, `/v1/info`, and (if already paired) `/v1/command` are usable until the token is rotated. Rotation invalidates every paired phone at once (no per-device revocation yet). |
| Blind token brute force | Constant-time comparison + per-IP lockout (8 failures / 60s window -> 60s lockout, shared across all authenticated endpoints) | A distributed brute force across many source IPs is not rate-limited; token entropy (a v4 UUID, ~122 bits) is the remaining defense there. |
| A malicious website in the user's browser trying to drive the API | Auth requires a custom header (`x-wakemate-token`), which a simple cross-origin form post cannot set, and which triggers a CORS preflight for `fetch`/`XHR` that the app doesn't need to explicitly allow (no `Access-Control-Allow-Origin` is returned, so the browser blocks reading the response and, for state-changing requests, blocks the request from completing for a script that doesn't already know the token) | No Origin/Host allowlist is enforced server-side as defense in depth; recommended future work. |
| Malicious/compromised paired phone silently enabling input/power control | `/v1/pairing/activate` now shows a native, always-on-top desktop dialog naming the requesting IP and requires an explicit "Yes" before flipping `allow_input_commands`/`allow_power_commands` | The dialog shows an IP, not a verified device identity; there is no cryptographic device-identity check. |
| The pre-logon boot service (`schtasks /RU SYSTEM`) being reachable before anyone signs in | The headless server refuses pairing activation and every input command unconditionally, regardless of config | It still executes power actions (sleep/restart/shutdown) if `allow_power_commands` was already enabled through a prior, approved pairing -- this is intentional (remote-restarting a hung headless machine is a legitimate use case) but is a real capability exposed at elevated privilege before login. |
| Excessive OS privilege | Everything except the pre-logon boot task runs as the interactive user, not SYSTEM/admin | The boot task itself must run as SYSTEM because Windows Task Scheduler requires that account (or stored credentials) for an `ONSTART` trigger with "run whether logged in or not"; see the table row above for the compensating control. |
| Tampered/malicious update package | No auto-update mechanism exists yet | N/A today; if auto-update is added, it must verify a signature before replacing the running binary. |
| Local file tampering / insecure local files | Config file no longer holds the secret once migrated to the credential store; file permissions are whatever the OS default is for the user's `%APPDATA%` (not further hardened) | A local attacker with the same OS user account can still read/write the config file and any fallback plaintext token. |
| Debug functionality left on in production | No `RUST_LOG=debug`/devtools-equivalent shipped enabled; tracing defaults to `info` and never logs the token itself (only its length, e.g. on rotation) | -- |

## Known limitation: no transport encryption

HTTP, not HTTPS, is used end-to-end. This was evaluated and deliberately **not** changed in this pass: the paired mobile app (`Wakemate-Mobile/src/services/deviceService.ts`) is hardcoded to `http://` and uses `axios` with default certificate validation, so switching the Companion to a self-signed HTTPS certificate unilaterally would break every existing paired phone outright (the client would reject the untrusted certificate) rather than improve security in practice.

The correct fix is a coordinated change: the Companion generates a self-signed certificate on first run (e.g. via `rcgen`) and includes its fingerprint in the pairing QR payload; the mobile app pins that fingerprint on first pair (trust-on-first-use) instead of relying on public CA validation. That is mobile-app work outside this pass's scope and is the single most important piece of remaining work -- see the final report's "Remaining risks" section.

## Current Authentication

- Single shared bearer token via `x-wakemate-token`, compared in constant time, stored in the OS credential store
- Per-IP rate limiting/lockout on repeated authentication failures across all authenticated endpoints
- Explicit desktop confirmation gate before pairing can grant input/power capabilities
- No built-in account system
- No built-in TLS termination (see above)

## Operational Guidance

- Keep remote access disabled unless you actively need it.
- Enable discovery only when your mobile app needs auto-discovery.
- Rotate the pairing token after testing, screenshots, screen sharing, or suspected exposure -- this revokes every currently paired phone.
- Use "Reset Companion..." from the tray if you want to wipe all local pairing state and start clean; it does not touch any cloud account.
- Do not enable remote input or power actions on shared or public machines unless that risk is acceptable, and always click "Deny" on a pairing prompt you did not expect.

## Release Recommendation

Before exposing WakeMATE beyond a trusted home LAN in a production release, prioritize (in order): (1) transport encryption with mobile-side certificate pinning, (2) per-device pairing/revocation instead of a single shared token, (3) an Origin/Host allowlist as defense in depth against browser-based abuse.
