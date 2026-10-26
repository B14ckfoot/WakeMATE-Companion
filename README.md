# WakeMATE Companion 0.3

WakeMATE Companion connects the [WakeMATE mobile app](../../Wakemate-Mobile) to a computer on your private LAN. Your phone can wake a sleeping PC directly. Once Windows is signed in and the companion is running, each approved phone can use only the permissions selected on that desktop.

This version introduces **protocol 5**, native Windows setup and permission dialogs, and HTTPS-only authenticated access. See [migration notes](docs/PROTOCOL5_MIGRATION.md) before upgrading an existing installation.

## Platform status

Windows 10/11 x64 is the supported development target: tray, native settings, pairing approval, and installer. A clean Windows VM and physical phone still need release QA. macOS remains a headless developer preview without menu-bar setup or approval; Linux is not an end-user target. See [macOS notes](docs/MACOS_BUILD.md).

## Install and pair on Windows

1. Install the current versioned Windows package and approve its installation prompt. Administrator access is limited to installation, prerequisites, firewall rules, and legacy cleanup.
2. Close Setup, then open **WakeMATE Companion** from the Windows Start menu as your normal user. **Setup does not launch the app.** Signing is not configured; verify the source and published checksum, and do not disable Windows security protections to install it.
3. In the native setup/settings dialog, enable access on your private network and choose the intended adapter. The companion starts local-only until you consent. Your phone and computer must be on the same reachable LAN.
4. Choose **View Pairing QR Code** from the tray, then **Add device → Scan QR** on the phone. The visual QR establishes the computer's certificate identity.
5. Approve the recognized phone in the desktop permission dialog and select its access preset. Deny unexpected requests. The phone waits for actual approval before reporting success.

No config editing, copied tokens, cloud account, or public router exposure is required for normal pairing.

| Desktop permission preset | Phone access |
| --- | --- |
| Status and wake only | Computer details and Wake-on-LAN relay |
| Media remote | Status, wake, and media/volume keys |
| Desktop control | Status, wake, media, keyboard, and mouse |
| Desktop and power | Desktop control plus supported lock, sleep, restart, shutdown, and sign-out actions |

**Keyboard and mouse access grants broad control of your signed-in account.** A phone with keyboard access can perform actions beyond the app's power buttons. Only grant desktop control to a phone you trust with that account. Windows security prompts and the secure desktop remain protected.

An approved status/wake-only phone is successfully paired; disabled input or power access is an intentional permission decision.

## Everyday controls

- **Settings → Open Settings...** shows native connection settings and setup actions.
- **Pause Remote Access** stops approved phones from issuing commands and cancels queued input. Resume restores their saved grants.
- **Paired Devices → phone → Permissions...** changes that phone's preset; **Revoke this phone...** removes it.
- **Settings → Revoke All Phones...** removes all approved phones and invalidates outstanding pairing invitations.
- **Settings → Enable Private LAN Access** controls network exposure independently of per-phone grants.
- **Restart Connection** retries listeners after a network or adapter change, preserving the certificate identity and approved-device records.
- **Settings → Launch on Windows Startup** controls normal-user startup after sign-in.
- **Settings → Reset Companion...** clears approvals and network consent, then returns to guided setup. The TLS identity is retained.

Direct phone Wake-on-LAN works without a pre-login companion service. Pairing, status, and control require Windows sign-in and the running normal-user tray. Wake-on-LAN also depends on the PC firmware, adapter, and LAN configuration.

## Upgrades, uninstall, and retained data

There is no automatic updater. Run a newer installer over the existing installation, then launch normally from Start. Existing names, ports, and startup preferences are retained; older configurations must complete the protocol 5 consent flow. Existing phone records without permission grants receive status/wake access only. Select any additional grants on the desktop.

Use Windows **Settings → Apps → WakeMATE Companion → Uninstall** to remove the installed program and its machine registrations. User data is retained under `%APPDATA%\WakeMATE Companion`, including the TLS identity and device registry. Revoke phones or reset before uninstalling if you do not want retained approvals. Check startup entries in each Windows account that used the app; installer and multi-user cleanup need clean-VM verification.

