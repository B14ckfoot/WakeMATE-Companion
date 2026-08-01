---
paths:
  - ".github/workflows/**/*.yml"
  - "Cargo.toml"
  - "Cargo.lock"
  - "build-release.ps1"
  - "scripts/**/*.ps1"
  - "installer/**/*"
  - "docs/RELEASE_CHECKLIST.md"
  - "docs/EULA.txt"
  - "docs/PRIVACY_TEMPLATE.md"
  - "docs/THIRD_PARTY_NOTICES_TEMPLATE.md"
---

# Release and packaging

- Keep the Cargo package version, Inno Setup version, macOS bundle version, output names, checksums, and release tag expectations consistent.
- Run the full quality gate before packaging. A package job must not substitute for formatting, linting, tests, and release-build validation.
- Preserve reviewable Windows installer and macOS DMG artifacts with deterministic names and SHA-256 checksums. Keep the GitHub release a draft until a maintainer reviews it.
- When either platform pipeline changes, verify the other is still preserved and dependencies/paths remain valid. Do not present the current unsigned macOS preview as production-ready.
- Installer upgrades must preserve user config and pairing state; uninstall cleanup must cover registered tasks, startup entries, and firewall rules while documenting retained data accurately.
- Never commit certificates, private keys, provisioning data, signing passwords, tokens, or machine paths. Do not add placeholder signing that appears enabled; use protected CI secrets only when real signing is authorized.
- Do not tag, trigger a release, publish a draft, or alter signing configuration without explicit user permission.
