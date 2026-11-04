# pi (pidancr) 0.2.1 compatibility qualification

Provider ID: `pidancr`. Display name: **pi (pidancr)**. Publisher: `edjubert`.
Minimum app version: `0.16.0`.

Version `0.2.1` updates the displayed name, native permission routing and
release immutability. Existing `pidancr-v1.` session IDs are retained.
[Registry PR #22](https://github.com/merkr-software/cadencr-registry/pull/22)
remains the admission request for this new provider identity.

## Release qualification

- Qualified source: `5a28d3feda63ac8ed327d3696a7f48e21f5639d3`.
- Signed annotated tag `v0.2.1` verified locally and by GitHub, resolving to that source.
- [Passing author CI and assembly](https://github.com/edjubert/cadencr-plugin-provider-pi/actions/runs/41814529310).
- [Published release](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/tag/v0.2.1):
  **2026-11-04**, GitHub reports **`immutable: true`**.
- Verified `marketplace-release` artifact `13314481859`, ZIP SHA-256
  `ff718cd3c33f12c5baf99196ccb7a6968c8c12fb015d49660ab0fffcbfa35501`.

| Native platform | Rust checks | Reproducible packaging | Pi 1.5.0 native QA and dependency audit |
| --- | --- | --- | --- |
| macOS arm64 | Passed | Passed | Passed |
| macOS Intel | Passed | Passed | Passed |
| Linux arm64, Ubuntu 24.04 | Passed | Passed | Passed |
| Linux x86-64, Ubuntu 24.04 | Passed | Passed | Passed |

All eight public assets were downloaded without authentication at
`2026-11-04T10:41:35.972619+00:00` and matched the qualified CI bytes, GitHub
asset digests, archive checksums and executable hashes. Archive documentation,
license, notices and icon match the declared source's Git blobs.
The [SHA256SUMS](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/download/v0.2.1/SHA256SUMS)
and [native evidence](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/download/v0.2.1/native-qa.json)
are published with the release.

[Registry contribution commit `b9d577c`](https://github.com/edjubert/cadencr-registry/commit/b9d577c56ee032f8b7b6ee3dbe2c39c13ba6fb84)
contains byte-for-byte copies of the publicly downloaded `package.json` and
`submission.json`. Local metadata, submission and complete base/candidate
validation passed with the pinned registry CLI below.

## Tooling and contracts

Packaging uses the canonical CLI compiled from CadencR `v0.16.0`, source
`7affa5ec15088db7c6dfe4f743f08fe981ac2b7d` (CLI version `0.15.4`).
Registry validation uses the pinned CLI source
`3ebb603160d9dcf054adbb6315c68cb8702cf7ce` and accepted registry base
`2c6ee24e485c1ff6b80c53c35b333a9b91973b44`.

| Contract | Implementation |
| --- | --- |
| Distinct marketplace identity | `pidancr` / `edjubert`, separate from official `pi` |
| Descriptor, package and ACP identity | `pidancr`, **pi (pidancr)**, version `0.2.1` |
| Native session identity | Canonical workspace-bound `pidancr-v1.` IDs |
| Native runtime discovery | `PI_COMMAND`, with legacy local `CADENCR_PI_COMMAND` fallback |
| ACP lifecycle | ACP v1, durable resume, close and explicit missing-session errors |
| Discovery and live model arguments | Same validated arguments passed to native Pi |
| Extension permissions | Unique process-wide request IDs for independently answered concurrent sessions |
| Packaging and source binding | Allowlisted staging, canonical reproducible archives and source-pinned submission |

## Local checks

| Check | Result |
| --- | --- |
| Formatting, 42 Rust tests and Clippy | Passed |
| Release binary | Built |
| Workflow and script syntax | Passed |
| Canonical descriptor/package validation and packaging twice | Passed; identical archive digests |
| Extracted macOS arm64 archive with actual Pi `1.5.0` and Node `22.19.0` | Passed |
| Concurrent extension confirmations | Reproduced the collision before the fix; opposite responses reach their respective sessions after it |
| RustSec audit of `Cargo.lock` | No known vulnerabilities or informational warnings |
| Pi `1.5.0` production dependency audit | No reported vulnerabilities |

The native fixture uses isolated configuration/session storage and seven inert
HTTP model requests on `127.0.0.1`, without personal credentials. It checks the
provider identity, model discovery, ACP negotiation, model selection, missing
empty sessions, streaming/usage, restart/resume, workspace isolation, duplicate
resume rejection, cancellation, close during a prompt, automatic naming and
two concurrent extension confirmation dialogs with opposite responses.
Assembly requires matching provider ID/name, archive/executable digests and native
evidence for every declared target.
See the [author security review](SECURITY_REVIEW.md) for scope and audit provenance.

## Registry acceptance and limits

The `0.2.1` contribution is pushed to the registry fork, and PR #22's title and
description reference this release. During verification on **2026-11-04**,
GitHub's ongoing PR incident still reports the previous head in PR #22,
delaying its new workflow. The previous candidate's green checks do not
qualify contribution `b9d577c`; trusted registry checks and maintainer review
remain required.

Linux qualification uses Ubuntu 24.04/GNU libc; older distributions and Windows
are not qualified.
A signed marketplace installation and packaged-app lifecycle have not been
exercised. Registry acceptance, mirroring and signed catalog publication remain
required before marketplace listing.

Pi tools execute without per-tool approval; extension confirm/select dialogs
are bridged. Conversations from connector `0.1.0` using raw native IDs require a
new CadencR conversation; native Pi session files remain available through Pi.