Protocol 5 has no permanent shared credential and does not access Windows Credential Manager. An old `WakeMATE Companion` / `pairing-token` entry may remain from an earlier release; it is inert and may be removed manually through Windows Credential Manager. No password needs to be entered into WakeMATE or shared for cleanup.

## Security and privacy

- Local operation, with no cloud accounts, analytics, ads, or telemetry.
- A persistent self-signed certificate identifies this computer. The phone pins its exact SHA-256 fingerprint from the QR before sending a credential. Changing an IP does not erase that identity.
- Authenticated requests use HTTPS. Missing, corrupt, inaccessible, or mismatched pins fail closed. Optional legacy HTTP exposes only credential-free public health.
- QR invitations are temporary and single-use. Approved phones get separate bearer credentials; only their hashes and selected grants are stored on the companion.
- Permissions, pause, and revocation are persisted as one versioned device-registry snapshot. Storage failures are surfaced and deny access; do not assume a failed operation survived a restart.
- Remote input uses one bounded worker with authorization checks, cancellation, checked Windows `SendInput`, and held-button expiry. It does not bypass UAC or synthesize Ctrl+Alt+Delete.

Read the [security model](docs/SECURITY_MODEL.md) and [troubleshooting guide](docs/TROUBLESHOOTING.md) for the practical boundaries.

## Development

Rust/Axum serves the local protocol. `src/app.rs` owns endpoint authorization, `src/pairing.rs` owns temporary approval sessions, `src/devices.rs` owns durable per-phone grants, `src/config.rs` owns preferences and consent, and `src/tls.rs` owns certificate identity. `src/input_executor.rs` serializes input; `src/windows_input.rs` performs checked OS calls. Native desktop dialogs, adapter selection, and runtime readiness are in `src/desktop.rs`, `src/network.rs`, and `src/runtime_status.rs`.

Protocol 5 retains `/v1` route names. Public health is `GET /v1/health`. Enrollment/status use temporary QR-session credentials; approved-device credentials serve `/v1/info`, `/v1/pairing/check`, `/v1/wake`, and `/v1/command` according to their grants. `/v1/pairing/activate` no longer enables shared-token control. Discovery uses UDP `41234`; HTTPS defaults to TCP `7778`. Discovery metadata cannot establish trust. Pairing QR contract v3 carries `protocol_version: 5`, the TLS port, and certificate fingerprint.

Preferences live in `wakemate.config.json` in the user's app-data folder. See the [sample](wakemate.config.example.json). Normal runtime uses this user-scoped path; `--config-path` is limited to install-preparation tooling. Configuration contains no permanent shared credential. `wakemate.tls.json` contains the persistent certificate/private key; `wakemate.devices.json` contains approved-device hashes and grants. Preserve these files when diagnosing a problem.

Listener thread lifetime is isolated in `src/server_thread.rs`; the tray renders
service state and local actions. Successful commands have per-phone budgets,
HTTP work has bounded concurrency/deadlines, and system actions are serialized.

Run the Windows quality gate from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1
```

Build and package explicitly:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1
```

The installer helper verifies source/build provenance, matching versions, executable hash, and the Microsoft runtime signature. Each packaging run writes a fresh directory under `dist/installer/` with checksums and provenance. Old files elsewhere in `dist` are historical artifacts, not the current build. The quality gate must pass before packaging.

See [Windows build notes](docs/WINDOWS_BUILD.md), the [release checklist](docs/RELEASE_CHECKLIST.md), and [migration guide](docs/PROTOCOL5_MIGRATION.md). Signing requires real protected credentials; current artifacts remain unsigned until signing is configured and verified. Release automation stages drafts for maintainer review. Clean-VM installation, physical Android/iOS pairing, and macOS runtime/package validation remain separate release gates.
