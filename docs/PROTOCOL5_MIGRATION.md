# Protocol 5 migration

Protocol 5 separates temporary QR enrollment from approved-device access. The
companion no longer generates, loads, stores, or authenticates a permanent
shared pairing token. Each approved phone has its own credential and only the
permissions selected on the desktop.

## Upgrade and first launch

1. Preserve a backup of the previous installation's data before upgrading.
2. Start the companion as the normal desktop user. Complete the interactive
   network setup before pairing a phone.
3. Choose the intended private adapter and enable local network access. A
   selected adapter address stays selected; an unavailable address causes a
   visible failure instead of expanding access to another interface.
4. Open a fresh QR code in the companion, scan it in the current mobile app,
   and approve the phone with the intended permissions. Approval for status
   and wake access does not imply permission for media, input, or power actions.

Existing computer names, port choices, startup preferences, and explicit network
preferences remain in the configuration. The new `onboarding_completed` field
defaults to `false`, including for older files. Until interactive setup records
consent, API listeners remain on loopback and discovery is disabled. Installer
preparation cannot grant network consent.

An optional `preferred_address` must contain a literal private IPv4 address.
Invalid values stop loading with an actionable configuration error and leave the
file unchanged. They are never silently replaced by a wildcard address.

## Retired configuration and credentials

Legacy JSON fields `api_token`, `token_storage`, `allow_input_commands`,
`allow_power_commands`, and `require_auth_for_info` are accepted as unknown
fields and omitted when the configuration is saved. Global capability switches
cannot grant a phone permission under protocol 5.

The removed credential-store backend never reads or writes an old OS keyring
entry. A retained entry from an older version is harmless to the new protocol;
this migration does not delete unrelated OS credentials. Reset clears network
consent and preferences; approved-device revocation is handled separately by the
companion's device registry transaction.

The persistent TLS identity is preserved. Corrupt configuration is backed up
before local-only recovery, while unreadable files and invalid adapter choices
fail explicitly. Atomic sibling-file writes remain in place.

## Mobile transport

The mobile app requires a certificate pin obtained through the visual QR channel
for every authenticated request. Missing or malformed pins require a fresh QR
scan. Locked or unavailable secure storage requires unlocking/reopening the app;
it never authorizes an HTTP fallback. IP changes retain the computer's original
certificate identity, and discovery cannot replace that identity.

The only plaintext HTTP exception is a public health probe without credentials.
An omitted `allow_insecure_http` setting now defaults to `false`. An explicit
`true` preserves only the optional credential-free health listener. Older
HTTP-only pairings therefore need an updated companion and a new QR scan.
Legacy shared-token activation is rejected by protocol 5; the mobile app can use
the older activation flow only over pinned HTTPS after the companion explicitly
reports a protocol below 5.

Direct Wake-on-LAN from the phone continues to work while the computer's
companion is offline. Remote status, input, media, power, and relay operations
require the running normal-user companion and the appropriate approved grant.

## Validation boundary

Automated Rust and mobile tests verify the protocol and storage behavior. A
release still requires physical Android/iOS verification of certificate
pinning, address changes, pairing suspension/resume, and selected permissions,
plus Windows installer/upgrade/uninstall and user-session checks. Existing
installer files in `dist` are not evidence that the current source was packaged.
