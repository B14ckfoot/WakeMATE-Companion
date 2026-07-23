# Installer Notes

This folder contains an Inno Setup template for packaging WakeMATE Companion as a Windows installer.

## Before Building The Installer

- Produce a signed `target\release\wakemate-companion.exe`
- Place the official Microsoft Visual C++ x64 redistributable at `installer\redist\VC_redist.x64.exe`
- Replace placeholders in `wakemate-companion.iss`
- Review `..\docs\EULA.txt` with qualified counsel for the intended release jurisdictions
- Review `INSTALL_WARNING.txt`
- Finalize `..\docs\THIRD_PARTY_NOTICES_TEMPLATE.md`

## Build

Build the release binary:

`powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1`

Then compile the installer:

`powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1`

Or have the installer script force a fresh release build first:

`powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1 -BuildRelease`

Open Inno Setup and compile:

`installer\wakemate-companion.iss`

The installer will bundle `installer\redist\VC_redist.x64.exe` and silently install it only when the target machine does not already have the Visual C++ runtime.

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
4. Without a certificate, the installer and `.exe` from this pipeline are **unsigned**. Windows SmartScreen will show an "Unknown publisher" warning on first run until enough installs build reputation (for OV certs) or immediately trust it (for EV certs). This is expected and disclosed, not a bug.

## macOS Packaging

See `installer/macos/package-macos.sh` and `docs/MACOS_BUILD.md` -- macOS packaging today produces an **unsigned, non-notarized** `.dmg` of the current headless-service build; there is no tray/pairing UI on macOS yet and no Apple Developer signing credentials configured in this repository.
