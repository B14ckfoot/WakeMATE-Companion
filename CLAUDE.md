# WakeMATE Companion

## Product and scope

- WakeMATE Companion 0.3 and `B14ckfoot/WakeMate-Mobile` form one private-LAN protocol ecosystem. Keep offline direct phone Wake-on-LAN separate from online companion status/control.
- Windows 10/11 x64 is the primary target. The installer does not launch the app; the normal-user Start-menu launch opens native guided setup. LAN access requires interactive consent.
- Make ordinary setup usable without config editing, copied tokens, cloud accounts, or public router exposure. Do not add telemetry, analytics, advertising, or tracking.
- macOS is a headless developer preview without menu-bar setup/approval, login integration, signing/notarization, or end-user parity. Linux is not a primary product target.

## Architecture

- `src/main.rs` supervises actual listener readiness and shutdown; `src/tray.rs` owns Windows tray lifecycle.
- `src/desktop.rs` contains native settings/approval dialogs. `src/network.rs` selects cached private adapters; `src/runtime_status.rs` exposes HTTPS/discovery readiness separately.
- `src/app.rs` separates public health, QR enrollment/status, approved-device operations, and capability checks. `src/types.rs` owns protocol 5. Pairing QR contract remains v3.
- `src/pairing.rs` owns temporary single-use QR sessions and expiring desktop approvals. There is no permanent shared pairing credential or runtime keyring backend.
- `src/devices.rs` stores token hashes, selected per-phone grants, pause, and generation in one atomic versioned registry. Old records without grants default to status/wake only.
- `src/config.rs` stores non-secret preferences, `onboarding_completed`, and optional private `preferred_address`. LAN binds/discovery stay closed until consent.
- `src/input.rs` validates/represents commands; `src/input_executor.rs` owns the bounded cancellable worker; `src/windows_input.rs` implements checked `SendInput`. HTTP workers never emit OS input directly.
- `src/tls.rs` preserves the computer's persistent certificate identity.

## Non-negotiable boundaries

- Authenticated traffic uses pinned HTTPS. Optional HTTP is credential-free public health only; missing/corrupt pins or keychain errors never authorize downgrade. Discovery cannot establish/replace trust.
- QR-session credentials can enroll/poll only their session; device credentials cannot enroll another phone. Protocol 5 rejects permanent shared credentials and legacy activation.
- Require explicit desktop-selected per-device grants. Status/wake is the default; media, keyboard/mouse, and power are separate checks. Keyboard/mouse access is broad trust of the signed-in account, not a power-action sandbox.
- Preserve pause, one/all-phone revoke, permission changes, reset, and cancellation with truthful persistence errors. Never report an uncommitted grant or failed revoke as durable success.
- Keep normal-user execution, the single owning Windows session, UAC/secure-desktop protections, and honest unsupported results. Retired pre-login `--headless-server` mode exits before reading user state or binding listeners.
- Preserve TLS identity, atomic storage/recovery, constant-time comparisons, authentication lockout, input limits, and cancellation/held-state cleanup. Never log or commit credentials/private keys.

## Engineering workflow

- The staged redesign is authorized. Use clear module/actor boundaries and reviewable changes, preserving the original repository, unrelated work, existing supported preferences, and TLS identity. Do not rewrite merely for style or retain a legacy mechanism that defeats the new boundary.
- Before editing a protocol path, trace producers and consumers across QR → discovery → storage → enrollment → native desktop approval → polling → commands/status. Review the mobile repository too.
- Add meaningful tests for changed security/failure behavior. Inject input/storage/approval boundaries so tests never control the real desktop or alter installed credentials/config.
- Read applicable `.claude/rules/` files. These capture security/platform/release invariants; they do not impose a blanket smallest-change policy or another approval flow within the user's authorized work.
- Never discard unrelated work or use destructive Git commands. Do not commit, push, tag, publish, alter signing configuration, or edit memory without the user's explicit request.

## Checks and packaging

- Windows full source gate: `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1` (loads validated MSVC, then format/lint/test/release build).
- Narrow Rust checks: `cargo fmt --all -- --check`; `cargo clippy --release --all-targets --locked -- -D warnings`; `cargo test --release --locked`; `cargo build --release --locked`.
- Run dependency audit for release review with `cargo audit`. Do not ignore findings or warnings just to make checks green.
- Build provenance: `powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1`.
- Package: `powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1`. Existing builds require matching source/executable provenance; compatibility switches never bypass it. Use the new run-specific output directory, checksums, and provenance.
- Mobile gate: lint, TypeScript, Jest, and `verify:ios` separately. Native builds and physical Android/iOS pairing remain required device validation.
- No source/build result substitutes for clean-VM install/upgrade/uninstall, account/session, firewall, real OS input, wake, or macOS runtime QA. Label unsigned artifacts honestly and keep release drafts reviewable.

## References and handoff

Use [README](README.md), [security model](docs/SECURITY_MODEL.md), [migration](docs/PROTOCOL5_MIGRATION.md), and [release checklist](docs/RELEASE_CHECKLIST.md) as current product documentation. Historical audits are evidence of earlier work, not a current release gate.

Report the resulting behavior, changed files, test evidence, remaining runtime/device boundaries, and any required manual steps. Keep permissions and unsupported/error states understandable to a nontechnical user.
