# Redistributable Bundle

Place the official Microsoft Visual C++ x64 redistributable in this folder with this exact filename:

`VC_redist.x64.exe`

The Inno Setup installer is configured to:

- bundle that file if present
- check whether the Visual C++ x64 runtime is already installed
- run the redistributable silently only when needed

This prerequisite is for end-user runtime support on Windows. It does not fix this machine's current local build-toolchain problem.
