# WakeMATE Companion

WakeMATE Companion is the desktop counterpart to the [WakeMATE mobile app](../../Wakemate-Mobile). It sits in your system tray, lets your phone find this computer on your home network, wakes other machines with Wake-on-LAN, and — only after you approve it on this desktop — accepts remote mouse, keyboard, media, and power commands from your paired phone.

> **Audit note (2026-07-23):** see [`WAKE_MATE_COMPANION_ARCHITECTURE_AUDIT.md`](../../WAKE_MATE_COMPANION_ARCHITECTURE_AUDIT.md) at the workspace root for the current architecture audit, confirmed integration issues, and the improvement roadmap. Known limitations found there are listed [below](#known-limitations).

## Supported Platforms

| Platform | Status |
| --- | --- |
| Windows 10 / 11, x64 | **Supported** — tray app, pairing UI, startup integration, installer |
| macOS | **Developer preview only** — compiles and runs as a headless network service; no menu-bar icon or pairing UI yet, so pairing cannot be approved on macOS. Not for end users. See [docs/MACOS_BUILD.md](./docs/MACOS_BUILD.md). |
| Linux | Compiles headless; not a target platform |

---

## Install (End Users — Windows)

1. Download **`WakeMATE Companion Setup.exe`** from the project's Releases page *(link placeholder — publish via the draft-release workflow, see Development below)*.
2. Run the installer. **The installer is not yet code-signed**, so Windows SmartScreen may show "Windows protected your PC" — click **More info → Run anyway** only if you downloaded it from the official Releases page and, ideally, verified the published SHA-256 checksum. Never disable SmartScreen or your antivirus to install it.
3. Approve the one-time administrator prompt. Admin rights are used **only during install/uninstall** (Program Files, the Microsoft VC++ runtime if missing, and a boot-time status service); the app itself runs as your normal user.
4. When the installer finishes, WakeMATE launches into the system tray automatically.

On a fresh install the companion is pre-configured so your phone can discover it (`allow_remote_connections`, `allow_discovery`, and launch-at-startup are enabled). Remote input and power control stay **off** until you explicitly approve pairing on this desktop.

No terminal, runtime installation, config editing, or manual port setup is required for normal use.

### First-run pairing with the mobile app

1. Make sure the phone and this PC are on the same Wi-Fi/LAN.
2. Click the WakeMATE tray icon → **View Pairing QR Code**.
3. In the WakeMATE mobile app, scan the QR code — either from **Add device → Scan QR** (saves the computer *and* pairs in one scan) or from **Settings** via the camera button next to the pairing-token field. The QR carries the device name, address, pairing token, HTTPS port, and certificate fingerprint (pairing contract v2). The phone pins that fingerprint before sending the token.
4. The mobile app saves the token and requests activation. A dialog appears **on this desktop**: *"A device at `<ip>` wants to pair…"*. Click **Yes** only if you just initiated pairing from your own phone. The phone polls the companion and shows the real outcome (approved, denied, or still waiting).
5. Remote mouse/keyboard/media/power controls are now enabled for the paired phone.

### Permissions the app asks for, in plain language

- **Windows Firewall prompt** (first time LAN access is enabled): allows your phone to reach the companion on your **private** network. Allow it for private networks only; without it, the phone cannot connect, but the app still runs locally.
- **Startup registration**: the tray app registers under your user's startup programs so it's available after login. Toggle it anytime via tray → **Launch on Windows Startup**.
- **Boot-time status service**: the installer registers a background task so your phone can see the PC's online status *before* anyone signs in. This pre-logon service is locked down — it refuses pairing and all input commands unconditionally.
- Nothing else: no microphone, camera, Bluetooth, or location access is used or requested.

### Everyday use

The tray icon's status line shows one of: `Server not running`, `Local only — not discoverable`, `Ready to pair`, `Paired — remote control enabled`, or an error. Tray actions:

- **View Pairing QR Code** — pair another phone (any valid phone can also re-scan)
- **Paired Devices** — lists every phone that completed pairing, each with a one-click **Revoke** (with confirmation); a revoked phone loses access immediately and must re-scan the QR to reconnect
- **Rotate Pairing Token** — immediately un-pairs **all** phones (the enrollment token rotates and every per-device token is revoked together)
- **Launch on Windows Startup** — toggle
- **Open Data Folder** — opens the config folder for troubleshooting
- **Reset Companion…** — clears the token, all pairing/capability flags, and all local settings back to secure defaults (with confirmation)
- **Quit WakeMATE**

### Updating

There is **no automatic update yet**. To update, download the newer installer and run it over the existing install — your settings, pairing token, and approvals are preserved (the installer only creates a fresh config when none exists).

### Uninstalling

Use Windows **Settings → Apps → WakeMATE Companion → Uninstall**. This removes the app, the boot-time service, and the startup registration. Your config folder (`%APPDATA%\WakeMATE Companion`), TLS identity, and pairing token stored in Windows Credential Manager are currently left behind; use tray → **Reset Companion…** *before* uninstalling to clear pairing settings, then remove the app-data folder if you also want the retained TLS identity deleted.

### Troubleshooting

See [docs/TROUBLESHOOTING.md](./docs/TROUBLESHOOTING.md). Quick checks: both devices on the same network; tray status says `Ready to pair` or `Paired`; firewall allowed for private networks; after a failed pairing prompt, retry from the phone (prompts are rate-limited to one per 10 seconds).

---

## Security & Privacy

- Everything is local — WakeMATE has **no cloud account, no telemetry**; the phone talks directly to this PC over your LAN.
- HTTPS binds to `127.0.0.1` unless LAN access is explicitly enabled; UDP discovery additionally requires its own flag.
- The companion generates a persistent self-signed certificate and the mobile app pins its SHA-256 fingerprint from the visual pairing QR. Public certificate authorities and DHCP-stable hostnames are not required.
- A plaintext HTTP listener remains enabled by default for one migration release so older phones keep working. Current QR-paired builds use pinned HTTPS; set `allow_insecure_http` to `false` after every phone has been upgraded and re-scanned.
- Input and power commands are off by default and require both a config flag **and** the one-time desktop approval dialog.
- The pairing token lives in the **Windows Credential Manager**, not in a plaintext file (older plaintext tokens are migrated automatically; the config file's `api_token` field then reads as empty — this is intentional).
- Token comparison is constant-time; repeated bad tokens trigger a per-IP lockout; a single-instance lock prevents duplicate copies.
- The pre-logon boot service refuses pairing activation and all input commands unconditionally.
- Full threat model: [docs/SECURITY_MODEL.md](./docs/SECURITY_MODEL.md).

## Known Limitations

Confirmed in the 2026-07-23 audit (items struck through were fixed the same day in protocol v2):

- ~~No transport encryption~~ — fixed for current mobile builds with self-signed TLS and QR fingerprint pinning. The old HTTP listener is still available behind `allow_insecure_http` during the migration window.
- ~~One shared token / no per-device revocation~~ — fixed in protocol v3: each phone enrolls for its own token (only its SHA-256 hash is stored on the PC) and can be revoked individually from tray → **Paired Devices**. The QR token remains the enrollment credential; rotating it still revokes everything at once.
- ~~Mouse drag/hold from the phone does not work~~ — fixed: the companion now implements the `mouse_button` down/up command.
- ~~The mobile "Add device → Scan QR" screen can't read the companion's QR~~ — fixed: the QR now carries a JSON payload (name, IP, port, MAC, token) both mobile scanners understand.
- ~~The phone reported pairing success before the desktop approved~~ — fixed: new `/v1/pairing/status` endpoint; the phone polls and reports approved/denied/waiting truthfully.
- **Unsigned binaries** — SmartScreen warnings until code signing lands.
- **No auto-update, no macOS UI** yet.

---

## Development (Contributors)

Everything below is for building from source; end users never need it.

### Architecture

WakeMATE has two behaviors:

1. **Offline wake path** — send a Wake-on-LAN magic packet to a saved MAC address; works while the target PC sleeps (handled by the network adapter, not the app).
2. **Online control path** — once awake and paired, the companion accepts authenticated pinned-HTTPS commands for status, wake relay, input, media, and power. On Windows, a headless boot task (`schtasks /RU SYSTEM`, required by Task Scheduler for pre-logon triggers) answers `/v1/health` and `/v1/info` before sign-in, while the tray app starts at logon. Plain HTTP is a temporary compatibility listener controlled by `allow_insecure_http`.

### API summary

Public: `GET /`, `GET /v1/health` (includes `protocol_version`, currently `2`).
Authenticated (`x-wakemate-token` header; per-IP rate-limited): `GET /v1/info`, `GET /v1/pairing/check`, `GET /v1/pairing/status` (returns `approval: idle|pending|approved|denied` plus the capability flags, so the phone can poll for the desktop's Yes/No), `POST /v1/pairing/activate` (returns `pending_approval`; grant happens only via the desktop dialog), `POST /v1/wake`, `POST /v1/command`.
Discovery: UDP port `41234`, probe string `wakemate:discover`, JSON reply with device name, IP, MAC, legacy API port, TLS port, TLS fingerprint, version, and `protocol_version`; only active when remote access **and** discovery are both enabled. Discovery metadata is informational; the phone only trusts a fingerprint obtained through the visual QR channel.
Pairing QR (contract v2): JSON `{"v":2,"kind":"wakemate-pairing","name":...,"ip":...,"api_port":...,"tls_port":...,"fp":...,"mac":...,"token":...,"protocol_version":2}`; `ip`/`mac` are omitted when the network can't be detected. `fp` is the lowercase SHA-256 digest of the leaf certificate's DER bytes.

Command payload examples (all `POST /v1/command`, snake_case tagged):

```json
{ "type": "mouse_move", "delta_x": 12, "delta_y": -8 }
{ "type": "mouse_click", "button": "left", "double": false }
{ "type": "mouse_button", "button": "left", "action": "down" }
{ "type": "key_press", "key": "CTRL+ALT+DELETE" }
{ "type": "text_input", "text": "hello from WakeMATE" }
{ "type": "media", "action": "play_pause" }
{ "type": "system", "action": "lock" }
```

Wake payload (`POST /v1/wake`): `{ "mac": "00:11:22:33:44:55", "broadcast": "255.255.255.255", "port": 9 }`

### Configuration reference

First run creates `wakemate.config.json` under:

- Windows: `%APPDATA%\WakeMATE Companion\`
- macOS: `~/Library/Application Support/WakeMATE Companion/`
- Linux: `$XDG_CONFIG_HOME/WakeMATE Companion/` or `~/.config/WakeMATE Companion/`

Fields: `bind_address` (legacy HTTP address and host used for HTTPS), `tls_port`, `allow_insecure_http`, `discovery_port`, `discovery_message`, `api_token` / `token_storage` (empty + `"keyring"` once migrated to the OS credential store), `device_name`, `launch_on_startup`, `allow_input_commands`, `allow_power_commands`, `allow_remote_connections` (when `false`, both listeners are forced to `127.0.0.1`), `allow_discovery`, `require_auth_for_info`. Sample: [wakemate.config.example.json](./wakemate.config.example.json). The only supported environment variable is `RUST_LOG` (see [.env.example](./.env.example)).

The persistent certificate and private key are stored together as `wakemate.tls.json` in the same app-data folder. A corrupt identity is treated as an error instead of silently generating a new certificate and breaking existing pins.

`GET /v1/info` (authenticated) additionally returns Wake-on-LAN fields the mobile app should save while the PC is online: `mac_address`, `broadcast_address`, `wol_port`, plus `interface_name`, `subnet_mask`, `ping_address`. When the PC later sleeps, the phone sends the magic packet directly using those saved values.

### Building

1. Install Rust via `rustup` (on Windows, use a Visual Studio Developer PowerShell).
2. `cargo run`

Quality gate (format/lint/test/build) before committing:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1
```

Windows release + installer:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\installer\build-installer.ps1
```

The installer prepares a clean-install config with `allow_remote_connections`, `allow_discovery`, and `launch_on_startup` enabled (existing configs are left alone on upgrade) and registers the pre-logon boot task. Code signing is prepared but disabled in [installer/wakemate-companion.iss](./installer/wakemate-companion.iss) until a certificate exists.

Tray icons: `assets/tray-icon.ico` / `assets/tray-icon.png`, with a built-in brand-colored fallback ([src/theme.rs](./src/theme.rs)).

### Releasing

Push a `v*` tag → [.github/workflows/release.yml](./.github/workflows/release.yml) builds the Windows installer (+ SHA-256) and an **unsigned** macOS `.dmg`, then stages a **draft** GitHub release; a maintainer must review and publish manually. Checklist: [docs/RELEASE_CHECKLIST.md](./docs/RELEASE_CHECKLIST.md).

### Documentation

- [docs/SECURITY_MODEL.md](./docs/SECURITY_MODEL.md) — threat model and mitigations
- [docs/MACOS_BUILD.md](./docs/MACOS_BUILD.md) — macOS status, packaging, tray-parity porting plan
- [docs/WINDOWS_BUILD.md](./docs/WINDOWS_BUILD.md) — Windows toolchain notes
- [docs/TROUBLESHOOTING.md](./docs/TROUBLESHOOTING.md) — common issues
- [docs/RELEASE_CHECKLIST.md](./docs/RELEASE_CHECKLIST.md) — pre-ship checklist
- [docs/EULA.txt](./docs/EULA.txt) — installer-visible end-user license agreement
- [docs/PRIVACY_TEMPLATE.md](./docs/PRIVACY_TEMPLATE.md), [docs/THIRD_PARTY_NOTICES_TEMPLATE.md](./docs/THIRD_PARTY_NOTICES_TEMPLATE.md) — legal templates that still contain placeholders
