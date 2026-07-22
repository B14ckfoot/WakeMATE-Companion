# WakeMATE Companion

WakeMATE Companion is the Rust desktop counterpart to the [WakeMATE mobile app](../../Wakemate-Mobile): a tray app that discovers this computer on the local network, sends Wake-on-LAN packets, and accepts authenticated remote control commands once a phone has paired with it and been approved on this desktop.

Supported today: **Windows** (tray icon, pairing UI, startup integration). macOS/Linux builds compile and run as a headless local network service, but do not yet have the tray/pairing UI -- see [docs/MACOS_BUILD.md](./docs/MACOS_BUILD.md) for exactly what's missing and the plan to close that gap.

## Security Posture

- HTTP binds to `127.0.0.1` unless `allow_remote_connections` is explicitly enabled; UDP discovery additionally requires `allow_discovery`.
- Input and power commands are disabled by default and require both a config flag *and* a one-time desktop approval (see Pairing below) to turn on.
- Detailed device info (`GET /v1/info`) requires the pairing token by default.
- The pairing token lives in the OS credential store (Windows Credential Manager today; the `keyring` crate also supports macOS Keychain once macOS packaging lands) instead of a plaintext file. Existing plaintext tokens from older installs are migrated in automatically the first time the app runs.
- Token comparison is constant-time, and repeated bad tokens against any authenticated endpoint trigger a per-IP lockout (see [docs/SECURITY_MODEL.md](./docs/SECURITY_MODEL.md)).
- The pre-logon boot service (see Architecture) refuses pairing activation and all input commands unconditionally -- there's no one present to approve anything in that context.
- A single-instance lock stops a second copy of WakeMATE from running.

To use WakeMATE over your LAN, you must explicitly opt in by setting:

- `allow_remote_connections` to `true`
- `allow_discovery` to `true` if you want UDP discovery
- `allow_input_commands` and `allow_power_commands` only if you want those capabilities (also gated by the pairing approval below)

## Architecture

WakeMATE has two different behaviors:

1. Offline wake path
   - Send a Wake-on-LAN magic packet to a saved MAC address.
   - This works even when the target PC is asleep.
2. Online control path
   - Once the PC is awake and remote access is enabled, the companion accepts authenticated commands for status and system actions.
   - On Windows installs, WakeMATE registers a headless boot task (`schtasks /RU SYSTEM`, since Task Scheduler requires that account for a trigger that fires before any user signs in) so `/v1/health` and `/v1/info` can answer before sign-in, while the tray app still starts at logon. This pre-logon instance is intentionally locked down further than the normal tray-hosted server: it refuses pairing activation and every input command outright, since no one is present to approve anything and no interactive desktop session exists to receive injected input anyway.

Important note: a sleeping PC cannot receive HTTP commands through the app server. Wake-on-LAN is handled by the network adapter, not by the app process.

## Pairing

Scanning the QR code from the tray no longer silently grants control. The flow is now:

1. The phone reads the token from the QR code and calls `POST /v1/pairing/activate` with it.
2. If the token is valid and a desktop session is running WakeMATE's tray app, a native Windows dialog pops up on the desktop: *"A device at `<ip>` wants to pair with this computer and enable remote mouse, keyboard, and power control... Allow it?"*
3. Only clicking **Yes** on that desktop dialog sets `allow_input_commands` and `allow_power_commands` to `true` and saves them. The HTTP call itself returns immediately with `status: "pending_approval"` rather than blocking, since the person may take a few seconds to notice the prompt.
4. If no tray app is running anywhere to show that prompt (including the headless pre-logon service), activation is refused with a clear error instead of silently granting nothing or, worse, silently granting everything.

Rotating the pairing token (from the tray menu) invalidates every previously paired phone at once -- there is currently no per-device revocation, only revoke-all-via-rotation. "Reset Companion..." in the tray menu goes further: after a confirmation dialog, it clears the stored token, all pairing/capability flags, and all other local settings back to secure defaults (a fresh token is generated immediately after). This only touches local device state; WakeMATE has no cloud account to sign out of.

## API

Public endpoints:

- `GET /`
- `GET /v1/health`

Authenticated endpoints (require `x-wakemate-token: <token>`; rate-limited per source IP):

- `GET /v1/info`
- `GET /v1/pairing/check`
- `POST /v1/pairing/activate` -- see Pairing above; returns `pending_approval`, not an immediate grant
- `POST /v1/wake`
- `POST /v1/command`

Discovery:

- UDP port `41234` by default
- discovery message `wakemate:discover`
- disabled unless `allow_remote_connections` and `allow_discovery` are both `true`
- unauthenticated discovery response includes device name, local IP, MAC address when available, API port, and version

## Example Command Payloads

Wake another device:

```json
{
  "mac": "00:11:22:33:44:55",
  "broadcast": "255.255.255.255",
  "port": 9
}
```

Mouse move:

```json
{
  "type": "mouse_move",
  "delta_x": 12,
  "delta_y": -8
}
```

Mouse click:

```json
{
  "type": "mouse_click",
  "button": "left",
  "double": false
}
```

Keyboard combo:

```json
{
  "type": "key_press",
  "key": "CTRL+ALT+DELETE"
}
```

Text input:

