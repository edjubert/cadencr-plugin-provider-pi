# CadencR 0.16.0 compatibility qualification

## Published release qualification

The author release was published and verified on **2026-11-03**:

- [Pi Rust connector v0.2.0](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/tag/v0.2.0).
- Source commit: `f662f6cbbca81d84171d93a893012f77beaad6b4`.
- Signed tag: `v0.2.0`, verified locally and confirmed to resolve to that commit.
- [Passing native CI](https://github.com/edjubert/cadencr-plugin-provider-pi/actions/runs/41688435007).
- [Registry contribution PR #22](https://github.com/merkr-software/cadencr-registry/pull/22).

| Platform | Rust checks | Canonical reproducible packaging | Extracted archive with actual Pi 0.85.1 |
| --- | --- | --- | --- |
| macOS arm64 | Passed | Passed | Passed |
| macOS Intel | Passed | Passed | Passed |
| Linux arm64, Ubuntu 24.04 | Passed | Passed | Passed |
| Linux x86-64, Ubuntu 24.04 | Passed | Passed | Passed |

Each native runner passed formatting, 42 Rust tests and Clippy. Final assembly
bound the archive and executable digests to the native QA evidence and exact
source commit. All eight public assets were downloaded without authentication
and matched the reviewed CI bytes and GitHub asset digests. The four archives
also matched the package metadata and published
[SHA256SUMS](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/download/v0.2.0/SHA256SUMS).
The [native evidence](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/download/v0.2.0/native-qa.json)
records the executable digest and protocol checks for each target.

Final local registry validation used base
`2c6ee24e485c1ff6b80c53c35b333a9b91973b44` and a macOS build of CLI `0.15.4`
from the registry's pinned source `3ebb603160d9dcf054adbb6315c68cb8702cf7ce`.
Metadata, submission and complete base/candidate validation passed. The PR
contains byte-for-byte copies of the public package and submission assets.

The [registry workflow](https://github.com/merkr-software/cadencr-registry/actions/runs/41724562124)
currently reports `action_required`, with no jobs executed, pending maintainer
intervention. Registry acceptance, mirroring and signed catalog publication
remain required before marketplace listing. A signed marketplace installation
and packaged-app lifecycle have not been exercised. Older Linux distributions
and Windows are not qualified.

## Initial local qualification

The following sections preserve the initial checks before source publication;
their pending release steps were subsequently completed as recorded above.

Checked on **2026-11-03** against CadencR release `v0.16.0`, source commit
`7affa5ec15088db7c6dfe4f743f08fe981ac2b7d`. The canonical CLI was compiled
from that exact source (CLI version `0.15.4`).

Registry contribution validation used the official
`merkr-software/cadencr-registry` base
`e31b07eea3d17cc501f8e8b607784ebc1c6c98b0`.

## Requirements and corrections

| Requirement | Result |
| --- | --- |
| Distinct provider identity and publisher continuity | Use `pi-rust` / `edjubert`; official `pi` is already owned by `cadencr` |
| Managed package metadata, separate from a local descriptor | Added package template, checksummed archives and source-pinned submission assembly |
| Exact agreement of binary, descriptor and ACP implementation version | `0.2.0`; checked by preparation and native QA |
| ACP v1 negotiation and capability projection | Fixed negotiation; advertise implemented resume and close |
| Native runtime resolution under the managed environment policy | Added `PI_COMMAND`; the host removes inherited `CADENCR_*` variables |
| Discovery and live model selection agree | Forward the same validated native arguments to both commands |
| Durable, workspace-local restore | Canonical workspace-bound IDs and exact native UUID lookup |
| Empty session in the prompt-free conformance probe | Explicit ACP session-not-found error; the host records restore as unprobed |
| Safe release files and reproducibility | Dedicated allowlisted staging; canonical Rust packer; two identical archive digests |
| Source/release/checksum binding | Final assembly requires clean exact HEAD, matching native evidence and canonical registry validation |

## Executed checks

| Check | Result |
| --- | --- |
| `cargo test --locked` | 42 passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo fmt --check`, `git diff --check` | Passed |
| Release binary, Rust `1.97.1` | Built |
| Local staged descriptor via canonical `plugin validate` | Passed |
| Canonical `pack-provider`, twice | Identical archives |
| Canonical `validate-metadata` | Passed |
| Native QA of the extracted macOS arm64 archive | Passed with actual Pi `0.85.1` and Node `22.19.0` |
| Canonical `validate-submission` | Passed using a clean local source snapshot |
| Canonical `registry validate --base … --candidate …` | Passed against the official registry base above |
| Submission from the original dirty source worktree | Correctly refused before writing output |
| GitHub workflow, `actionlint 1.7.12` | Passed |

The native test used five inert HTTP model requests on `127.0.0.1`, isolated
Pi configuration/session storage and no personal credentials or external model.
It verified discovery, ACP negotiation, model selection, missing empty sessions,
text streaming, context usage, context preservation after connector restart,
workspace isolation, duplicate restore rejection, cancellation, close while a
prompt was running, and automatic naming.

The assembly test committed a **temporary local source snapshot** to exercise
the clean-HEAD requirement. Its submission is a validation fixture, not
published provenance. The original working branch was neither committed,
pushed nor tagged by these checks.

## Initial candidate and release gates

The locally tested `provider-pi-rust-0.2.0-darwin-aarch64.tar.gz` has SHA-256:

```text
4f6c568da741efcbde6c48598f76a81b6202c4a29acc3c7e9e508c55dafde5df
```

This records the local candidate bytes. CI rebuilds and computes its own digests;
native compilation across toolchains/runner paths is not asserted to produce
this same executable. Archive reproducibility is checked for identical staged
bytes with the same canonical packer.

- At this initial stage, macOS Intel, Linux x86-64 and Linux arm64 qualification
  was pending the native GitHub workflow jobs. All four targets later passed.
- A signed marketplace installation and packaged-app lifecycle were not exercised.
  The installed desktop was still `0.15.1`; qualification here used the exact
  `0.16.0` source contracts and canonical CLI, plus direct native protocol QA.
- Author release publication and public-download verification were completed
  after these local checks. Registry acceptance and managed-catalog signing
  remain separate steps.
- Raw ACP session IDs from `0.1.0` require a new CadencR conversation. Existing
  native Pi session files remain available through Pi.
- Pi tools still execute without per-tool approval. Extension confirm/select
  dialogs are bridged; this release does not add a Pi approval extension.

See [release procedure](README.md). The initial local checks did not publish
externally; the final release and PR are linked above.
