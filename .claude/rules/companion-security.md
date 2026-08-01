---
paths:
  - "src/app.rs"
  - "src/config.rs"
  - "src/credential_store.rs"
  - "src/devices.rs"
  - "src/discovery.rs"
  - "src/input.rs"
  - "src/main.rs"
  - "src/pairing.rs"
  - "src/secure_attention.rs"
  - "src/security.rs"
  - "src/system.rs"
  - "src/tls.rs"
  - "src/types.rs"
  - "wakemate.config.example.json"
  - "docs/SECURITY_MODEL.md"
---

# Companion security

- Start protocol or authorization work with a threat-boundary review: identify the caller credential, trust channel, endpoint class, runtime privilege, capability gate, replay behavior, rate limit, and failure response.
- Keep public health, authenticated info/wake, pairing enrollment/status, input, power, and pre-login routes deliberately separate. Network reachability is never authorization.
- New pairing requires an interactive desktop approval. Per-device tokens authenticate only approved devices; they must not mint other device tokens. Preserve revocation and shared-token rotation semantics.
- Compare sensitive tokens in constant time and retain per-IP failure tracking/lockout. Do not log raw tokens or certificate keys.
- Store enrollment secrets in the OS credential store when available and report fallback honestly. Store only per-device token hashes on disk. Preserve safe write-before-delete and atomic migration ordering.
- Preserve the persistent TLS identity and lowercase SHA-256 leaf fingerprint. Reject corrupt identity; never silently rotate it or downgrade a pinned client to HTTP.
- Treat config, JSON bodies, query fields, discovery probes, MAC/IP/port values, command enums, and device metadata as untrusted. Reject unknown or malformed commands instead of guessing.
- Require approved capability checks before input or power execution. Pre-login mode must refuse pairing and input; do not widen elevated capabilities casually.
- Never fake Ctrl+Alt+Delete. Keep Secure Attention Sequence and other unsupported/permission-gated actions structured and truthful, including fallback reporting.
- Add/update tests for changed authentication, pairing, revocation, parsing, status, command authorization, config migration, rate limiting, or certificate behavior, and review the mobile contract in parallel.
