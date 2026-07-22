# WakeMATE Companion

WakeMATE Companion is a Rust desktop tray app for discovering a computer on the local network, sending Wake-on-LAN packets, and accepting authenticated remote control commands from a paired mobile app.

## Security Posture

This repo is now set up with secure defaults:

- remote API access is disabled by default
- UDP discovery is disabled by default
- input and power commands are disabled by default
- detailed device info requires the pairing token by default
- config is stored in the user app-data folder instead of next to the executable
- the pairing token can be rotated from the Windows tray menu

To use WakeMATE over your LAN, you must explicitly opt in by setting:

- `allow_remote_connections` to `true`
- `allow_discovery` to `true` if you want UDP discovery
- `allow_input_commands` and `allow_power_commands` only if you want those capabilities

## Architecture

WakeMATE has two different behaviors:

1. Offline wake path
   - Send a Wake-on-LAN magic packet to a saved MAC address.
   - This works even when the target PC is asleep.
2. Online control path
   - Once the PC is awake and remote access is enabled, the companion accepts authenticated commands for status and system actions.
   - On Windows installs, WakeMATE now registers a headless boot task so the API can come online before user sign-in, while the tray app still starts at logon.

Important note: a sleeping PC cannot receive HTTP commands through the app server. Wake-on-LAN is handled by the network adapter, not by the app process.

## API

Public endpoints:

- `GET /`
- `GET /v1/health`

Authenticated endpoints:

- `GET /v1/info`
- `GET /v1/pairing/check`
- `POST /v1/pairing/activate`
- `POST /v1/wake`
- `POST /v1/command`

Auth header:

- `x-wakemate-token: <your token>`

Pairing activation:

- `POST /v1/pairing/activate` enables `allow_input_commands` and `allow_power_commands` for the paired computer and saves the updated config.

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

A sample config is included as [wakemate.config.example.json](./wakemate.config.example.json).

Main fields:

- `bind_address`: LAN bind target used only when remote access is enabled
- `discovery_port`: UDP discovery port
- `discovery_message`: discovery probe string
- `api_token`: secret token required in `x-wakemate-token`
- `device_name`: friendly device name
- `launch_on_startup`: registers WakeMATE under the current Windows user startup key for the tray app, enabled by default
- `allow_input_commands`: enables mouse, keyboard, and media control
- `allow_power_commands`: enables sleep, restart, shutdown, lock, and logoff
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

## Tray App

On Windows, WakeMATE starts as a tray app and looks for custom icons in:

1. `assets/tray-icon.ico`
2. `assets/tray-icon.png`

The tray menu includes a `Launch on Windows Startup` toggle that keeps the config file and the current user's Windows startup registration in sync. Clean installs default this setting to `on`.

On Windows, the installer also registers a separate boot-time headless server task so the authenticated API can answer before the desktop user signs in after Wake-on-LAN.

If neither file exists, WakeMATE uses a built-in fallback icon.

Tray actions:

- `Show Pairing QR Code`
- `Rotate Pairing Token`
- `Open Data Folder`
- `Quit WakeMATE`

The pairing popup renders a QR code for the current `api_token` and closes on click-away or `Esc`.

## Local Development

1. Install Rust via `rustup`
2. Review `wakemate.config.example.json`
3. On Windows, open Visual Studio Developer PowerShell
4. Run `cargo run`

For Windows release packaging, run:

`powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1`

For normal Windows use, launch the built `wakemate-companion.exe` directly. The binary is configured as a tray app, so it starts without leaving a console window open.

The Windows installer also prepares a clean-install config with `allow_remote_connections`, `allow_discovery`, and `launch_on_startup` enabled so the mobile app can discover the PC immediately on first launch, and it registers a boot-time headless server task so wake status can become reachable before user sign-in. Existing user configs are left alone on reinstall or upgrade.

## Release Docs

Before shipping a public Windows download, review:

- [docs/RELEASE_CHECKLIST.md](./docs/RELEASE_CHECKLIST.md)
- [docs/PRIVACY_TEMPLATE.md](./docs/PRIVACY_TEMPLATE.md)
- [docs/THIRD_PARTY_NOTICES_TEMPLATE.md](./docs/THIRD_PARTY_NOTICES_TEMPLATE.md)
- [installer/wakemate-companion.iss](./installer/wakemate-companion.iss)

## Integration Note

For true Wake-on-LAN behavior, the mobile app should save the authenticated `mac_address`, `broadcast_address`, and `wol_port` while the PC is online. Later, when the PC is asleep or off, the app can send the magic packet directly using those saved values.
