# Troubleshooting WakeMATE 0.3

## The installer finished, but no tray icon appeared

This is expected: Setup never launches the companion or initializes an administrator's user profile. Close Setup and open **WakeMATE Companion** from Start as your normal user. Do not run the app as administrator. Complete the native settings/consent flow before pairing a phone.

An existing older configuration also needs the protocol 5 consent step. Until that completes, listeners are local-only and discovery is off.

## The phone cannot find this computer

1. Open **Settings → Open Settings...** from the tray and select the intended live private adapter.
2. Enable private network access. Ensure both devices are on a reachable home LAN; guest Wi-Fi/client isolation can prevent direct communication.
3. Read the connection status in Settings. HTTPS and discovery report their actual readiness separately. Use **Restart Connection** after fixing an adapter or port conflict.
4. Check Windows Firewall's Private-profile rules for this installed executable. The installer creates TCP `7778` and UDP `41234` rules limited to LocalSubnet. Public-profile networks do not receive those rules.

Do not enable public access, router forwarding, or an unrestricted firewall exception to fix discovery. Custom ports require corresponding narrowly scoped firewall rules; the installer rules cover the defaults. HTTPS can be ready while UDP discovery is unavailable.

If a chosen adapter's IP changed, choose its current address in Settings and restart the connection. A selected stale address is not silently replaced by all interfaces. The phone's saved certificate pin remains the computer identity when its IP changes.

## Secure pairing is missing or invalid

Update the companion and phone app, display a fresh QR from the companion tray, and scan it again from **Add device → Scan QR**. HTTP-only pairings and partial/malformed TLS metadata cannot authorize commands. Discovery cannot repair or replace a missing certificate pin.

If the message says secure storage is unavailable, unlock the phone and reopen WakeMATE. A locked keychain is not an expired pairing and does not erase the saved pin. If the error persists, resolve secure storage/native-build availability before scanning again.

Expo Go cannot provide WakeMATE's native certificate-pinning module. Development requires a current native build; restarting Metro alone does not install native code.

A certificate mismatch is a real identity failure. Verify that the QR is displayed on the intended computer. Do not disable certificate checks. Re-scan deliberately if that computer's TLS identity was intentionally replaced.

## Pairing waits, is denied, or times out

Keep the desktop prompt visible and approve only the phone you just scanned. Select the permissions you intend to grant. The prompt expires after two minutes; a QR invitation expires after ten minutes and can enroll only once. If denied, expired, or busy, close the old request and generate a fresh QR.

The mobile app stages the new credential before waiting, so suspension/resume can recover an approval. A pending request is not success. Protocol 5 never falls back to shared-token activation to obtain missing permissions.

## Pairing succeeded, but a control is denied

Open **Paired Devices → phone → Permissions...** on the computer. A status/wake-only grant is a successful pairing. Media, desktop input, and power require the corresponding desktop-selected preset. The phone cannot enlarge its own grant.

Desktop control is broad access to the signed-in account. Grant it only when you trust the phone to operate that account; withholding a power button does not constrain what keyboard/mouse access can do.

If **Pause Remote Access** is checked, resume deliberately to restore the existing grants. Repeated rejected credentials can trigger a temporary lockout; wait, then fix the pairing rather than retrying rapidly.

## Input stopped, a drag ended, or Windows rejected a command

Held mouse buttons expire after a two-second lease. Pause, revoke, connection restart, and shutdown cancel queued input and release owned synthetic state. Text requests have a 1024-character limit and execute in short chunks. A full queue rejects more work instead of growing indefinitely.

Windows may refuse input into elevated applications, security prompts, another session, or the secure desktop. WakeMATE reports insertion failures; it does not bypass those restrictions. Ctrl+Alt+Delete cannot be sent as a keyboard shortcut. The separate security-screen action reports its actual supported result or the explicitly chosen lock fallback.

Use **Pause Remote Access** to stop remote work, then verify the local desktop state. Already emitted input cannot be undone automatically. A physical key held by the local user must not be treated as synthetic state to release.

## The registry or settings could not be saved

Stop relying on the failed permission/revocation/pause change until storage is repaired. The current process denies authorization after a registry persistence failure, but a failed write cannot prove the change survives restart.

Open **Settings → Open Data Folder** and preserve the files and error details. Check that the normal Windows user can write their own app-data folder. Corrupt configuration is backed up and recovered local-only; unreadable settings or invalid preferred-adapter values fail without being overwritten. Corrupt device storage pauses access.

**Reset Companion...** is the explicit recovery path after storage is repaired. It clears approvals and network consent and returns to guided setup. Every phone must pair again. It retains the persistent TLS identity. Do not delete or regenerate `wakemate.tls.json` as a routine connectivity fix.

## Wake works only after sign-in, or never wakes the PC

The companion has no pre-login Windows server. The phone sends Wake-on-LAN directly while the computer is asleep; status/control start only after Windows sign-in and normal-user startup. Check firmware/adapter wake support and the saved MAC/broadcast address. Sending a packet does not prove the computer woke.

A relay through another running companion needs its wake grant. Public health or an unauthenticated discovery response does not authorize a relay.

## Another copy/session is already running

Use the tray in the owning Windows session to quit or restart the installed copy. Do not start development and installed copies against the same ports or force-kill every process with the same filename. The app must not control a different user's desktop. Installer/session-switch behavior still needs clean-VM and multi-user QA.

## Uninstall left data or an old credential entry

Retaining user data is intentional. Revoke/reset from the tray before uninstalling when you want to clear phone approvals. Then uninstall through Windows Settings. Verify startup entries in each Windows account that used the companion, especially after over-the-shoulder administrator installation of older versions.

The app-data folder retains the TLS identity/config/device registry. An older Windows Credential Manager `WakeMATE Companion` / `pairing-token` entry is unused by protocol 5 and may be removed manually there. Never send its value or a pairing QR/token in a support request.

## Build or release problems

Use the MSVC-aware [Windows build workflow](WINDOWS_BUILD.md), not a bare shell with missing compiler headers. Run the full quality gate before packaging. The installer helper rejects changed source, changed executables, mismatched versions, or an invalid runtime redistributable signature; rebuild rather than bypassing provenance checks.

Use the newly reported packaging output directory and its checksum/provenance files. Old `dist` installers can share a version number with different source generations. Automated tests and successful packaging do not replace [clean-VM and physical-phone release QA](RELEASE_CHECKLIST.md). macOS is still a [developer preview](MACOS_BUILD.md).
