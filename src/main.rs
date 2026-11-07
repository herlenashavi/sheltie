mod aws;
mod config;
mod connect;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use config::{Config, config_path};
use connect::Mode;

/// Herd your SSM sessions: EC2 shells, RDS and private EKS in one command.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List targets.
    Ls {
        /// Machine-readable output (used by the Herdr mod).
        #[arg(long)]
        json: bool,
    },
    /// Connect: a shell, a database client, or a kube-ready shell.
    #[command(visible_alias = "c")]
    Connect {
        name: String,
        /// Local port instead of the configured one.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Only open the tunnel and hold it until ctrl+c.
    #[command(visible_alias = "t")]
    Tunnel {
        name: String,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Write an example config if there is none.
    Init,
    /// Open the config in $EDITOR (creating it from the example first).
    Edit,
    /// Check the tools sheltie needs and the config.
    Doctor,
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("sheltie: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Ls { json } => {
            let cfg = Config::load()?;
            let list = cfg.listings();
            if json {
                println!("{}", serde_json::to_string(&list)?);
            } else {
                let w = list.iter().map(|l| l.name.len()).max().unwrap_or(0);
                for l in list {
                    println!("{:<w$}  {:<6}  {}", l.name, l.kind, l.description.unwrap_or(""));
                }
            }
        }
        Cmd::Connect { name, port } => connect::run(&Config::load()?.resolve(&name)?, Mode::Connect, port)?,
        Cmd::Tunnel { name, port } => connect::run(&Config::load()?.resolve(&name)?, Mode::TunnelOnly, port)?,
        Cmd::Init => {
            let path = config_path();
            if path.exists() {
                bail!("{} already exists", path.display());
            }
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, config::EXAMPLE)?;
            println!("wrote {}: edit it with your targets", path.display());
        }
        Cmd::Edit => {
            let path = config_path();
            if !path.exists() {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, config::EXAMPLE)?;
            }
            let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status()?;
            if !status.success() {
                bail!("{editor} exited with {status}");
            }
        }
        Cmd::Doctor => doctor(),
    }
    Ok(())
}

fn doctor() {
    let check = |bin: &str, args: &[&str], hint: &str| {
        let ok = std::process::Command::new(bin)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        println!("{} {bin}{}", if ok { "✓" } else { "✗" }, if ok { String::new() } else { format!("  ({hint})") });
    };
    check("aws", &["--version"], "brew install awscli");
    check("session-manager-plugin", &["--version"], "brew install --cask session-manager-plugin");
    check("kubectl", &["version", "--client"], "only needed for eks targets");
    match Config::load() {
        Ok(c) => println!("✓ {} ({} targets)", config_path().display(), c.targets.len()),
        Err(e) => println!("✗ {e:#}"),
    }
}
