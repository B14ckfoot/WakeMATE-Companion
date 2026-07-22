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
