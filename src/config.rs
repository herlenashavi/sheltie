use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `~/.config/sheltie/targets.toml`, or `$SHELTIE_CONFIG`.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("SHELTIE_CONFIG") {
        return PathBuf::from(p);
    }
    xdg("XDG_CONFIG_HOME", ".config").join("sheltie").join("targets.toml")
}

/// `~/.local/state/sheltie`: tunnel logs and generated kubeconfigs.
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("sheltie")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(fallback),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default, rename = "target")]
    pub targets: Vec<Target>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Defaults {
    pub profile: Option<String>,
    pub region: Option<String>,
    /// Default SSM instance for targets that don't name one.
    pub instance: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Target {
    pub name: String,
    pub description: Option<String>,
    pub profile: Option<String>,
    pub region: Option<String>,
    /// The SSM-managed instance to connect to or tunnel through:
    /// an instance id (`i-0abc…`) or a tag filter (`Name=bastion`).
    pub instance: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    /// An interactive shell on the instance.
    Shell,
    /// A database (RDS/Aurora) or any TCP host the instance can reach.
    Rds {
        /// Endpoint hostname. Or give `cluster` / `db_instance` to look it up.
        host: Option<String>,
        cluster: Option<String>,
        db_instance: Option<String>,
        port: Option<u16>,
        local_port: Option<u16>,
        /// Run once the tunnel is up; `{local_port}`, `{host}`, `{port}` are filled in.
        client: Option<String>,
    },
    /// Generic TCP tunnel (Redis, OpenSearch, an internal HTTP service…).
    Tunnel {
        host: String,
        port: u16,
        local_port: Option<u16>,
        client: Option<String>,
    },
    /// A private EKS API endpoint: tunnel + a kubeconfig that points at it.
    Eks {
        cluster: String,
        local_port: Option<u16>,
    },
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Shell => "shell",
            Kind::Rds { .. } => "rds",
            Kind::Tunnel { .. } => "tunnel",
            Kind::Eks { .. } => "eks",
        }
    }
}

/// A target with defaults applied.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub target: Target,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub instance: Option<String>,
}

#[derive(Serialize)]
pub struct Listing<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub description: Option<&'a str>,
    pub profile: Option<&'a str>,
    pub region: Option<&'a str>,
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                bail!("no config at {} (run `sheltie init` to create one)", path.display())
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut seen = std::collections::HashSet::new();
        for t in &cfg.targets {
            if !seen.insert(t.name.as_str()) {
                bail!("{}: target name `{}` is used twice", path.display(), t.name);
            }
        }
        Ok(cfg)
    }

    pub fn resolve(&self, name: &str) -> Result<Resolved> {
        let Some(t) = self.targets.iter().find(|t| t.name == name) else {
            let names: Vec<_> = self.targets.iter().map(|t| t.name.as_str()).collect();
            bail!("no target `{name}`; known: {}", names.join(", "));
        };
        let d = &self.defaults;
        Ok(Resolved {
            profile: t.profile.clone().or_else(|| d.profile.clone()),
            region: t.region.clone().or_else(|| d.region.clone()),
            instance: t.instance.clone().or_else(|| d.instance.clone()),
            target: t.clone(),
        })
    }

    pub fn listings(&self) -> Vec<Listing<'_>> {
        self.targets
            .iter()
            .map(|t| Listing {
                name: &t.name,
                kind: t.kind.label(),
                description: t.description.as_deref(),
                profile: t.profile.as_deref().or(self.defaults.profile.as_deref()),
                region: t.region.as_deref().or(self.defaults.region.as_deref()),
            })
            .collect()
    }
}

pub const EXAMPLE: &str = include_str!("../examples/targets.toml");
