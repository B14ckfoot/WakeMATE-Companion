---
paths:
  - "Cargo.toml"
  - "src/main.rs"
  - "src/system.rs"
  - "src/tray.rs"
  - "installer/macos/**/*"
  - "docs/MACOS_BUILD.md"
---

# macOS platform

- Treat current macOS output as a developer-preview headless service, not a production companion. Pairing approval is unavailable until an interactive menu-bar implementation exists.
- Production parity requires a native menu-bar experience, pairing QR and explicit approval UI, login-item integration, clear status/revocation controls, and real-machine QA.
- Respect macOS Accessibility, Automation, local-network, login-item, and other privacy permissions. Explain required permission and return a truthful permission/unsupported result; never bypass controls.
- Prefer per-user startup integration. Do not introduce a privileged LaunchDaemon or pre-login capability without an explicit need and security review.
- A production distribution must use the real bundle identifier, Developer ID signing, hardened runtime as appropriate, notarization, stapling, and verification. Keep preview artifacts labeled unsigned/non-notarized.
- Guard macOS implementation separately from Windows, and do not reduce Windows tray, approval, startup, installer, or command behavior while adding parity.
- Validate platform work on macOS and mount/launch packaged artifacts before claiming support; CI configuration alone is not proof of runtime quality.
