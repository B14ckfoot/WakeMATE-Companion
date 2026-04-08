# Windows Build Notes

## Recommended Release Command

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
```

The helper script:

- locates the latest Visual Studio installation
- imports the x64 developer environment automatically
- adds the active MSVC desktop `lib\x64` directory to `LIB` when needed
- falls back to `lib\onecore\x64` on this machine if the standard desktop import libraries are missing
- runs `cargo build --release`

The expected output is:

`target\release\wakemate-companion.exe`

## Long-Term Toolchain Fix

This Visual Studio install currently exposes `msvcrt.lib` under `VC\Tools\MSVC\<version>\lib\onecore\x64` instead of the usual desktop `lib\x64` path. The helper script works around that so packaging can continue.

For the cleanest setup, use Visual Studio Installer and confirm these components are installed or repaired for the active toolchain:

- MSVC x64/x86 build tools
- Windows SDK
- Desktop C++ libraries for x64

After repair, verify `VC\Tools\MSVC\<version>\lib\x64\msvcrt.lib` exists and then re-run `.\build-release.ps1`.
