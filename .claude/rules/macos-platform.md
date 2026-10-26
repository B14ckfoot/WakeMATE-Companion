---
paths:
  - "Cargo.toml"
  - "src/main.rs"
  - "src/system.rs"
  - "src/input.rs"
  - "src/input_executor.rs"
  - "installer/macos/**/*"
  - "docs/MACOS_BUILD.md"
---

# macOS preview boundary

- Label current macOS output a headless developer preview. It has no native setup/menu-bar approval parity and must not silently auto-approve devices.
- Keep shared protocol/security behavior aligned with Windows while guarding platform-specific input/UI code. Truthfully return unsupported/permission-required when a platform cannot perform an action.
- Respect Accessibility, Automation, local-network, login-item, and OS security permissions. Do not introduce a privileged pre-login LaunchDaemon as a setup shortcut.
- Production parity requires native menu-bar/setup/approval/status/revocation, normal-user startup, real permission handling, and real-machine QA.
- Distribution remains unsigned/non-notarized until real Developer ID, hardened runtime as applicable, notarization/stapling, and verification are configured. Build/package success alone is not runtime support.
