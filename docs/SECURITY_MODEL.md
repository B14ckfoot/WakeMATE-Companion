# WakeMATE Security Model

## Default State

- HTTP binds to `127.0.0.1` unless `allow_remote_connections` is explicitly enabled.
- UDP discovery is disabled unless both `allow_remote_connections` and `allow_discovery` are enabled.
- Input and power commands are disabled by default.
- Detailed device info requires the pairing token by default.
- The pairing token is stored in the user app-data directory, not beside the installed executable.

## Windows Installer Behavior

- On a clean Windows install, the installer prepares the first-run config with `allow_remote_connections` and `allow_discovery` enabled so the mobile app can auto-discover the PC immediately.
- Existing configs are preserved on reinstall or upgrade.
- Input and power commands still stay disabled unless the user enables them.

## Trust Model

WakeMATE is designed for use on trusted devices and trusted local networks. If remote access is enabled, anyone who obtains the pairing token may be able to control the machine according to the enabled feature flags.

## Operational Guidance

- Keep remote access disabled unless you actively need it.
- Enable discovery only when your mobile app needs auto-discovery.
- Rotate the pairing token after testing, screenshots, screen sharing, or suspected exposure.
- Do not enable remote input or power actions on shared or public machines unless that risk is acceptable.

## Current Authentication

- Single shared bearer token via `x-wakemate-token`
- No built-in account system
- No built-in TLS termination

## Release Recommendation

If you later expose WakeMATE beyond a trusted home LAN, add stronger session design, transport protection, rate limiting, and explicit device-pairing UX before doing so.
