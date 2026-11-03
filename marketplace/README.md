# Marketplace release

This repository's first marketplace version is **0.2.0**, compatible with
**CadencR >=0.16.0**. Its provider ID is `pi-rust` and publisher is `edjubert`.
The official registry already owns `pi` under publisher `cadencr`; that identity
cannot be transferred through a normal version submission.

The package template is an authoring input. Its empty platform map is filled
only by the packaging script; it is not installable metadata.

## Prepare and qualify

The GitHub workflow builds native binaries on macOS arm64/Intel and Linux
arm64/x86-64. Each runner validates the local descriptor, packages twice with
CadencR's canonical Rust CLI, checks identical digests, and executes the
**extracted archive** against actual Pi `0.85.1` and a local model fixture.
Only passing targets enter the final `package.json` and `submission.json`.
Windows is not declared.
Linux binaries are built and tested on Ubuntu 24.04 with GNU libc; compatibility
with older distributions is not established.

The CLI is built from CadencR `v0.16.0`, commit
`7affa5ec15088db7c6dfe4f743f08fe981ac2b7d` (CLI version `0.15.4`).
The app and CLI have independent version numbers.

For local macOS arm64 preparation, with Python >=3.12 and this CLI available:

```bash
cargo build --locked --release
python3 scripts/package-release.py package \
  --target darwin-aarch64 \
  --binary target/release/cadencr-plugin-provider-pi \
  --cli /absolute/path/to/cadencr \
  --output /absolute/new/candidate
node scripts/native-qa.mjs \
  --provider /absolute/new/candidate/extracted/bin/provider \
  --pi /absolute/path/to/pi \
  --output /absolute/new/candidate/native-qa.json
```

The QA fixture uses isolated config/session directories and no personal
credentials. It exercises discovery, negotiation, model selection, missing
empty-session handling, streaming, usage, restart/resume, workspace isolation,
cancellation, close during an active prompt and automatic naming. It retains its scratch directory
under the system temporary directory.

## Publish and submit

1. Review and commit the source, then push it and the matching `v0.2.0` tag.
2. Run **Validate and prepare marketplace release** from that tag with `publish`
   unchecked. Review the `marketplace-release` artifact: all archives,
   `package.json`, `submission.json`, `SHA256SUMS`, native QA evidence and notes.
3. Explicitly dispatch the same tag with `publish` checked to create the author
   GitHub release. An existing release is refused; assets are not overwritten.
4. Verify the downloaded public archive hashes against `SHA256SUMS`.
5. In a personal fork of `merkr-software/cadencr-registry`, add:

   ```text
   packages/pi-rust-0.2.0.json      ← package.json
   submissions/pi-rust-0.2.0.json   ← submission.json
   ```

6. Run `cadencr registry validate --base <accepted-registry> --candidate <fork>`,
   then submit the reviewed contribution as a pull request.

Registry CI, publisher continuity, source ownership and maintainer acceptance
remain required. The registry signs and mirrors accepted packages. An author
release alone does not make the plugin appear in the marketplace.

The workflow prepares artifacts on pushes/PRs; publication requires the explicit
dispatch input. It never opens a registry PR, replies to reviews, merges, or signs
the registry catalog.
