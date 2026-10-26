# WakeMATE security model — protocol 5

WakeMATE is a local-network companion for a signed-in desktop account. Network reachability never grants authorization. Windows is the production development target; platform runtime and installer claims still require real-device and clean-VM validation.

## Trust boundaries

| Boundary | Enforcement | Practical limit |
| --- | --- | --- |
| Network exposure | Local-only defaults; interactive onboarding consent; private adapter selection; program/port-scoped Private/LocalSubnet installer firewall rules | A trusted LAN can still contain hostile devices. Do not expose the API through public router forwarding. |
| Computer identity | Persistent self-signed certificate; exact leaf SHA-256 pin obtained through the visual QR channel; native Android/iOS pin verification | Discovery replies, IP addresses, names, and MAC addresses are hints, not authenticated identity. |
| Enrollment | Temporary, single-use QR session; explicit native desktop approval; selected per-phone permissions | A phone name and source IP are not cryptographically verified human identity. Approve only an expected request. |
| Approved operations | Per-device bearer token hash, capability checks, pause/revocation state, and live checks before input execution | A stolen phone credential has its saved permissions until revoked. |
| Desktop privilege | Normal-user process; elevated interactive runtime refused; retired pre-login mode does not bind listeners | Keyboard/mouse access gives broad signed-in-account control, even without the power grant. |
| Browser requests | Origin-bearing API requests rejected; authenticated routes require the native token header | Native malware and software running as the same OS user are outside the browser boundary. |
| Persistence | Versioned atomic registry snapshots for hashes, grants, pause, and generation; flushed sibling-file replacement; surfaced failures | A failed disk write cannot establish a durable grant, revoke, or pause. Repair/reset before relying on persistence. |

## Credentials and transport

There is **no permanent shared pairing credential** in protocol 5. Configuration no longer generates or loads `api_token` or `token_storage`, and the runtime no longer accesses an OS keyring. Old OS entries are retained but inert. Only approved-device token hashes live on the companion; phones store their own tokens and pins in SecureStore.

QR sessions expire after ten minutes and can enroll only once. The desktop approval deadline is two minutes. New invitations, cancellation, reset, shutdown, and invalidated sessions cannot approve stale callbacks. A device credential cannot enroll another phone. Desktop approval is reported only after the registry commit succeeds.

Every authenticated mobile request requires pinned HTTPS. Missing or corrupt pins require re-pairing; inaccessible keychain storage requires unlocking/reopening the phone app. Neither condition selects HTTP. The saved certificate identity follows an IP change, and a mismatch has no downgrade/retry path. Redirects are refused by the native pinned transport.

`allow_insecure_http` defaults to false, including when omitted from an older file. An explicit true enables only `GET /v1/health` without credentials; enrollment and control are absent from that listener. The current mobile app permits the same narrow public-health exception. Protocol 5 rejects legacy shared-token activation.

Token hashes are compared in constant time. Authentication failures are rate-limited across authenticated endpoints. Responses disable caching and MIME sniffing. Request bodies, command variants, ports, addresses, wake destinations, and input ranges are validated before use.

## Permissions and account trust

The least-privilege preset grants status and wake. Media, keyboard/mouse, and power are separate capability checks. The native dialog offers Status and wake only, Media remote, Desktop control, and Desktop and power presets. Existing approvals without a grant record default to status/wake; the desktop must authorize more.

A keyboard/mouse grant is broad control of the signed-in account: it can interact with applications and invoke actions beyond WakeMATE's dedicated power commands. Per-command authorization is not an account sandbox. Do not grant desktop access to a phone you would not trust to operate that account.

Paired Devices supports changing grants and revoking one phone. Pause denies approved-phone operations without forgetting grants. Revoke All Phones and reset invalidate pending pairing and cancel input as well as changing durable device state. Storage errors pause authorization and are visible; unsuccessful changes must never be displayed as durable success.

## Input and OS boundaries

A single owned input worker receives validated commands through a queue bounded at 32. It rechecks live authorization and cancellation generations before execution and between short text chunks. Text is limited to 1024 Unicode characters; chunks contain at most 16 characters. Revoke, pause, disconnect/restart, and shutdown cancel pending work.

Held mouse buttons belong to their requesting phone and expire after a two-second lease. Cancellation/watchdog cleanup releases owned synthetic state. Windows `SendInput` return counts are checked, including partial insertion. Cleanup is best effort when Windows refuses input; already emitted input cannot be rolled back. Physical modifiers and the secure desktop must remain protected.

WakeMATE does not bypass UAC, OS access controls, or Secure Attention Sequence policy. Ctrl+Alt+Delete is refused as a keystroke. The structured security-screen command reports unsupported, permission-required, execution-failed, or an explicitly requested safe fallback truthfully.

## Storage, recovery, and deployment limits

Config migration strips retired shared-secret/global-capability fields while preserving supported preferences. `onboarding_completed` defaults false and gates LAN binds/discovery. A selected `preferred_address` must be a private IPv4 literal; invalid values fail without overwriting the file or widening the bind. A stale valid address causes a visible bind failure.

Corrupt config is backed up and recovered local-only. Unreadable files fail explicitly. Corrupt device storage preserves evidence and pauses access. Persistent TLS identity corruption fails rather than silently creating a new certificate. Windows files rely on the user's app-data ACLs; a process with the same user's access can read the TLS private key or change local state.

There is no automatic updater. Packaging records source/executable hashes and preserves old output separately. Current builds are unsigned unless a real signing process is configured and verified; checksums are integrity checks, not publisher authentication. macOS remains an unsigned, non-notarized headless preview without interactive approval. See the [release checklist](RELEASE_CHECKLIST.md) and [migration guide](PROTOCOL5_MIGRATION.md).
