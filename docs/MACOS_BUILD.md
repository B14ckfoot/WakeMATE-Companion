# macOS Build Notes

## Current State (read this first)

WakeMATE Companion's source **compiles and runs on macOS today**, but only as a headless local network service:

- `src/main.rs` has a `#[cfg(not(target_os = "windows"))]` path that runs the axum HTTP server and UDP discovery responder directly, with graceful shutdown on Ctrl+C/SIGTERM.
- `src/system.rs` already has macOS branches for `sleep` (`pmset sleepnow`), `restart`/`shutdown` (`shutdown -r|-h now`), and `open_path` (`open`). `lock` and `logoff` are not implemented on macOS yet (`perform_system_action` returns a clear "not implemented" error for those two, rather than doing nothing silently).
- There is **no tray/menu-bar icon, no pairing QR window, and no login-item toggle on macOS**. `src/tray.rs` -- the icon, the pairing popup, the "Reset Companion" flow, the desktop pairing-confirmation dialog -- is entirely behind `#[cfg(target_os = "windows")]`.

Practically: a macOS build today is a background network service you'd start from a terminal or a launchd plist, with the same HTTP API as Windows (Wake-on-LAN, health/info, and -- once separately approved, see below -- remote input/power), but no visual presence and no way to approve a pairing request from the desktop, so **pairing activation is refused on macOS today** (see `PairingCoordinator::unavailable()` in `src/pairing.rs`). It is not a finished consumer app.

This document describes exactly what's needed to close that gap, and how to package what exists today into a `.dmg` in CI.

## Why the tray wasn't ported blind in this pass

The Windows tray implementation depends on:

1. **`winit`'s Windows-only extension trait** (`platform::windows::{CornerPreference, WindowAttributesExtWindows}`) for the borderless, rounded, shadowed popup window.
2. **Raw `GetAsyncKeyState`/`GetCursorPos` Win32 calls** for click-away detection outside the popup (in addition to the `WindowEvent::Focused(false)` handler, which already exists and may be sufficient on its own).
3. **Win32 registry calls** for the "launch on startup" toggle.
4. **A raw `MessageBoxW` call** for the native pairing-confirmation dialog.

`tray-icon`, `winit`, and `softbuffer` (the three crates the popup rendering depends on) do all declare macOS support, and the QR/text-rendering code in `tray.rs` (the `render_pairing_popup`, `draw_qr_code`, `draw_text` functions and friends) is already 100% cross-platform pixel-buffer code with no Windows-specific calls -- that part could move to macOS largely as-is. But items 1, 3, and 4 above need real macOS-specific replacements, and there is no macOS machine or CI runner available in this working session to compile-check or visually verify that port. Shipping unverified, hand-written AppKit/Cocoa-adjacent code as if it were tested would violate this project's own instruction not to claim untested platform work is production-ready, so it was intentionally left as documented follow-up work rather than guessed at.

## Porting Plan (concrete, for the next macOS-capable session)

1. **Cargo.toml**: move `tray-icon`, `winit`, `softbuffer`, `image`, `qrcodegen`, `font8x8` out of the `[target.'cfg(target_os = "windows")'.dependencies]` section into a `[target.'cfg(any(target_os = "windows", target_os = "macos"))'.dependencies]` section (Linux stays headless-only; it was never a target platform here).
2. **`src/tray.rs`**: change `#[cfg(target_os = "windows")] mod tray;` in `main.rs` to `#[cfg(any(target_os = "windows", target_os = "macos"))]`, then:
   - Gate the `platform::windows::WindowAttributesExtWindows` builder calls behind `#[cfg(target_os = "windows")]`, with a plain `Window::default_attributes()` (no corner/shadow styling) on macOS to start.
   - Replace the `GetAsyncKeyState`/`GetCursorPos` polling in `should_close_for_click_away` with a `#[cfg(target_os = "windows")]` block, and rely on the existing `WindowEvent::Focused(false)` handler alone on macOS initially (add the platform-specific polling back only if focus-loss alone proves insufficient in testing).
   - Replace the `MessageBoxW`-based `system::confirm_pairing_dialog`/`confirm_dialog` with an `NSAlert`-based equivalent on macOS (the `objc2`/`objc2-app-kit` crates -- already pulled in transitively by `winit`/`tray-icon` per `Cargo.lock` -- expose this) or, as a lower-risk first cut, shell out to `osascript -e 'display dialog ... buttons {"Deny","Allow"} ...'`, which is what the mobile-adjacent Node/Electron ecosystem commonly does for a quick native Yes/No prompt without deep AppKit integration.
