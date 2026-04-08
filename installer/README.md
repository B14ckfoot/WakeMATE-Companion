# Installer Notes

This folder contains an Inno Setup template for packaging WakeMATE Companion as a Windows installer.

## Before Building The Installer

- Produce a signed `target\release\wakemate-companion.exe`
- Place the official Microsoft Visual C++ x64 redistributable at `installer\redist\VC_redist.x64.exe`
- Replace placeholders in `wakemate-companion.iss`
- Replace `..\docs\EULA_TEMPLATE.txt` with your final EULA text
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

Authenticode signing is strongly recommended before public distribution.
