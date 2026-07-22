# WakeMATE Assets

Drop custom desktop companion assets in this folder.

Windows executable and installer icon:

- `assets/app-icon.ico`

Current tray icon lookup order on Windows:

1. `assets/tray-icon.ico`
2. `assets/tray-icon.png`

If neither file exists, the Rust companion falls back to a built-in icon so the tray app still starts.

`app-icon.ico` is used for the compiled Windows `.exe` and installer branding.

`build-release.ps1` also stages these assets into `target/release` so a raw release-folder launch uses the same runtime tray icons as the installed app.

When running with `cargo run`, WakeMATE looks in the repo-root `assets/` folder first.

## Brand Assets Present vs. Still Needed

Present and already used: `app-icon.ico`, `tray-icon.png`, `LOGO.Brand.png`, `MenuBar.ICON.png`. Color tokens extracted from the mobile app's actual screens live in `src/theme.rs` and drive the tray popup and fallback icon; see `docs/MACOS_BUILD.md`'s packaging section for how `LOGO.Brand.png` is converted into a macOS `.icns` when packaging.

Still needed from the design team before a polished release:
- A macOS-appropriate menu-bar template icon (monochrome, respects light/dark menu bar automatically) once the macOS tray port (`docs/MACOS_BUILD.md`) lands -- the current Windows tray icon is full-color and not a menu-bar template image.
- A finalized, non-Expo-default bundle identifier and app name for both the mobile app and this Companion if `com.anonymous.wakematemobile` / `com.wakemate.companion` are placeholders rather than the real registered identifiers.
- Confirmation of the exact brand hex values in `src/theme.rs` against an authoritative brand guide, if one exists beyond what's inferable from the mobile app's source.
