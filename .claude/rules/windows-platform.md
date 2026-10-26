---
paths:
  - "src/tray.rs"
  - "src/desktop.rs"
  - "src/system.rs"
  - "src/network.rs"
  - "src/runtime_status.rs"
  - "src/windows_input.rs"
  - "src/input_executor.rs"
  - "src/secure_attention.rs"
  - "src/main.rs"
  - "build.rs"
  - "build-release.ps1"
  - "scripts/**/*.ps1"
  - "installer/**/*.ps1"
  - "installer/**/*.iss"
  - "docs/WINDOWS_BUILD.md"
  - "docs/TROUBLESHOOTING.md"
---

# Windows lifecycle

- Run the companion as a normal interactive user and refuse elevated/service contexts before loading state. One owning session controls the listeners/desktop; other accounts must not receive remote input.
- Setup performs machine installation only and never launches the tray or initializes another account's per-user state. Normal Start-menu launch handles native setup and user startup registration.
- Require onboarding consent before LAN bind/discovery. Preserve explicit private adapter selection; invalid values fail without widening the bind, and stale addresses report failure.
- Keep private LocalSubnet, installed-executable, protocol/port-scoped firewall rules. Report HTTPS and UDP readiness separately; do not advertise readiness before bind succeeds.
- No SYSTEM pre-login API. Retired headless tasks exit without reading user state/listening. Direct phone Wake-on-LAN remains available independently.
- Keep startup entries safely quoted/user-scoped. Upgrade/shutdown must target the installed path and stop gracefully, preserving development copies and unrelated processes.
- Pause/revoke/reset and session/security-desktop transitions cancel queued/held input. Checked `SendInput` is not a route around UAC, physical modifiers, secure desktop, or Secure Attention Sequence policy.
- Do not synthesize Ctrl+Alt+Delete. Return structured unsupported/permission/failure results and perform only explicitly requested supported fallbacks.
- Preserve upgrade data and document retained state. Verify multi-user startup/uninstall cleanup in a clean VM. Unsigned binaries remain unsigned; never advise disabling Windows protections.
- Keep Windows APIs behind compile-time guards and unit tests free of real desktop input, installed configuration, or OS credential mutations.
