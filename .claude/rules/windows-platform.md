---
paths:
  - "src/tray.rs"
  - "src/system.rs"
  - "src/secure_attention.rs"
  - "src/input.rs"
  - "src/main.rs"
  - "build.rs"
  - "build-release.ps1"
  - "scripts/**/*.ps1"
  - "installer/**/*.ps1"
  - "installer/**/*.iss"
  - "installer/branding/**/*"
  - "installer/redist/**/*"
  - "assets/**/*"
  - "docs/WINDOWS_BUILD.md"
  - "docs/TROUBLESHOOTING.md"
---

# Windows platform

- Keep the interactive companion running as the normal user. Use administrator/SYSTEM privileges only for narrowly justified install, firewall, or pre-login registration work.
- Keep startup registration user-scoped and safely quoted. The pre-login scheduled task remains headless, refuses pairing/input, and is removed during uninstall.
- Limit inbound firewall access to the intended local/private-network behavior and companion executable; do not broaden profiles, ports, or programs without explicit security review.
- Preserve installer upgrade behavior and existing config/credential/TLS state. Uninstall must clean installed files, startup registration, scheduled tasks, and firewall rules without claiming retained app data was erased.
- Do not claim unsigned binaries are trusted or advise bypassing SmartScreen/antivirus. Authenticode needs real protected credentials, timestamps, and verification.
- Ctrl+Alt+Delete is the Windows Secure Attention Sequence: never inject it as keystrokes or bypass UAC/policy. Report `unsupported` or `permission_required`, and perform only an explicitly requested safe fallback.
- Keep Windows APIs and dependencies behind appropriate `cfg(target_os = "windows")` guards and retain tests for quoting, command classification, tray behavior, and cleanup-sensitive changes.
