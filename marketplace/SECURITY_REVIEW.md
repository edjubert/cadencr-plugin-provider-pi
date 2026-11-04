# Author security review — pi (pidancr) 0.2.1

This report documents author-side checks performed on **2026-11-04**.
Registry maintainer approval and explicit admission authorization remain
independent requirements; this report is not a security certification.

## Execution and authentication boundary

| Surface | Reviewed behavior |
| --- | --- |
| Native executable | Rust connector implements `version`, `models` and stdio ACP v1; no connector HTTP listener or remote installer |
| Pi runtime | Installed separately by the user; Pi `1.5.0` is qualified with Node `22.19.0` |
| Process launch | `std::process::Command` with separate arguments, without a shell command string |
| Executable selection | User-controlled `PI_COMMAND`, legacy local fallback or `pi` on `PATH`; users must trust the selected executable |
| Credentials | Native Pi manages authentication; archives contain no Pi runtime, user settings, session data or credentials |
| Privileges | Pi inherits user privileges and its normal project trust policy; native tools execute without per-tool approval |
| Extension UI | Confirm/select dialogs use ACP permissions; unsupported input/editor dialogs are cancelled |
| Environment | The connector preserves the environment provided by the host; it does not provide process or network isolation |

## Source controls and native regression checks

- Forwarded Pi arguments cannot replace connector-owned transport, trust or
  session flags, including `--mode`, `--session-id` and their `=value` forms.
- Session restoration accepts canonical workspace-bound IDs containing a
  validated native UUID. Workspaces are absolute and canonicalized. Cross-workspace
  restoration and missing native sessions fail explicitly.
- Permission requests use process-wide identifiers. The native fixture asks
  for two simultaneous confirmation decisions, answers them in reverse order
  with opposite choices, and verifies that each Pi session receives its own choice.
- Model discovery and automatic naming use ephemeral sessions.
  Automatic naming disables native tools, extensions, skills, templates, themes
  and context files.
- RPC requests and extension decisions have timeouts; cancellation, shutdown
  and close during an active prompt are exercised by native qualification.
- ACP and Pi are local process trust boundaries. This connector does not claim
  to sandbox a compromised Pi runtime, malicious extensions or an untrusted host.

## Dependency audit provenance

| Audit | Input | Result |
| --- | --- | --- |
| `cargo-audit 0.22.2` | Committed `Cargo.lock`, 157 dependencies; RustSec DB commit `965bf9ad083b58e6340d65829d85217b18be9290`, updated 2026-11-03 | 0 known vulnerabilities, no informational warnings, no ignored advisories |
| `npm audit --omit=dev` | Fresh install of external `@earendil-works/pi-coding-agent@1.5.0` | 0 reported vulnerabilities |

Native Pi dependency audit is also required on every release qualification
runner. Results describe the advisory databases and resolved dependencies at
audit time. Pi is not bundled in the provider archives; future native runtime
updates and advisory checks are the user's responsibility.

## Artifact integrity and publication

- Build and qualification use the committed Rust lockfile and exact CadencR
  `v0.16.0` source for canonical validation and packaging.
- Staging copies only `bin/provider`, `README.md`, `LICENSE`,
  `THIRD_PARTY_NOTICES.md` and `icon.svg`; official Pi logo artwork is excluded.
- Each platform archive is packaged twice with equal SHA-256 digests, then the
  extracted executable runs the native fixture with isolated storage and inert
  loopback model responses.
- Final assembly requires matching archive/executable hashes, target, version,
  provider ID, displayed name and successful native evidence.
- The submission pins a full source commit and an exact signed release tag.
  Public downloads are compared byte-for-byte with the reviewed CI artifact.
- GitHub immutable releases are enabled for future author publications.
  All assets are uploaded to a draft before publication; the final release must
  report `immutable: true`, protecting its assets and tag against replacement.

The registry still owns acceptance, archive mirroring, catalog signing and
publication. A passing author audit or CI run does not grant that authority.
