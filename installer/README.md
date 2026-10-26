# Installer Notes

This folder contains an Inno Setup template for packaging WakeMATE Companion as a Windows installer.

## Before Building The Installer

- Run the Windows quality gate; production distribution additionally requires real signing credentials.
- Place the official Microsoft Visual C++ x64 redistributable at `installer\redist\VC_redist.x64.exe`
- Replace placeholders in `wakemate-companion.iss`
- Review `..\docs\EULA.txt` with qualified counsel for the intended release jurisdictions
- Review `INSTALL_WARNING.txt`
- Finalize `..\docs\THIRD_PARTY_NOTICES_TEMPLATE.md`

## Build

Compile the installer with:

`powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1`

The installer script runs `build-release.ps1` first, validates its source and
executable SHA-256 provenance and matching versions, and checks the Microsoft
signature on the bundled redistributable. CI can use `-UseVerifiedBuild` after
building in the same job; source or binary changes still refuse packaging.
The deprecated `-SkipPrereqCheck` switch cannot bypass these checks.

Each invocation writes a fresh directory under `dist/installer/` containing
the versioned installer, its SHA-256 checksum, and source/build provenance.
Previous outputs remain intact. `target/installer-latest.json` identifies the
exact current output. Use the script rather than packaging a stale binary in
the Inno Setup GUI.

The installer will bundle `installer\redist\VC_redist.x64.exe` and silently install it only when the target machine does not already have the Visual C++ runtime.

Setup handles machine files, prerequisites, narrowly scoped firewall rules,
and cleanup of the retired SYSTEM boot task. It never launches the tray or
prepares a Windows user's private configuration. Close Setup and launch
WakeMATE normally from the Start menu; this initializes the signed-in user's
state. Elevated and service runtime contexts are refused.

Firewall rules permit this installed program on Private networks from
LocalSubnet only: TCP 7778 for HTTPS and UDP 41234 for discovery. Existing
broad legacy rules are removed. Private-network operation still requires
explicit approval in the companion. Custom ports require separately reviewed
firewall changes.

Upgrades request a graceful exit only from the exact installed executable
path. A running older release requires manual tray exit; Setup never invokes
an unknown old command or force-kills processes by filename. One Windows
session owns the companion machine-wide until that instance exits.

Uninstall removes matching startup entries from loaded user registry hives,
regardless of the administrator account running uninstall. It checks the
exact installed executable command and preserves registrations for other
copies. Offline profiles are never loaded or edited; they may retain an
inert startup entry pointing to the removed binary. User configuration and
pairing/TLS data are retained unless the user clears them separately.

## Signing

Sign both:

- `wakemate-companion.exe`
- the generated installer `.exe`

Authenticode signing is strongly recommended before public distribution. See `docs/RELEASE_CHECKLIST.md` for exactly what a signing certificate purchase/setup involves.

## Windows Authenticode Signing (once you have a certificate)

1. Obtain a code-signing certificate from a public CA (e.g. DigiCert, Sectigo) or your organization's internal PKI -- either an OV (`.pfx`/hardware token) or EV certificate. EV certificates build SmartScreen reputation faster but require a hardware token and stricter identity verification.
2. Uncomment and fill in the `SignTool` line already present (commented out) in `wakemate-companion.iss`:
   `SignTool=signtool.exe sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $f`
3. In CI, store the certificate (base64-encoded `.pfx`) and its password as GitHub Actions repository secrets, decode it to a temp file in the workflow, and pass its path/password to `signtool` -- never commit a certificate or password to the repository.
4. Without a certificate, the installer and `.exe` from this pipeline are **unsigned** development artifacts. Signing and clean-machine SmartScreen/installer testing remain required before public distribution; signing does not guarantee a particular reputation outcome.

## macOS Packaging

See `installer/macos/package-macos.sh` and `docs/MACOS_BUILD.md` -- macOS packaging today produces an **unsigned, non-notarized** `.dmg` of the current headless-service build; there is no tray/pairing UI on macOS yet and no Apple Developer signing credentials configured in this repository.
