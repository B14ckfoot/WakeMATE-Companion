# macOS developer preview

Windows is the interactive target for version 0.3. macOS has a headless source
path and an unsigned packaging script, but this revision has not been built or
run on a Mac during the redesign. Windows test results do not validate macOS.

There is no macOS menu-bar setup, QR display, permission approval, or login-item
integration. New configurations remain local-only pending interactive consent.
Do not change consent or authorization files to imitate a desktop approval.
The headless runtime refuses interactive input and power commands. Its input
backend reports unsupported; it does not claim to have injected events.

For development on a Mac with Rust and the platform build tools:

```bash
cargo fmt --all -- --check
cargo clippy --release --all-targets --locked -- -D warnings
cargo test --release --locked
cargo audit
cargo build --release --locked
./installer/macos/package-macos.sh
```

The script requires macOS and hdiutil, derives the version from Cargo, and
creates a fresh versioned folder under dist/macos/. It records source and
binary hashes, a DMG checksum, and provenance without overwriting old packages.
CI defines these checks; its configuration is not evidence that they ran for
this revision. Capture and fix platform-specific failures on a Mac before
calling the preview validated.

Production support requires a native consent/settings surface, per-user
startup, platform permission handling, an implemented input backend, and real
machine pairing and sleep/wake tests. It also requires genuine Developer ID
signing credentials, hardened-runtime review, notarization, stapling, and
verification of the mounted package. No signing credentials are configured by
the redesign, and no signed or notarized macOS artifact is provided.
