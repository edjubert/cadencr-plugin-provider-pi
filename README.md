# Pidancr

<p align="center">
  <img src="icon.svg" alt="Cadencr + Pi" width="256" height="256">
</p>

> The 12 dots around the logo encode the digits of π (`3.14159265358`).
> Read clockwise from the green dot at the top: larger dots represent larger digits.

Cadencr code-backed provider connector for the [Pi coding agent](https://github.com/earendil-works/pi).

The connector implements the Cadencr provider executable contract:

| Command | Behavior |
| --- | --- |
| `version` | Prints the connector version. |
| `models --format acp-config-options-v1 --cwd <abs> [Pi args]` | Launches `pi --mode rpc --no-session --no-approve` in the workspace, calls `get_available_models` and `get_state`, and prints the ACP v1 `SessionConfigOption` array. |
| `run --protocol acp-v1` | Starts an ACP v1 JSON-RPC server over stdio. Each ACP session owns a persistent `pi --mode rpc` child process; Pi events are translated to ACP session updates. |

## Prerequisites

Authentication is handled entirely by the native Pi CLI, per the Cadencr
connector contract. Before installing or running the connector:

```bash
# Start Pi and use /login to authenticate, then /model to select a model
pi
```

The connector and Cadencr never collect, store, or broker Pi credentials.
Pi `0.85.1` is the version audited by the release's native protocol QA.

## Install the Pi CLI

```bash
# Via npm
npm install -g @earendil-works/pi-coding-agent@0.85.1

# Or see https://github.com/earendil-works/pi for other install methods
```

This installs the `pi` binary (Node.js >= 22.19.0 required). The connector spawns `pi --mode rpc`.

## Build

```bash
cargo build --release
```

## Binary resolution

`pi` is resolved in this order:

1. `PI_COMMAND` environment variable (absolute path recommended);
2. `CADENCR_PI_COMMAND` (legacy local installations only);
3. `pi` on `PATH`.

Managed marketplace processes remove inherited `CADENCR_*` variables. Use
`PI_COMMAND` for that installation mode. The connector is a native Rust binary;
Node is needed by Pi only.

## CadencR 0.16.0 marketplace

The independent provider is named **Pidancr** with ID `pidancr`,
publisher `edjubert`, and minimum app version `0.16.0`. The marketplace's
official `pi` connector is a separate implementation.

See [marketplace release instructions](https://github.com/edjubert/cadencr-plugin-provider-pi/blob/main/marketplace/README.md) for canonical
packaging, native platform qualification, author release and registry submission.
The repository's GitHub Actions workflow produces the reviewable release bundle;
publishing the author release and accepting the registry contribution are
separate steps.

The [Pidancr v0.2.0 release](https://github.com/edjubert/cadencr-plugin-provider-pi/releases/tag/v0.2.0)
is published for macOS and Linux on Intel/x86-64 and arm64.
[Registry PR #22](https://github.com/merkr-software/cadencr-registry/pull/22)
is awaiting maintainer acceptance before marketplace listing.

## Local Cadencr install

1. Copy `descriptor/pidancr.json` to `~/.cadencr/settings/providers/pidancr.json`.
2. Replace both `/ABSOLUTE/PATH/TO/...` placeholders with this repository's absolute path.
3. Restart Cadencr. Provider registration is read once at startup; hot reload is not supported.

Cadencr launched from the Finder does not inherit your shell `PATH`, so a
`pi` installed through a Node version manager (nvm, fnm, ...) is not found.
Add an `env` block to the descriptor's `installation.executable`:

```json
"env": {
  "PATH": "/ABSOLUTE/PATH/TO/node/bin:/opt/homebrew/bin:/usr/bin:/bin",
  "PI_COMMAND": "/ABSOLUTE/PATH/TO/node/bin/pi"
}
```

`PATH` must also contain the `node` that `pi`'s shebang resolves to.

To install from another directory, keep `installation.executable.command`
pointing at the release binary and `installation.assets.directory` at this
repository root. It contains `pi-logo.svg` for Cadencr and `icon.svg` for this
README.

### Cadencr development checkout

Settings default to a `cadencr-settings` directory beside the database file.
With the standard `pnpm dev` configuration (`CADENCR_DB_PATH=./cadencr.local.db`),
the service runs from `packages/service`, so install the descriptor at
`packages/service/cadencr-settings/providers/pidancr.json`.
The repository-root `cadencr-settings` directory is not read in this setup.
An explicit `CADENCR_SETTINGS_DIR` overrides this location.

Restart `pnpm dev` after installing the descriptor. Keep the executable and
assets paths absolute; a development install does not install the provider
into the packaged app's settings.

## Architecture

```
Cadencr host
    │ ACP v1 (stdio JSON-RPC)
    ▼
cadencr-plugin-provider-pi  (this connector)
    │ Pi's JSONL RPC protocol (`pi --mode rpc`)
    ▼
pi --mode rpc  (native Pi RPC child)
```

- Between the connector and Pi, communication uses **Pi's JSONL RPC protocol**
  (`pi --mode rpc`, see `docs/rpc.md` of `@earendil-works/pi-coding-agent`).
  This is separate from the ACP JSON-RPC connection between Cadencr and the
  connector.
- `models` uses a disposable Pi RPC process to discover available models.
- `run` starts an ACP v1 server. Each `session/new` spawns a fresh
  `pi --mode rpc` child.
- Cadencr title-generation sessions are marked through ACP metadata. The
  connector starts those Pi children with an ephemeral session, no tools,
  extensions, skills, prompt templates, themes, or context files, and replaces
  Pi's default system prompt with Cadencr's naming prompt. An empty
  append-system-prompt override also prevents Pi from loading global
  `APPEND_SYSTEM.md` instructions. The marker is
  `_meta.com.cadencr.purpose: "auto_name"`; `systemPrompt` carries the optional
  replacement text.
- The active model's `contextWindow` is sent to Cadencr as an ACP
  `usage_update` when the session starts or changes model. Pi's
  `get_session_stats` context estimate is forwarded when a turn settles.
- Pi `message_update` events with `text_delta` and `thinking_delta` become
  ACP `agent_message_chunk` and `agent_thought_chunk` updates.
- `tool_execution_start`, `tool_execution_update`, and `tool_execution_end`
  become ACP `tool_call` and `tool_call_update` updates.
- `agent_settled` ends `session/prompt`. Its ACP `stopReason` comes from the
  last assistant `message_end`: `stop`, `toolUse`, and `error` map to
  `end_turn`; `length` maps to `max_tokens`; `aborted` maps to `cancelled`.
- Durable resume: workspace-bound ACP identities restore the exact native Pi
  UUID through `pi --mode rpc --session-id <id>`. An unpersisted or missing
  session returns ACP's explicit session-not-found error.
- `session/close` terminates the Pi child process cleanly.

## Permissions

Pi has no tool-approval system; tools run without confirmation. Only extension
dialogs (`select`, `confirm`) are bridged to ACP
`session/request_permission`. Unsupported `input` and `editor` dialogs are
cancelled; notifications and status updates are ignored.

## Known limitations

- Compaction, fork, clone, steer, and follow-up operations are not supported.
- The connector has no separate direct bash operation and does not support
  image input in prompts.
- `--no-approve` is used for model discovery only. Sessions use Pi's default
  project-trust behavior.
- Version `0.2.0` replaces raw native session IDs with workspace-bound ACP IDs.
  Start a new CadencR conversation when upgrading from `0.1.0`; existing native
  sessions remain available through Pi itself.

## Notes

- Model discovery stays under Cadencr's 10-second host timeout with an
  internal 8-second budget.
- The connector relies on Pi's RPC mode (`pi --mode rpc`), not `pi-acp`.
- ACP session IDs bind Pi's native UUID to the canonical workspace.
- Thinking levels are exposed as a generic ACP select option when Pi
  models advertise multiple levels.
- Supported release targets must pass native QA on their own runners before
  appearing in marketplace metadata.

## License

The connector is licensed under [Apache-2.0](LICENSE).
See [third-party notices](THIRD_PARTY_NOTICES.md) for attribution.
