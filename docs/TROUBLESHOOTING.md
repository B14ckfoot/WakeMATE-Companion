# Troubleshooting

## Where to look first

- **Logs**: WakeMATE logs to stdout/stderr via `tracing`. The tray app doesn't keep a console window open, so redirect it if you need to capture logs interactively:
  `wakemate-companion.exe > wakemate.log 2>&1` (or set `RUST_LOG=debug` first for more detail; see `.env.example`).
- **Config / data folder**: use the tray menu's "Open Data Folder", or go directly to `%APPDATA%\WakeMATE Companion\` on Windows (`~/Library/Application Support/WakeMATE Companion/` on macOS). `wakemate.config.json` there is safe to inspect -- the pairing token itself is not stored in it once migrated to the OS credential store (see below).

## "Unauthorized" / 401 from the mobile app

- Confirm the phone has the current token. Rotating the token (tray menu) invalidates every previously paired phone at once.
- Check `x-wakemate-token` is being sent exactly as configured; comparison is exact and case-sensitive.

## "Too many failed attempts" / 429 responses

WakeMATE locks out a source IP for 60 seconds after 8 failed authentication attempts against any authenticated endpoint within a 60-second window (see `src/security.rs`). This is expected behavior if:

- A phone is retrying with a stale/incorrect token in a loop.
- Something on your network is scanning/probing the API.

Wait 60 seconds, fix the token, and try again. There is no way to clear the lockout early short of restarting the Companion (the lockout state is in-memory and per-process).

## The mobile app says "Success" but remote control still doesn't work

This is a known, intentional consequence of the pairing-confirmation hardening (see `docs/SECURITY_MODEL.md`): `/v1/pairing/activate` responds immediately (so the phone doesn't hit its own request timeout) with `status: "pending_approval"`, but `allow_input_commands`/`allow_power_commands` only actually flip to `true` after someone clicks "Yes" on the native desktop confirmation dialog. If that dialog never appeared:

- Confirm the WakeMATE tray app is actually running on the target desktop (not just installed). No dialog is possible without it.
- Confirm you're not hitting the pre-logon headless service instead of the tray-hosted server -- the headless instance refuses pairing activation outright with a distinct error message ("no one is signed in on this computer right now...").
- Check for a dialog that appeared behind another window; it's always-on-top and foreground-activated, but alt-tab if you don't see it.

## "Windows Credential Manager" / token storage questions

- WakeMATE stores the pairing token as a Generic Credential under Windows Credential Manager, target name `pairing-token.WakeMATE Companion`. You can inspect (but not read the secret value of) it via Control Panel -> Credential Manager -> Windows Credentials, or `cmdkey /list`.
- If the credential store is unavailable for any reason (locked-down environment, corporate policy, etc.), WakeMATE falls back to storing the token in the plaintext config file and sets `"token_storage": "file"` there so this is never silent. Check the logs for a `OS credential store unavailable` warning.
- Deleting the Credential Manager entry externally (or profile corruption) causes WakeMATE to mint a brand-new token on next start, silently invalidating any paired phones -- this is intentional fail-safe behavior (never running with an empty/no token) but will require re-pairing.

## Port already in use / server won't start

WakeMATE binds `7777` by default (configurable via `bind_address`). If another process holds that port, the HTTP listener will fail to bind and the tray icon will show an error state. The tray app also detects if *another WakeMATE instance* already has the port open and simply reuses it rather than erroring (see `server_is_reachable` in `src/tray.rs`) -- combined with the single-instance lock, a second launch of WakeMATE should exit quietly rather than fight over the port.

## Uninstalling and removing local data

The Windows uninstaller removes the installed program files, Start Menu shortcuts, the pre-logon scheduled task, and the startup Run-key registration. It deliberately does **not** delete `%APPDATA%\WakeMATE Companion\` or the Windows Credential Manager entry, so a reinstall doesn't force re-pairing.

To fully wipe local WakeMATE data (no cloud account is affected either way):

1. Run "Reset Companion..." from the tray before uninstalling (clears the stored token and all local settings), **or**
2. After uninstalling, manually delete `%APPDATA%\WakeMATE Companion\` and remove the `pairing-token.WakeMATE Companion` entry from Credential Manager.

## macOS / Linux

See `docs/MACOS_BUILD.md` for the current state (headless service only, no tray/pairing UI yet) before troubleshooting a macOS build -- most "nothing shows up" reports there are expected given today's scope.
