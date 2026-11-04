# Marketplace release

This repository's marketplace candidate is **0.2.1**, compatible with
**CadencR >=0.16.0**. Its provider ID is `pidancr`, display name is **pi (pidancr)**
and publisher is `edjubert`.
The official registry already owns `pi` under publisher `cadencr`; that identity
cannot be transferred through a normal version submission.

The [pi (pidancr) v0.2.1 release](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/tag/v0.2.1)
is published with a signed tag and GitHub's server-side `immutable: true`.
All eight public assets match the qualified CI bundle byte-for-byte.
[Registry PR #22](https://github.com/merkr-software/cadencr-registry/pull/22)
is the admission request. Marketplace availability requires maintainer
acceptance and signed catalog publication. See [qualification results](QA.md).

The package template is an authoring input. Its empty platform map is filled
only by the packaging script; it is not installable metadata.

## Prepare and qualify

The GitHub workflow builds native binaries on macOS arm64/Intel and Linux
arm64/x86-64. Each runner validates the local descriptor, packages twice with
CadencR's canonical Rust CLI, checks identical digests, and executes the
**extracted archive** against actual Pi `1.5.0` and a local model fixture.
Production dependencies of this external runtime must pass `npm audit`.
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
cancellation, close during an active prompt, automatic naming and independent
confirmations in two simultaneous sessions. It retains its scratch directory
under the system temporary directory.

## Publish and submit

1. Review and commit the source, then push it to `main` to run
   **Validate and prepare marketplace release**.
2. Review the successful run's `marketplace-release` artifact: all archives,
   `package.json`, `submission.json`, `SHA256SUMS`, native QA evidence and notes.
   Confirm that the submission pins the exact tested source commit.
3. Ensure GitHub immutable releases are enabled for the author repository.
   Tag that commit as `v0.2.1` and push the tag. After publication authorization,
   either publish the reviewed bundle with `gh release create --verify-tag --draft`,
   upload all assets and publish the draft with `gh release edit --draft=false`,
   or dispatch the same tag with `publish` checked to qualify and publish a new
   bundle through the workflow. An existing release is refused; assets are not
   overwritten. Confirm GitHub reports `immutable: true` for the published release;
   this locks its assets and tag on the server.
4. Verify every downloaded public asset against the qualified CI bundle and
   the archive hashes against `SHA256SUMS`.
5. In a personal fork of `merkr-software/cadencr-registry`, add:

   ```text
   packages/pidancr-0.2.1.json      ← package.json
   submissions/pidancr-0.2.1.json   ← submission.json
   ```

6. Run `cadencr registry validate --base <accepted-registry> --candidate <fork>`,
   then submit the reviewed contribution as a pull request.

Registry CI, publisher continuity, source ownership and maintainer acceptance
remain required. The registry signs and mirrors accepted packages. An author
release alone does not make the plugin appear in the marketplace.

The workflow prepares artifacts on pushes/PRs; its publication job requires the
explicit dispatch input. A manually published bundle must be the exact reviewed
CI bytes for the matching source tag. The workflow never opens a registry PR,
replies to reviews, merges, or signs the registry catalog.
