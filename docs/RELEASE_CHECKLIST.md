# WakeMATE 0.3 release checklist

A green source/build gate is necessary but does not validate installation or physical devices. Record the source revision, build/provenance files, test results, and remaining boundaries for the exact artifact being reviewed. Historical audits are not current release evidence.

## Source and protocol gates

- [ ] Cargo, installer, release metadata, artifact names, and tag agree on the version; wire protocol is 5 and pairing QR contract remains 3.
- [ ] Windows format, Clippy, tests, release build, and dependency audit pass with the locked dependency set and validated MSVC environment.
- [ ] Mobile lint, TypeScript, full Jest, and generated iOS invariant checks pass against this companion contract.
- [ ] No permanent shared token/keyring runtime or global input/power authorization remains. Retired JSON fields are omitted on save.
- [ ] QR-session credentials cannot command the desktop, and approved-device credentials cannot enroll another phone. Legacy activation is rejected.
- [ ] Missing/corrupt/unavailable mobile pins and certificate mismatch fail before credentials are sent. IP overrides retain the saved identity. HTTP supports only unauthenticated public health.
- [ ] Each capability is checked with the phone's current durable grants. Approval with status/wake only succeeds without enabling input/power. Legacy records receive only least-privilege grants.
- [ ] Pause, changed grants, one/all-phone revoke, reset, restart, and shutdown cancel stale queued/held input. Storage failure is surfaced and cannot report durable success.
- [ ] Input tests cover a full bounded queue, authorization changes, held-button expiry, text limits/chunking, physical modifiers, and partial/failed `SendInput`.
- [ ] Logs, fixtures, and artifacts contain no real tokens, QR credentials, private keys, signing material, or user-specific configuration.

## Exact artifact gates

- [ ] Run the quality gate before packaging. Build fresh or use an existing build only with matching source/executable provenance.
- [ ] Package into the newly generated versioned run directory; retain the installer SHA-256 and provenance JSON. Do not select an older same-version file from another `dist` directory.
- [ ] Verify the bundled VC++ redistributable has a valid Microsoft signature. Verify prerequisite installation failures and restart requirements are reported accurately.
- [ ] Verify a source or executable change after building makes packaging fail. Confirm new outputs preserve earlier artifacts.
- [ ] Record whether both executable and installer are signed. Until real signing is configured and verified, label them unsigned development/review artifacts; do not bypass SmartScreen or antivirus.
- [ ] Keep release automation in draft mode. Review the exact uploaded files before publication; do not tag/publish as part of an unrelated code change.

## Clean Windows VM and account gates

- [ ] Fresh standard-user installation with administrator approval: Setup does not launch the tray or initialize the administrator's user state.
- [ ] Normal Start-menu launch opens guided native setup. Before consent, HTTPS is loopback-only and discovery is disabled. Elevated interactive runtime is refused.
- [ ] Select a private adapter, approve LAN access, pair by one QR scan, and exercise all four permission presets and denied actions.
- [ ] Verify Private/LocalSubnet TCP 7778 and UDP 41234 rules for the installed program; no Public-profile or broad legacy allow rule remains. Check custom-port behavior explicitly.
- [ ] Test pause/resume, change permissions, one/all-phone revoke, reset-to-setup, fresh invitation expiry, late prompt rejection, and restart persistence.
- [ ] Test read-only/unwritable storage and corrupt config/registry/TLS identity. Verify fail-closed recovery and preservation of evidence.
- [ ] Upgrade an existing installation without losing supported preferences or silently restoring broad grants. Omitted HTTP compatibility defaults false.
- [ ] Test changed/removed adapters, VPNs/virtual adapters, multiple NICs, DHCP change, occupied HTTPS/discovery ports, and partial readiness.
- [ ] Test Windows sign-in/sign-out, locking/unlocking, fast user switching, and two-account startup. The wrong session must never receive remote input.
- [ ] Test graceful upgrade while running, cancellation, uninstall/reinstall, legacy scheduled-task cleanup, firewall cleanup, and startup entries for each affected Windows account. Document retained user data and inert older keyring entries.

## Physical Android and iOS gates

- [ ] Use current native builds; Expo Go and a Metro restart cannot validate certificate pinning.
- [ ] Pair, deny, expire, re-scan, suspend/terminate/resume during approval, and recover a durably approved credential.
- [ ] Change the certificate and confirm failure before sending a token. Change only the IP and confirm the original pin is retained.
- [ ] Test missing/corrupt pin and locked/unavailable secure storage without HTTP fallback. Confirm public health discovery has no credential.
- [ ] Test actual mouse/drag, keyboard/modifiers, media, allowed/denied power, queue pressure, text chunking, pause/revoke during execution, and truthful unsupported security-screen results.
- [ ] Test direct Wake-on-LAN while the companion is offline and verify the PC actually wakes; packet dispatch alone is not proof.

## Platform and publication review

- [ ] Keep macOS labeled unsigned/non-notarized headless developer preview. Build/package success is not menu-bar, approval, permissions, startup, or real-machine parity. See [macOS notes](MACOS_BUILD.md).
- [ ] Finalize the software license, publisher/support details, privacy policy, third-party notices, installer warning, and EULA review before a public release.
- [ ] Explain local operation, broad signed-in-account trust for keyboard/mouse grants, retained data, and the absence of an automatic updater.
- [ ] Publish only after the required artifact/device/account checks above have evidence. Leave unperformed checks and limitations explicit in the release notes.
