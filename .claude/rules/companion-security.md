---
paths:
  - "src/**/*.rs"
  - "wakemate.config.example.json"
  - "docs/SECURITY_MODEL.md"
  - "docs/PROTOCOL5_MIGRATION.md"
---

# Protocol and security invariants

- Review caller credential, trust channel, endpoint class, selected grant, persistence, replay/expiry, and failure response before changing authorization.
- Protocol 5 has no permanent shared credential/keyring runtime. QR-session tokens enroll/poll only their own session; approved-device tokens cannot enroll phones. Reject legacy activation.
- Authenticated APIs use pinned HTTPS; optional HTTP exposes credential-free public health only. Discovery is untrusted metadata. Keep exact persistent leaf-certificate pinning and reject corrupt TLS identity rather than silently regenerating it.
- Require native desktop approval and per-phone grants. Default status/wake; check media, input, and power independently. Explain that keyboard/mouse gives broad signed-in-account control.
- Persist hashes, permissions, pause, and generation atomically. Failed writes deny access and surface errors; a failed revoke/pause is not proof of durable success. Corrupt storage must preserve evidence and fail closed.
- Invalidate pending approvals and cancel stale input on permission changes, pause, revoke, reset, restart, and shutdown. Check live authorization at execution, not just request admission.
- Keep one bounded input worker: queue 32, text limit 1024 characters, short cancellable chunks, owned held-button lease/cleanup, and checked OS insertion counts. Preserve physical modifiers and truthful partial/failure results.
- Keep constant-time secret/hash comparisons, authentication lockout, validated bodies/commands/addresses/ports, Origin rejection, and non-cacheable API responses. Never log tokens/private keys.
- Test changed parsing, grants, persistence failures, expiry, revocation, pinning, and input cancellation using injected boundaries. Review every affected mobile producer/consumer.
