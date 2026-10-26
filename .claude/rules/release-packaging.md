---
paths:
  - ".github/workflows/**/*.yml"
  - "Cargo.toml"
  - "Cargo.lock"
  - "build-release.ps1"
  - "scripts/**/*.ps1"
  - "installer/**/*"
  - "docs/RELEASE_CHECKLIST.md"
---

# Release evidence

- Keep Cargo, Windows installer, macOS metadata, artifact names, and release tag versions consistent. Protocol version is a separate explicit wire contract.
- Format/lint/tests/release build and dependency audit precede packaging. Do not suppress errors/findings or treat successful packaging as device/installer QA.
- Verify build/source/executable provenance before packaging. Compatibility switches must never bypass this. Write each package run to a fresh output directory and retain SHA-256 plus provenance alongside it.
- Verify bundled prerequisites, including the Microsoft VC++ redistributable signature. Preserve previous artifacts; an old same-version installer is not evidence of current source.
- Keep installer elevation narrow, no automatic tray launch, program/port/Private/LocalSubnet firewall scope, graceful path-scoped shutdown, and accurate upgrade/uninstall/retained-data behavior.
- Lock or verify workflow dependencies and minimize job token privileges. Keep generated releases draft-only pending maintainer review.
- Never enable placeholder signing or commit signing credentials, keys, certificates, tokens, or machine-specific config. Label unsigned/non-notarized output truthfully.
- Preserve both platform pipelines while retaining macOS preview labeling. Clean-VM/account/session, physical Android/iOS, actual wake/input, and macOS runtime checks remain explicit release boundaries.
- Do not commit/push/tag/publish or change signing configuration unless the user explicitly asks.
