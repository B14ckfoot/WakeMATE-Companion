# Third-Party Notices Template

Complete this file before release. This is a placeholder, not the final notice set.

## How To Generate

Recommended workflow:

1. Install a license inventory tool such as `cargo-license`.
2. Generate a dependency report from the locked dependency graph.
3. Review every crate license manually.
4. Copy required notice text for crates that require attribution or bundled license text.
5. Ship the completed notice file with the installer and install directory.

Example commands:

```powershell
cargo install cargo-license
cargo license --json > third-party-licenses.json
```

## Notice Table

| Component | Version | License | Copyright | Notes |
| --- | --- | --- | --- | --- |
| axum | [fill] | [fill] | [fill] | [fill] |
| tokio | [fill] | [fill] | [fill] | [fill] |
| serde | [fill] | [fill] | [fill] | [fill] |
| uuid | [fill] | [fill] | [fill] | [fill] |
| enigo | [fill] | [fill] | [fill] | [fill] |
| network-interface | [fill] | [fill] | [fill] | [fill] |
| tray-icon | [fill] | [fill] | [fill] | [fill] |
| winit | [fill] | [fill] | [fill] | [fill] |
| qrcodegen | [fill] | [fill] | [fill] | [fill] |
| image | [fill] | [fill] | [fill] | [fill] |

## Release Rule

Do not ship a public installer until this file has been replaced with the final reviewed notices.