3. **`src/system.rs`**: add macOS equivalents for:
   - `sync_launch_on_startup` -- write/remove a `~/Library/LaunchAgents/com.wakemate.companion.plist` (a `launchctl load`/`unload` pair, or `SMAppService` on macOS 13+) instead of a registry Run key.
   - `enable_preferred_dark_mode` -- not applicable; macOS apps follow the system appearance automatically, so this can just stay a no-op.
   - Do not add a privileged `LaunchDaemon` for pre-login networking. Windows also intentionally avoids a privileged pre-login Companion server now; direct phone Wake-on-LAN covers waking without splitting user-scoped credentials/TLS state across security principals.
   - Implement `lock` (there is no simple CLI equivalent to Windows' `LockWorkStation`; the common approach is `osascript -e 'tell application "System Events" to keystroke "q" using {control down, command down}'` or invoking the login window via a small Swift/ObjC helper) and `logoff` (`osascript -e 'tell application "System Events" to log out'`, which prompts the user rather than forcing it -- macOS does not have a silent forced-logoff CLI primitive by default).

## Packaging: What Exists Today

`installer/macos/package-macos.sh` builds whatever the current build target is (the headless service, until the above lands) into `WakeMATE Companion.app` and an **unsigned** `.dmg`:

```bash
cargo build --release
./installer/macos/package-macos.sh
```

`.github/workflows/ci.yml` runs this on a `macos-latest` GitHub Actions runner on every push and uploads the resulting `.dmg` as a build artifact -- this has been prepared and is ready to run, but has not been executed in this working session (there is no macOS machine here, and no GitHub remote is configured for this repository yet to trigger Actions). Do not treat an unrun workflow as verified; trigger it once this repo has a GitHub remote and confirm the artifact actually mounts and launches before relying on it.

`installer/macos/Info.plist.template` sets a **placeholder** bundle identifier (`com.wakemate.companion`) that must be replaced with the identifier registered under your actual Apple Developer team before signing.

## What Signed, Notarized macOS Distribution Requires

A `.dmg` a random Mac will actually open without a Gatekeeper warning needs, in order:

1. **An Apple Developer Program membership** (paid, per-organization or per-individual) to obtain a **Developer ID Application** certificate.
2. **Code signing** the `.app` bundle (`codesign --deep --force --options runtime --sign "Developer ID Application: <Name> (<TeamID>)" "WakeMATE Companion.app"`) with the **hardened runtime** enabled (`--options runtime`), plus an entitlements file if the app needs any hardened-runtime exception (none are anticipated for a local-network HTTP/UDP service beyond the local network usage description already in `Info.plist.template`).
3. **Notarization**: submit the signed `.app` (typically zipped, or the final signed `.dmg`) to Apple via `xcrun notarytool submit ... --keychain-profile <profile> --wait`, using an app-specific password or API key tied to the same Developer account.
4. **Stapling** the notarization ticket to the `.dmg` (`xcrun stapler staple WakeMATE-Companion.dmg`) so it verifies offline.
5. **Universal binary** (optional but recommended): build both `--target x86_64-apple-darwin` and `--target aarch64-apple-darwin`, then combine with `lipo -create -output wakemate-companion <intel-binary> <arm-binary>` before packaging, so one `.dmg` serves both Intel and Apple Silicon Macs.

None of steps 1-4 can be completed without an actual Apple Developer account and a macOS machine (or a macOS CI runner with the certificate imported as a secret) to run `codesign`/`notarytool`. This repository does not have those credentials and none have been fabricated or stubbed in; `[UninstallRun]`-equivalent signing steps are intentionally absent from `package-macos.sh` rather than filled with placeholder secrets.

## Minimum macOS Version

`Info.plist.template` sets `LSMinimumSystemVersion` to `11.0` (Big Sur), matching current `winit`/`tray-icon` baseline support. Revisit this once the tray port (above) lands and is actually tested on a real Mac.
