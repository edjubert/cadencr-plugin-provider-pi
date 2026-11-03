# CadencR 0.16.0 compatibility qualification

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

## Qualified artifact and remaining release gates

The locally tested `provider-pi-rust-0.2.0-darwin-aarch64.tar.gz` has SHA-256:

```text
4f6c568da741efcbde6c48598f76a81b6202c4a29acc3c7e9e508c55dafde5df
```

This records the local candidate bytes. CI rebuilds and computes its own digests;
native compilation across toolchains/runner paths is not asserted to produce
this same executable. Archive reproducibility is checked for identical staged
bytes with the same canonical packer.

- macOS Intel, Linux x86-64 and Linux arm64 qualification remains pending the
  native GitHub workflow jobs; no local result certifies those targets.
- A signed marketplace installation and packaged-app lifecycle were not exercised.
  The installed desktop was still `0.15.1`; qualification here used the exact
  `0.16.0` source contracts and canonical CLI, plus direct native protocol QA.
- Registry acceptance, author release publication, public-download verification
  and managed-catalog signing remain separate steps.
- Raw ACP session IDs from `0.1.0` require a new CadencR conversation. Existing
  native Pi session files remain available through Pi.
- Pi tools still execute without per-tool approval. Extension confirm/select
  dialogs are bridged; this release does not add a Pi approval extension.

See [release procedure](README.md). No GitHub release or registry PR was created.
