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
