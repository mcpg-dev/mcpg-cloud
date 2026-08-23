# mcpg cloud — mcpg.cloud tenant CLI

`mcpg-cloud` is the terminal client for mcpg.cloud, the managed MCP Gateway
service. You hand it a gateway configuration file and it publishes that file as
a running, addressable gateway; from there it is how you inspect instances,
follow logs, diff and roll back config versions, prove ownership of a custom
domain, and mint machine credentials for CI. It talks only to the control plane's
HTTP API — it links no server code and can be installed on its own.

**Rust · clap-parsed · OIDC PKCE sign-in · streaming publish**

## What it does

- Signs in with an OIDC authorization-code flow using PKCE (`login`) and stores
  the resulting tokens in `<state-dir>/credentials.json`; every other command
  attaches them as `Authorization: Bearer` automatically.
- Publishes a config file as a named managed gateway (`publish <NAME>
  --config gw.yaml`) and streams the provisioning phase ladder as it happens.
  Re-publishing the same name updates that instance in place instead of
  creating a duplicate.
- Records each publish as a numbered config version, so `versions`, `diff
  --from N --to M`, and `rollback --to N` work against real history.
- Lists running gateways with their endpoint URLs and instance uids
  (`instances`), and recent provisioning operations for the org (`operations`).
- Shows a gateway's recent logs as a tail snapshot, or streams them live with
  `logs <NAME> --follow`.
- Tears an instance down by published name or by instance uid — a UUID-shaped
  target is treated as a uid, anything else is resolved as a name.
- Claims and verifies custom domains: `domains add` prints the DNS TXT record to
  create, `domains verify` checks it and marks the domain verified. The control
  plane refuses to publish `--custom-hostname` for an unverified domain.
- Mints, lists, and revokes org service tokens — non-interactive `mcpgst_…`
  bearers for CI that are not tied to a human login. The secret is printed once,
  at creation, and never stored.
- Reports which orgs the current login can act on (`whoami`), and pins a default
  org, workspace, and environment so later commands take only their primary noun
  (`use`).
- Resolves the org / workspace / environment coordinates a command needs from
  flags, then environment variables, then the stored `use` context, and fails
  with an error naming all three sources when none supplies a value.

## Install / Run

The crate is `publish = false`; build it from this workspace or install the
released binary alongside the rest of the `mcpg` toolchain.

```bash
cargo build -p mcpg-cloud --release      # → target/release/mcpg-cloud
```

`mcpg` dispatches bare-word subcommands to sibling binaries, so with
`mcpg-cloud` on `PATH` (or next to `mcpg`) every command is reachable as
`mcpg cloud …` as well as `mcpg-cloud …`.

```bash
mcpg cloud login --issuer https://auth.mcpg.dev
mcpg cloud use --org acme --workspace default --env prod

mcpg cloud publish edge --config gateway.yaml --replicas 2 --size m
mcpg cloud instances
mcpg cloud logs edge --follow
```

## Configuration

There is no config file. The flags below have environment-variable forms; the
per-command deployment flags are argv-only. Every one of them is global except
`--issuer`, which belongs to `login`. The org, workspace, and environment
coordinates can be pinned once with `mcpg cloud use`, which writes
`<state-dir>/context.json`.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--cp-url` | `MCPG_CP_URL` | `http://127.0.0.1:7843` | Control-plane HTTP base URL. `--control-plane-url` is an accepted alias. |
| `--org` | `MCPG_ORG` | from `use` context | Org slug. |
| `--workspace` | `MCPG_WORKSPACE` | from `use` context | Workspace. |
| `--env` | `MCPG_ENV` | from `use` context | Environment. |
| `--state-dir` | `MCPG_STATE_DIR` | `~/.mcpg` (`./mcpg-state` when no home directory is detectable) | Where credentials and the stored context live. |
| `--json-logs` | `MCPG_JSON_LOGS` | off | Emit structured JSON logs instead of the human-readable format. |
| `--issuer` | `MCPG_FED_ISSUER` | — | OIDC issuer for `login`. |

Precedence for the coordinates is flag, then environment variable, then the
stored context. `RUST_LOG` sets the tracing filter and defaults to `info`.
Against a loopback control plane running with authentication disabled, no login
is required and commands work with no stored credentials.

## Publishing

```bash
mcpg cloud publish edge \
  --config gateway.yaml \
  --image-tag latest \
  --replicas 2 \
  --size m \
  --region eu-west-1 \
  --custom-hostname mcp.example.com
```

| Flag | Default | Description |
|---|---|---|
| `--config` | — | Path to the config file. Read verbatim and validated server-side. |
| `--image-tag` | `latest` | Gateway image tag to run. |
| `--replicas` | `1` | Replica count. |
| `--size` | `s` | Instance size class: `s`, `m`, `l`, or `xl`. |
| `--region` | platform default | Target region for the instance. |
| `--isolation-tier` | platform default | Requested isolation tier. |
| `--custom-hostname` | — | A hostname already verified through `mcpg cloud domains`. |

The size class is capped by the org's plan — `community` reaches `s`, `pro` `m`,
`team` `l`, and `enterprise` `xl` — and the control plane, not the CLI, enforces
that cap. `rollback` takes the same deployment flags as `publish` apart from
`--custom-hostname`, and they are not inherited from the version being restored,
so pass them explicitly when the instance deviates from the defaults.

## Service tokens

```bash
mcpg cloud service-token create ci-deploy --role member --expires-days 90
mcpg cloud service-token list
mcpg cloud service-token revoke <ID>
```

Creating a token requires org ownership. `--role` is repeatable and defaults to
`member`; `--expires-days` defaults to 90, and `0` means the token never
expires. The secret is displayed once at creation — nothing stores it, so
`list` can only ever show ids and metadata.

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets
```

The full integration suite — it boots a real control plane and provisioner and
drives the command helpers against the live HTTP API — runs upstream, where
those services live.

## Licence

Apache-2.0. See [LICENSE](LICENSE).

## See also

- <https://mcpg.dev/docs/reference/cli/cloud> — the full command reference
- <https://mcpg.cloud/docs/publish-a-config> — what a publish does end to end
- <https://mcpg.cloud/docs/versions-and-rollback> — config versions, diffs, rollbacks
- <https://mcpg.cloud/docs/custom-domains> — claiming and verifying a hostname
