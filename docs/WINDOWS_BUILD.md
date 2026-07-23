# Windows Build Notes

## Recommended Release Command

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
```

The helper script:

- locates the first installed Visual Studio toolchain that has a complete x64 compiler, C headers, and import libraries
- imports the x64 developer environment automatically
- adds the active MSVC desktop `lib\x64` directory to `LIB` when needed
- falls back to `lib\onecore\x64` on this machine if the standard desktop import libraries are missing
- runs `cargo build --release`
- stages runtime icon assets into `target\release\assets`
- copies `assets\app-icon.ico` into `target\release\app-icon.ico`

The expected output is:

`target\release\wakemate-companion.exe`

For a release-folder run that matches the installed app more closely, launch the `.exe` from `target\release` after the helper script has staged the icon files.

## Long-Term Toolchain Fix

The Visual Studio 18 installation on this development machine is incomplete: it has no usable `vcvarsall.bat` environment, omits `include\vcruntime.h`, and exposes `msvcrt.lib` only under `lib\onecore\x64`. Rustls uses `ring`, whose build script compiles C code, so a `LIB`-only workaround is not enough.

The helper validates each installed MSVC toolchain before importing it. On this machine it skips Visual Studio 18 and uses the complete Visual Studio 2022 C++ toolchain. It still supports `lib\onecore\x64` as a last-resort library fallback when the selected compiler and headers are otherwise complete.

For the cleanest setup, use Visual Studio Installer and confirm these components are installed or repaired for the active toolchain:

- MSVC x64/x86 build tools
- Windows SDK
- Desktop C++ libraries for x64

After repair, verify all three paths exist under the same toolset and then re-run `.\build-release.ps1`:

- `bin\HostX64\x64\cl.exe`
- `include\vcruntime.h`
- `lib\x64\msvcrt.lib`
