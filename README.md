# cadencr-plugin-provider-pi

Cadencr code-backed provider connector for the [Pi coding agent](https://github.com/earendil-works/pi).

The connector implements the Cadencr provider executable contract:

| Command | Behavior |
| --- | --- |
| `version` | Prints the connector version. |
| `models --format acp-config-options-v1 --cwd <abs>` | Launches `pi --mode rpc --no-session --no-approve` in the workspace, calls `get_available_models` and `get_state`, and prints the ACP v1 `SessionConfigOption` array. |
| `run --protocol acp-v1` | Starts an ACP v1 JSON-RPC server over stdio. Each ACP session owns a persistent `pi --mode rpc` child process; Pi RPC events are bridged to ACP session updates. |

## Prerequisites

Authentication is handled entirely by the native Pi CLI, per the Cadencr
connector contract. Before installing or running the connector:

```bash
# Configure Pi providers and API keys
pi config

# Or edit ~/.pi/agent/settings.json directly
```

The connector and Cadencr never collect, store, or broker Pi credentials.

## Install the Pi CLI

```bash
# Via npm
npm install -g @earendil-works/pi-coding-agent

# Or see https://github.com/earendil-works/pi for other install methods
```

This installs the `pi` binary (Node.js >= 22.19.0 required). The connector spawns `pi --mode rpc`.

## Build

```bash
cargo build --release
```

## Binary resolution

`pi` is resolved in this order:

1. `CADENCR_PI_COMMAND` environment variable (absolute path recommended);
2. `pi` on `PATH`.

## Local Cadencr install

1. Copy `descriptor/pi.json` to `~/.cadencr/settings/providers/pi.json`.
2. Replace both `/ABSOLUTE/PATH/TO/...` placeholders with this repository's absolute path.
3. Restart Cadencr. Provider registration is read once at startup; hot reload is not supported.

Cadencr launched from the Finder does not inherit your shell `PATH`, so a
`pi` installed through a Node version manager (nvm, fnm, ...) is not found.
Add an `env` block to the descriptor's `installation.executable`:

```json
"env": {
  "PATH": "/ABSOLUTE/PATH/TO/node/bin:/opt/homebrew/bin:/usr/bin:/bin",
  "CADENCR_PI_COMMAND": "/ABSOLUTE/PATH/TO/node/bin/pi"
}
```

`PATH` must also contain the `node` that `pi`'s shebang resolves to.

To install from another directory, keep `installation.executable.command`
pointing at the release binary and `installation.assets.directory` at this
repository root (the directory that holds `icon.svg`).

### Cadencr development checkout

Settings default to a `cadencr-settings` directory beside the database file.
With the standard `pnpm dev` configuration (`CADENCR_DB_PATH=./cadencr.local.db`),
the service runs from `packages/service`, so install the descriptor at
`packages/service/cadencr-settings/providers/pi.json`.
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
    │ spawns per ACP session
    ▼
pi --mode rpc  (native Pi RPC child)
```

- `models` uses a disposable Pi RPC process to discover available models.
- `run` starts an ACP v1 server. Each `session/new` spawns a fresh
  `pi --mode rpc` child. Pi RPC events (message streams, tool calls,
  permissions, thinking, stats) are translated to ACP `session/update`
  notifications.
- Durable resume: `session/resume` spawns `pi --mode rpc --session <id>`
  to restore the native Pi session context.
- `session/close` terminates the Pi child process cleanly.

## Notes

- Model discovery stays under Cadencr's 10-second host timeout with an
  internal 8-second budget.
- The connector relies on Pi's RPC mode (`pi --mode rpc`), not `pi-acp`.
- Pi's native session ID is used as the ACP session ID for durable resume.
- Thinking levels are exposed as a generic ACP select option when Pi
  models advertise multiple levels.
- Publication to the Cadencr registry (signed package via `pack-provider`)
  is a later step; the registry contribution gate is not open yet.

## License

The connector is licensed under [Apache-2.0](LICENSE).
See [third-party notices](THIRD_PARTY_NOTICES.md) for attribution.