```json
{
  "type": "text_input",
  "text": "hello from WakeMATE"
}
```

Media command:

```json
{
  "type": "media",
  "action": "play_pause"
}
```

System command:

```json
{
  "type": "system",
  "action": "lock"
}
```

## Configuration

On first run, WakeMATE creates `wakemate.config.json` in the app-data folder:

- Windows: `%APPDATA%\WakeMATE Companion\wakemate.config.json`
- macOS: `~/Library/Application Support/WakeMATE Companion/wakemate.config.json`
- Linux: `$XDG_CONFIG_HOME/WakeMATE Companion/wakemate.config.json` or `~/.config/WakeMATE Companion/wakemate.config.json`

The pairing token itself is **not** stored in this file once the OS credential store is available -- see Security Posture above. `api_token` in the file will read as an empty string in that case; `token_storage` records whether the token currently lives in the OS credential store (`"keyring"`) or, as a fallback when the credential store is unavailable, in this file (`"file"`).

A sample config is included as [wakemate.config.example.json](./wakemate.config.example.json). There is no `.env` -- this is a config-file-based app, not an environment-variable-based one; see [.env.example](./.env.example) for the one supported environment variable (`RUST_LOG`).

Main fields:

- `bind_address`: LAN bind target used only when remote access is enabled
- `discovery_port`: UDP discovery port
- `discovery_message`: discovery probe string
- `api_token` / `token_storage`: see above
- `device_name`: friendly device name
- `launch_on_startup`: registers WakeMATE under the current Windows user startup key for the tray app, enabled by default
- `allow_input_commands`: enables mouse, keyboard, and media control (still requires desktop pairing approval; see Pairing)
- `allow_power_commands`: enables sleep, restart, shutdown, lock, and logoff (still requires desktop pairing approval)
- `allow_remote_connections`: when `false`, HTTP is forced to `127.0.0.1`
- `allow_discovery`: enables UDP discovery, but only when remote access is also enabled
- `require_auth_for_info`: requires the pairing token for `GET /v1/info`

Wake-on-LAN fields returned by `GET /v1/info` after authentication:

- `interface_name`
- `mac_address`
- `subnet_mask`
- `broadcast_address`
- `ping_address`
- `wol_port`

## Tray App (Windows)

WakeMATE starts as a tray app and looks for custom icons in:

1. `assets/tray-icon.ico`
2. `assets/tray-icon.png`

If neither file exists, WakeMATE uses a built-in fallback icon drawn in the WakeMATE brand color (see [src/theme.rs](./src/theme.rs)).

Tray actions:

- `View Pairing QR Code` -- renders a QR code for the current `api_token`, styled with the WakeMATE brand palette; closes on click-away or `Esc`
- `Rotate Pairing Token` -- invalidates all currently paired phones
- `Launch on Windows Startup` -- toggle, kept in sync with the Windows startup registration
- `Open Data Folder` -- opens the config directory for troubleshooting
- `Reset Companion...` -- see Pairing above
- `Quit WakeMATE`

The status line and tray tooltip show one of a small set of states (`Server not running`, `Local only`, `Ready to pair`, `Paired`, or an error) rather than raw address details, so it's obvious at a glance what WakeMATE is currently doing.

## Local Development

1. Install Rust via `rustup`
2. Review `wakemate.config.example.json`
3. On Windows, open Visual Studio Developer PowerShell
4. Run `cargo run`

Run the full format/lint/test/build quality gate before committing:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1
```

For Windows release packaging, run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1
```

For normal Windows use, launch the built `wakemate-companion.exe` directly. The binary is configured as a tray app, so it starts without leaving a console window open.

The Windows installer also prepares a clean-install config with `allow_remote_connections`, `allow_discovery`, and `launch_on_startup` enabled so the mobile app can discover the PC immediately on first launch, and it registers a boot-time headless server task so wake status can become reachable before user sign-in. Existing user configs are left alone on reinstall or upgrade. Input/power commands stay off (and gated behind pairing approval) regardless.

## Documentation

- [docs/SECURITY_MODEL.md](./docs/SECURITY_MODEL.md) -- threat model and current mitigations
- [docs/MACOS_BUILD.md](./docs/MACOS_BUILD.md) -- current macOS support, packaging, and the tray-parity porting plan
- [docs/WINDOWS_BUILD.md](./docs/WINDOWS_BUILD.md) -- local Windows toolchain notes
- [docs/TROUBLESHOOTING.md](./docs/TROUBLESHOOTING.md) -- common issues and how to diagnose them
- [docs/RELEASE_CHECKLIST.md](./docs/RELEASE_CHECKLIST.md) -- before shipping a public download
- [docs/PRIVACY_TEMPLATE.md](./docs/PRIVACY_TEMPLATE.md), [docs/EULA_TEMPLATE.txt](./docs/EULA_TEMPLATE.txt), [docs/THIRD_PARTY_NOTICES_TEMPLATE.md](./docs/THIRD_PARTY_NOTICES_TEMPLATE.md) -- legal templates, still placeholders

## Integration Note

For true Wake-on-LAN behavior, the mobile app should save the authenticated `mac_address`, `broadcast_address`, and `wol_port` while the PC is online. Later, when the PC is asleep or off, the app can send the magic packet directly using those saved values.
