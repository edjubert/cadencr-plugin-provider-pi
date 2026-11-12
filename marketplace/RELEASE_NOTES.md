## Pi 0.6.1

- Use the official Pi artwork (`pi-logo.svg`) as the marketplace provider icon,
  matching local installations. The previous opaque connector illustration
  appeared as a disk in CadencR's monochrome conversation badges.
- Include the official logo in every platform archive while retaining
  `icon.svg` for the README illustration.
- Reject packaging when the local descriptor and marketplace icon paths
  differ.

## Pi 0.6.0

- Declare a `mode` selector (ACP category `mode`) in the `models` output and
  in every live session, with each choice annotated
  `_meta.cadencr.permissionMode` (`ask` → `ask`, `accept-edits` →
  `acceptEdits`, `smart-approve` → `auto`, `auto-approve` →
  `bypassPermissions`), mirroring the Mistral Vibe connector. CadencR maps
  its permission-mode chip onto these values and applies the selected mode
  to the live session through `session/set_config_option`.
- In `ask` mode, extension `confirm`/`select` dialogs are bridged to ACP
  `session/request_permission` as before. In every other mode the connector
  answers them itself (confirm → yes, select → first option), so
  auto-approve no longer prompts on every dialog.

## Pi 0.5.0

- Register the connector under the `pi` provider ID with display name **Pi**
  in marketplace metadata, local descriptors and ACP initialization. The
  registry handed the `pi` namespace over to this connector after retiring
  its first-party `pi` plugin (registry PR #22). Publisher remains
  `edjubert`, and workspace-bound `pidancr-v1.` session identities are
  unchanged.
- Publish new versioned archives through GitHub's immutable-release protection.
  Publication checks the server-side `immutable` flag after all draft assets
  have been uploaded and the release has been published.
- Include the displayed name in every native qualification record and require
  it to match the package metadata during final assembly.
- Use process-wide identifiers for extension confirmation requests and qualify
  independent confirmation responses in simultaneous sessions with native Pi.
- Recommend and qualify native Pi `1.5.0`; its production dependencies must pass
  `npm audit` on every release runner.

Compatible with CadencR >=0.16.0. Requires a separately installed Pi CLI
(`1.5.0` audited) and Node.js >=22.19.0. Authenticate using Pi itself.
Credentials and the native Pi runtime are not bundled.
Pi tools execute without per-tool approval; extension confirm/select dialogs
are bridged to ACP permissions. Existing raw Pi session IDs from connector
0.1.0 require a new CadencR conversation; their native Pi session files remain
available through Pi. Sessions from `pidancr 0.2.0` retain their identities.
