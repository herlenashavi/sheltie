# sheltie

Herd your SSM sessions. One command, or one popup in [Herdr](https://herdr.dev), to reach:

- **shells** on SSM-managed EC2 instances
- **RDS / Aurora** through a port-forward, opening `psql`/`mysql` once it's up
- any **TCP service** an instance can reach (Redis, OpenSearch…)
- **private EKS** API endpoints, with a kubeconfig of their own

It drives the `aws` CLI, so your profiles and SSO work as they do in your shell. An expired SSO session triggers `aws sso login`.

## Install

```sh
brew install awscli
brew install --cask session-manager-plugin
cargo install --path .
sheltie edit     # creates ~/.config/sheltie/targets.toml from the example
sheltie doctor
```

## Use

```sh
sheltie ls
sheltie connect orders-db     # tunnel + psql; the tunnel closes when psql exits
sheltie tunnel orders-db      # tunnel only, until ctrl+c
sheltie connect prod-eks      # tunnel + a shell with KUBECONFIG set
sheltie connect prod-bastion  # SSM shell
```

Targets are described in [`examples/targets.toml`](examples/targets.toml). `instance` is the SSM instance to connect to or tunnel through: an id (`i-…`) or a tag filter (`Name=bastion`).

Tunnels run in their own process group, so ctrl+c inside `psql` cancels a query without dropping the tunnel. Closing the terminal (SIGHUP), SIGTERM, or ctrl+c while only a tunnel is held all close the tunnel. Tunnel logs are in `~/.local/state/sheltie/logs/`, and generated kubeconfigs in `~/.local/state/sheltie/kube/`.

## Herdr mod

`herdr/sheltie` is a popup for the Herdr app. It lists your targets and opens each connection in its own tab (`rds:orders-db`), so closing the tab closes the session.

```sh
ln -s "$PWD/herdr/sheltie" ~/.config/herdr-app/mods/sheltie
herdr-app check ~/.config/herdr-app/mods/sheltie
herdr-app do mod.open --name sheltie
```

To bind it to a key, add this to `~/.config/herdr-app/keys.json`:

```json
{ "global": { "cmd-shift-a": "mod.open:sheltie" } }
```

In the popup: type to filter, ↵ connects, ⌃T opens just the tunnel, ⌃E edits targets, Esc closes it.
