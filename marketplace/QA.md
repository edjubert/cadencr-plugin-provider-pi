# pi (pidancr) 0.2.1 compatibility qualification

Provider ID: `pidancr`. Display name: **pi (pidancr)**. Publisher: `edjubert`.
Minimum app version: `0.16.0`.

Version `0.2.1` updates the displayed name, native permission routing and
release immutability. Existing `pidancr-v1.` session IDs are retained.
[Registry PR #22](https://github.com/merkr-software/cadencr-registry/pull/22)
remains the admission request for this new provider identity.

## Release qualification

The release workflow must qualify all four native targets before publication.
Its `marketplace-release` artifact binds each archive/executable hash, provider
ID, displayed name and Pi `1.5.0` native evidence to the exact source commit.
Publication requires a signed `v0.2.1` tag, the reviewed CI bytes, a successful
public-download verification and GitHub's server-side `immutable: true`.
The registry package and submission must be byte-for-byte copies of those assets.

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

Both checks in the [registry workflow for the previous candidate](https://github.com/merkr-software/cadencr-registry/actions/runs/41732178470)
passed on **2026-11-04**. The revised `0.2.1` contribution requires checks and
maintainer review against its new exact head. Linux qualification
uses Ubuntu 24.04/GNU libc; older distributions and Windows are not qualified.
A signed marketplace installation and packaged-app lifecycle have not been
exercised. Registry acceptance, mirroring and signed catalog publication remain
required before marketplace listing.

Pi tools execute without per-tool approval; extension confirm/select dialogs
are bridged. Conversations from connector `0.1.0` using raw native IDs require a
new CadencR conversation; native Pi session files remain available through Pi.
