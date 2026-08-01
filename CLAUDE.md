# WakeMATE Companion

## Product

- This Rust companion and `B14ckfoot/WakeMate-Mobile` form one local-network protocol ecosystem.
- The phone discovers, pairs with, wakes, monitors, and remotely controls computers that the desktop user approved. Wake-on-LAN and offline waking are core capabilities.
- Favor a professional, simple, dependable flow for nontechnical users. Normal pairing must not require config editing or token copying, and desktop approval remains explicit.
- Prefer local operation. Do not add cloud accounts, analytics, telemetry, ads, or third-party tracking without explicit user approval.
- Security and privacy are product requirements. Never weaken authentication, TLS identity, token storage, approval, authorization, or platform security to make a feature appear functional.

## Architecture

- Rust desktop app with Axum HTTP/HTTPS APIs and UDP discovery. `src/main.rs` assembles runtime modes; `src/app.rs` routes/authenticates; `src/config.rs` migrates persistent config; `src/pairing.rs` coordinates approval; `src/devices.rs` stores approved-device token hashes; `src/tls.rs` owns persistent certificate identity.
- Windows 10/11 x64 tray app and Inno Setup installer are production targets.
- macOS is developer preview: the headless service builds/packages, but menu-bar UI, pairing approval, login startup, permissions, signing, notarization, and release parity are incomplete. Linux is not a primary end-user target.
- The API covers health/status, discovery, enrollment/pairing, Wake-on-LAN relay, input/media commands, and authorized system actions. The wire contract is versioned in `src/types.rs`.
- The pre-login mode has a narrower boundary than the interactive tray: it refuses pairing and input; any retained power ability must still derive from prior approved authorization.

## Commands

- Local development: `cargo run`
- Format: `cargo fmt --all`; check only: `cargo fmt --all -- --check`
- Lint: `cargo clippy --release --all-targets --locked -- -D warnings`
- Tests: `cargo test --release --locked`
- Debug build: `cargo build`; release build: `cargo build --release --locked`
- Full Windows quality gate: `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1`
- Windows release binary: `powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1`
- Windows installer: `powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1` (add `-BuildRelease` to rebuild first).
- macOS preview: `cargo build --release --locked`, then `./installer/macos/package-macos.sh`; output is unsigned and non-notarized.

## Security invariants

- Preserve local-first/no-telemetry behavior, explicit desktop approval, OS credential storage with honest fallback reporting, per-device authorization/revocation, constant-time secret comparison, and authentication lockout behavior.
- Preserve the persistent self-signed certificate identity and exact fingerprint consistency. Corrupt identity must fail clearly, not regenerate silently and break pins.
- Never silently downgrade a trusted HTTPS client to HTTP or broaden the transitional compatibility listener. Do not expose WakeMATE to the public Internet.
- Keep public, authenticated, pairing, input, power, and pre-login capabilities explicitly separated. A request reaching the server never implies permission to execute it.
- Remote input/power requires the appropriate approved capability. Pairing credentials and per-device credentials must retain their distinct scopes.
- Never synthesize Ctrl+Alt+Delete as keystrokes or bypass UAC, Secure Attention Sequence policy, macOS privacy controls, firewall protections, or code-signing requirements. Return structured, truthful unsupported/permission-required results.
- Preserve safe atomic config migrations and existing installations. Never log or commit secrets, credentials, certificates, private keys, tokens, or machine-specific paths.

## Platform and release rules

- Keep Windows-only code behind compile-time guards. Runtime operation should use normal-user privileges; admin/SYSTEM work must be narrowly limited to installation or the justified pre-login task.
- Do not claim macOS parity until menu-bar UI, desktop approval, permissions, startup, packaging, signing/notarization, and real-machine QA work.
- Installer changes must preserve upgrades and user settings, minimize firewall/admin scope, and clean up tasks/startup/firewall registrations on uninstall without unexpectedly erasing retained user data.
- Keep versions consistent across Cargo, installer, artifact names, and release metadata. Quality gates precede packaging; releases retain checksums and draft review. Never enable signing with placeholder secrets.

## Engineering workflow

- Inspect the relevant implementation first, state the probable root cause before a broad rewrite, and prefer the smallest complete fix.
- For QR payloads, discovery, pairing, authentication, ports, commands, status responses, or protocol versions, find all producers and consumers and review the mobile repository too. Preserve compatibility deliberately and update tests/docs.
- Add or update focused tests for authentication, parsing, pairing/approval, revocation, status, command authorization, config migration, and TLS behavior as applicable.
- Run the narrowest checks first, then the full quality gate when practical. Never suppress warnings, lint rules, errors, or failing tests just to get green output.
- Never discard unrelated work or use destructive Git commands. Do not commit, push, tag, publish a release, or alter signing configuration unless the user explicitly asks.

## Session handoff

- Report root cause, files changed, commands/tests and results, remaining limitations, and unavoidable manual steps.
- Use Claude auto memory only for durable, non-obvious recurring discoveries (verified fixes, build constraints, platform environment details, or stable architecture not suited here). Never store secrets, speculation, logs, temporary status, manifest-obvious facts, duplicated instructions, or completed-task narratives. Keep the index concise and move detailed recurring notes into topic files.
