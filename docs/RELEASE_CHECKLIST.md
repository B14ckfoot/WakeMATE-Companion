# WakeMATE Release Checklist

This checklist is for shipping a public Windows download. It is not legal advice.

## Product Identity

- Choose the final publisher name shown in Windows installer and code-signing metadata.
- Choose the final product version.
- Choose a support email and website URL.
- Choose a software license for this repo.
- Replace all placeholder text in the privacy policy, EULA, and installer files.

## Security Gates

- Confirm `allow_remote_connections` defaults to `false`.
- Confirm `allow_discovery` defaults to `false`.
- Confirm the Windows installer prepares a clean-install config that enables remote access and discovery for first-run pairing.
- Confirm `allow_input_commands` and `allow_power_commands` default to `false`.
- Confirm `GET /v1/info` requires `x-wakemate-token` in production builds.
- Confirm the config file is created under the user app-data folder, not next to the `.exe`.
- Confirm no tokens, passwords, or personal data are written to logs.
- Confirm the pairing token is stored via the OS credential store (`token_storage: "keyring"` after first run), with the file-based fallback only engaging (and logging a warning) when the credential store is genuinely unavailable.
- Confirm the tray `Rotate Pairing Token` action works and persists the new token.
- Confirm `/v1/pairing/activate` does **not** grant input/power capability without an explicit "Yes" on the native desktop confirmation dialog, and that it is refused outright from the pre-logon headless service.
- Confirm repeated bad tokens against an authenticated endpoint trigger the per-IP lockout (429) within a few dozen requests.
- Confirm "Reset Companion..." clears the stored token and local settings after its own confirmation dialog, and does not claim to affect any cloud account (it can't -- there isn't one).
- Test on a trusted LAN and on an untrusted network profile.
- If remote access is enabled, make sure Windows Firewall prompts are understood and documented.
- Re-read `docs/SECURITY_MODEL.md`'s "Known limitation: no transport encryption" section -- this has **not** been fixed and is the top item to resolve before any wider-than-home-LAN release.

## Packaging Gates

- Run `.\scripts\quality-check.ps1` (fmt, clippy `-D warnings`, tests, release build) and confirm it passes.
- Build `.\build-release.ps1` or `cargo build --release` from a working Visual Studio Developer shell.
- Verify the final binary path and SHA256 hash.
- Bundle `installer\redist\VC_redist.x64.exe` if you keep using the MSVC Windows target.
- Sign the `.exe` and installer with Authenticode.
- Verify SmartScreen reputation behavior on a clean Windows machine.
- Verify the installer creates Add/Remove Programs entries.
- Verify uninstall removes installed files, shortcuts, the pre-logon scheduled task, and the startup Run-key entry cleanly; verify it leaves `%APPDATA%\WakeMATE Companion\` and the Credential Manager entry alone (see `docs/TROUBLESHOOTING.md`).
- Verify updates do not overwrite user config unexpectedly.
- macOS: see `docs/MACOS_BUILD.md` -- there is no signed/notarized artifact to gate on yet; do not ship an unsigned `.dmg` as a production release.

## QA Gates

- Test first run on a clean Windows VM.
- Test install under a standard user account.
- Test install under an administrator account.
- Test remote access disabled mode.
- Test remote access enabled mode.
- Test pairing QR flow.
- Test token rotation.
- Test Wake-on-LAN.
- Test input commands.
- Test power commands.
- Test uninstall and reinstall.

## Legal And Policy Gates

- Publish a privacy policy if you collect or transmit personal data, diagnostics, account details, or identifiers beyond local device control.
- Publish an EULA or Terms document if you want installer-visible usage terms or warranty/liability language.
- Clearly disclose that enabling remote access allows network control of mouse, keyboard, media, and power actions.
- Clearly disclose that the pairing token must be kept secret.
- Review third-party crate licenses and ship required notices.
- If you distribute in California or otherwise meet privacy-law thresholds, confirm whether notice-at-collection and consumer-rights disclosures apply.

## Recommended Files To Finalize

- `README.md`
- `docs/PRIVACY_TEMPLATE.md`
- `docs/EULA_TEMPLATE.txt`
- `docs/THIRD_PARTY_NOTICES_TEMPLATE.md`
- `installer/INSTALL_WARNING.txt`
- `installer/wakemate-companion.iss`

## Current Blockers In This Workspace

- No final software license has been chosen yet.
- No production signing certificate is configured yet.
- Privacy policy, EULA, publisher identity, and support contact details are still placeholders.
