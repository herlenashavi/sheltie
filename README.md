# sheltie

Herd your SSM sessions. Sheltie connects you to AWS resources through AWS Systems Manager with one command, or one popup in the [Herdr](https://herdr.dev) app:

- **shells** on SSM-managed EC2 instances
- **RDS / Aurora** through a port-forward, opening `psql`/`mysql` once it's up
- any **TCP service** an instance can reach (Redis, OpenSearch, internal HTTP…)
- **private EKS** API endpoints, with a kubeconfig of their own

It drives the `aws` CLI, so your profiles, SSO and credential helpers work as they do in your shell. An expired SSO session triggers `aws sso login` for you.

## Contents

- [Requirements](#requirements)
- [Installation](#installation)
- [Configuration](#configuration)
- [Usage](#usage)
- [Herdr mod](#herdr-mod)
- [How it works](#how-it-works)
- [Troubleshooting](#troubleshooting)
- [Uninstall](#uninstall)

## Requirements

On your machine:

| Tool | Install | Needed for |
|---|---|---|
| Rust toolchain | [rustup.rs](https://rustup.rs) | building sheltie |
| AWS CLI v2 | `brew install awscli` | everything |
| Session Manager plugin | `brew install --cask session-manager-plugin` | everything |
| `kubectl` | `brew install kubectl` | `eks` targets |
| Your DB client (`psql`, `mysql`, `redis-cli`…) | e.g. `brew install libpq` | `client` commands |

In AWS:

- The instance you connect to or tunnel through is **SSM-managed**: the SSM agent is running and its instance profile allows SSM (e.g. `AmazonSSMManagedInstanceCore`). Check with `aws ssm describe-instance-information`.
- Your identity may call `ssm:StartSession` (on the instance and the `AWS-StartSSMSession` / `AWS-StartPortForwardingSessionToRemoteHost` documents), `ssm:TerminateSession`, and `ec2:DescribeInstances` when you select instances by tag.
- For lookups: `rds:DescribeDBClusters` / `rds:DescribeDBInstances` (`cluster` / `db_instance`), and `eks:DescribeCluster` for EKS.
- The instance can reach the target: its security group is allowed into the database's or EKS control plane's security group on the target port.
- For EKS, your IAM identity has access to the cluster (an access entry or `aws-auth` mapping).

## Installation

```sh
git clone git@github.com:herlenashavi/sheltie.git
cd sheltie
cargo install --path .       # installs ~/.cargo/bin/sheltie
```

Make sure `~/.cargo/bin` is on your `PATH`, then create your config and check your setup:

```sh
sheltie edit      # creates ~/.config/sheltie/targets.toml from the example and opens it in $EDITOR
sheltie doctor    # checks aws, session-manager-plugin, kubectl and the config
```

To update later: `git pull && cargo install --path .`

## Configuration

Targets live in `~/.config/sheltie/targets.toml` (or the file named by `$SHELTIE_CONFIG`). [`examples/targets.toml`](examples/targets.toml) is a full example.

### Defaults

```toml
[defaults]
profile = "prod"          # AWS CLI profile (SSO profiles log in when expired)
region = "us-east-1"
instance = "Name=bastion" # SSM instance to connect to or tunnel through
```

Every target can override `profile`, `region` and `instance`.

`instance` is either an instance id (`i-0123456789abcdef0`) or a tag filter `Key=Value`. With a tag, the first **running** instance carrying it is used.

### Common fields

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | What you type: `sheltie connect <name>`. Unique. |
| `kind` | yes | `shell`, `rds`, `tunnel` or `eks` |
| `description` | no | Shown by `sheltie ls` and in the Herdr popup |
| `profile`, `region`, `instance` | no | Override `[defaults]` |

### `shell`: a shell on the instance

```toml
[[target]]
name = "prod-bastion"
kind = "shell"
description = "Shell on the prod bastion"
```

### `rds`: a database

Give the endpoint, or let sheltie look it up:

```toml
[[target]]
name = "orders-db"
kind = "rds"
cluster = "orders"        # Aurora cluster id; or db_instance = "orders-1"; or host = "…" + port = 5432
local_port = 15432        # default: the database's port
client = "psql -h 127.0.0.1 -p {local_port} -U app orders"
```

| Field | Meaning |
|---|---|
| `host` + `port` | Endpoint, given directly |
| `cluster` | Aurora cluster identifier; its writer endpoint is looked up |
| `db_instance` | RDS instance identifier; its endpoint is looked up |
| `port` | Overrides the looked-up port |
| `local_port` | Port on `127.0.0.1` (default: same as `port`) |
| `client` | Command run once the tunnel is up; the tunnel closes when it exits. Leave it out to just hold the tunnel. `{local_port}`, `{host}` and `{port}` are filled in. Runs through `sh -c`. |

Passwords: let the client find them in the usual places (`~/.pgpass`, `PGPASSWORD`, `~/.my.cnf`), or use IAM auth, for example:

```toml
client = "PGPASSWORD=$(aws rds generate-db-auth-token --profile prod --hostname {host} --port {port} --username app) psql 'host=127.0.0.1 port={local_port} user=app dbname=orders sslmode=require'"
```

### `tunnel`: any TCP service

```toml
[[target]]
name = "cache"
kind = "tunnel"
host = "cache.abc123.use1.cache.amazonaws.com"
port = 6379
local_port = 16379
client = "redis-cli -p {local_port}"   # optional
```

### `eks`: a private EKS cluster

```toml
[[target]]
name = "prod-eks"
kind = "eks"
cluster = "prod"
local_port = 8443         # default 8443
```

Sheltie tunnels to the cluster's API endpoint and writes `~/.local/state/sheltie/kube/<name>.yaml`. That kubeconfig points at `https://127.0.0.1:<local_port>`, sets `tls-server-name` to the real endpoint so TLS still verifies, and authenticates with `aws eks get-token`. Your `~/.kube/config` is never touched.

## Usage

```sh
sheltie ls                    # list targets (--json for scripts)
sheltie connect <name>        # alias: sheltie c <name>
sheltie tunnel <name>         # alias: sheltie t <name>
sheltie connect <name> --port 25432   # use another local port this time
sheltie edit                  # open the config in $VISUAL / $EDITOR
sheltie doctor                # check tools and config
```

What `connect` does for each kind:

| Kind | `sheltie connect` | `sheltie tunnel` |
|---|---|---|
| `shell` | SSM shell on the instance | same |
| `rds` / `tunnel` | Opens the tunnel, runs `client`, closes the tunnel when it exits. Without `client`, holds the tunnel. | Holds the tunnel until ctrl+c |
| `eks` | Opens the tunnel and starts `$SHELL` with `KUBECONFIG` set. `kubectl` works; `exit` closes the tunnel. | Holds the tunnel; use `KUBECONFIG=~/.local/state/sheltie/kube/<name>.yaml` from another terminal |

Examples:

```sh
sheltie c orders-db           # psql prompt, through the bastion
sheltie t orders-db           # keep the tunnel up for a GUI client (TablePlus, DBeaver) on 127.0.0.1:15432
sheltie c prod-eks            # then: kubectl get pods -A
```

## Herdr mod

`herdr/sheltie` is a popup for the [Herdr](https://herdr.dev) desktop app. It lists your targets and opens each connection in a tab of its own (named `rds:orders-db`, `eks:prod-eks`…), so closing the tab closes the session.

Install it by linking the folder into Herdr's mods, so `git pull` updates it:

```sh
ln -s "$PWD/herdr/sheltie" ~/.config/herdr-app/mods/sheltie
herdr-app check ~/.config/herdr-app/mods/sheltie
```

Open it from the command palette (⌘⇧P → *mod.open*, name `sheltie`), from a shell with `herdr-app do mod.open --name sheltie`, or bind a key in `~/.config/herdr-app/keys.json`:

```json
{ "global": { "cmd-shift-a": "mod.open:sheltie" } }
```

| In the popup | |
|---|---|
| type | filter by name, kind, description or profile |
| ↑ / ↓ | move |
| ↵ or click | `sheltie connect` in a new tab |
| ⌃T | `sheltie tunnel` in a new tab |
| ⌃E | edit targets in a new tab |
| Esc | close |

The mod runs `sheltie` from your login `PATH`, falling back to `~/.cargo/bin/sheltie`.

## How it works

- Shells use `aws ssm start-session --target <instance>`.
- Tunnels use `aws ssm start-session` with the `AWS-StartPortForwardingSessionToRemoteHost` document. Sheltie waits until the local port accepts connections before starting the client.
- The tunnel runs in its own process group, so ctrl+c inside the client (cancelling a query) doesn't drop it. Closing the terminal (SIGHUP), SIGTERM, the client exiting, or ctrl+c while only a tunnel is held all end it, `session-manager-plugin` included.
- Before connecting, sheltie checks credentials with `aws sts get-caller-identity`. If they fail and the profile uses SSO, it runs `aws sso login`.

Files:

| Path | |
|---|---|
| `~/.config/sheltie/targets.toml` | your targets (`$SHELTIE_CONFIG`, `$XDG_CONFIG_HOME` respected) |
| `~/.local/state/sheltie/logs/<name>.log` | output of the last tunnel for each target |
| `~/.local/state/sheltie/kube/<name>.yaml` | generated kubeconfigs |

## Troubleshooting

| Symptom | Fix |
|---|---|
| `running aws … No such file or directory` | Install the AWS CLI; run `sheltie doctor` |
| `SessionManagerPlugin is not found` | `brew install --cask session-manager-plugin` |
| `no running instance with tag …` | Check the tag and that the instance runs in that profile/region |
| `TargetNotConnected` | The instance isn't SSM-managed or its agent is offline: `aws ssm describe-instance-information` |
| `127.0.0.1:<port> is in use` | Another tunnel holds it; close it or pass `--port` |
| `tunnel exited` / `not listening after 45s` | The log tail is printed; full log in `~/.local/state/sheltie/logs/` |
| Client connects, then times out | The instance can't reach the target: security groups / NACLs |
| `kubectl`: `Unauthorized` | Your IAM identity has no access entry on the cluster |
| Popup says sheltie isn't installed | `cargo install --path .`, and make sure `~/.cargo/bin` is on your login `PATH` |

## Uninstall

```sh
cargo uninstall sheltie
rm ~/.config/herdr-app/mods/sheltie
rm -r ~/.config/sheltie ~/.local/state/sheltie   # config, logs, kubeconfigs
```
