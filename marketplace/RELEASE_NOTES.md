## Pi Rust connector 0.2.0

- Prepare native, reproducible provider archives for the CadencR 0.16.0 marketplace.
- Register the independent connector as `pi-rust`, published by `edjubert`.
  The official marketplace's `pi` connector is a separate implementation.
- Negotiate ACP v1 and advertise durable resume and close.
- Bind session identities to their workspace and restore only persisted native
  sessions; refuse missing sessions instead of silently replacing their context.
- Forward the same Pi arguments during model discovery and live sessions.
- Support `PI_COMMAND` in managed installations; retain `CADENCR_PI_COMMAND` as
  a fallback for local installations.

Requires a separately installed Pi CLI (`0.85.1` audited) and Node.js >=22.19.0.
Authenticate using Pi itself. Credentials and the native Pi runtime are not bundled.
Pi tools execute without per-tool approval; extension confirm/select dialogs are
bridged to ACP permissions. Existing raw Pi session IDs from connector 0.1.0
require a new CadencR conversation; their native Pi session files remain available
through Pi.
