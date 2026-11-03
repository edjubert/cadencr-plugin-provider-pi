# Pidancr 0.2.0 compatibility qualification

Provider ID: `pidancr`. Display name: **Pidancr**. Publisher: `edjubert`.
Minimum app version: `0.16.0`.

The previous author publication and tag were withdrawn before registry acceptance.
Version `0.2.0` is being qualified again under the `pidancr` identity; no archived
release is retained. [Registry PR #22](https://github.com/merkr-software/cadencr-registry/pull/22)
is in draft during qualification.

## Tooling and contracts

Packaging uses the canonical CLI compiled from CadencR `v0.16.0`, source
`7affa5ec15088db7c6dfe4f743f08fe981ac2b7d` (CLI version `0.15.4`).
Registry validation uses the pinned CLI source
`3ebb603160d9dcf054adbb6315c68cb8702cf7ce` and accepted registry base
`2c6ee24e485c1ff6b80c53c35b333a9b91973b44`.

| Contract | Implementation |
| --- | --- |
| Distinct marketplace identity | `pidancr` / `edjubert`, separate from official `pi` |
| Descriptor, package and ACP identity | `pidancr`, **Pidancr**, version `0.2.0` |
| Native session identity | Canonical workspace-bound `pidancr-v1.` IDs |
| Native runtime discovery | `PI_COMMAND`, with legacy local `CADENCR_PI_COMMAND` fallback |
| ACP lifecycle | ACP v1, durable resume, close and explicit missing-session errors |
| Discovery and live model arguments | Same validated arguments passed to native Pi |
| Packaging and source binding | Allowlisted staging, canonical reproducible archives and source-pinned submission |

## Local checks

| Check | Result |
| --- | --- |
| Formatting, 42 Rust tests and Clippy | Passed |
| Release binary | Built |
| Workflow and script syntax | Passed |
| Canonical descriptor/package validation and packaging twice | Passed; identical archive digests |
| Extracted macOS arm64 archive with actual Pi `0.85.1` and Node `22.19.0` | Passed |

The native fixture uses isolated configuration/session storage and five inert
HTTP model requests on `127.0.0.1`, without personal credentials. It checks the
provider identity, model discovery, ACP negotiation, model selection, missing
empty sessions, streaming/usage, restart/resume, workspace isolation, duplicate
resume rejection, cancellation, close during a prompt and automatic naming.
Assembly requires matching provider ID, archive/executable digests and native
evidence for every declared target.

## Remaining qualification

Native GitHub jobs must pass for macOS arm64/Intel and Linux arm64/x86-64 before
author publication and public-download verification. Linux qualification uses
Ubuntu 24.04/GNU libc; older distributions and Windows are not qualified.
A signed marketplace installation and packaged-app lifecycle have not been
exercised. Registry acceptance, mirroring and signed catalog publication remain
required before marketplace listing.

Pi tools execute without per-tool approval; extension confirm/select dialogs
are bridged. Conversations from connector `0.1.0` using raw native IDs require a
new CadencR conversation; native Pi session files remain available through Pi.
