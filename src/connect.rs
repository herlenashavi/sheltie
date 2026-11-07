use crate::aws;
use crate::config::{Kind, Resolved, state_dir};
use anyhow::{Context, Result, bail};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::fs::{self, File};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What to do once the target is reachable.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Open the client (psql, a kube shell…) when there is one.
    Connect,
    /// Only hold the tunnel open.
    TunnelOnly,
}

pub fn run(r: &Resolved, mode: Mode, port_override: Option<u16>) -> Result<()> {
    aws::ensure_login(r)?;
    let instance = aws::find_instance(r)?;
    match r.target.kind.clone() {
        Kind::Shell => shell(r, &instance),
        Kind::Rds { host, cluster, db_instance, port, local_port, client } => {
            let (host, port) = match host {
                Some(h) => (h, port.context("rds target with `host` also needs `port`")?),
                None => {
                    let (h, p) = aws::rds_endpoint(r, cluster.as_deref(), db_instance.as_deref())?;
                    (h, port.unwrap_or(p))
                }
            };
            let local = port_override.or(local_port).unwrap_or(port);
            forward(r, &instance, &host, port, local, client.filter(|_| mode == Mode::Connect))
        }
        Kind::Tunnel { host, port, local_port, client } => {
            let local = port_override.or(local_port).unwrap_or(port);
            forward(r, &instance, &host, port, local, client.filter(|_| mode == Mode::Connect))
        }
        Kind::Eks { cluster, local_port } => {
            let local = port_override.or(local_port).unwrap_or(8443);
            eks(r, &instance, &cluster, local, mode)
        }
    }
}

fn shell(r: &Resolved, instance: &str) -> Result<()> {
    eprintln!("sheltie: {} → {instance}", r.target.name);
    let err = aws::cmd(r).args(["ssm", "start-session", "--target", instance]).exec();
    Err(err).context("starting `aws ssm start-session` (is session-manager-plugin installed?)")
}

fn forward(
    r: &Resolved,
    instance: &str,
    host: &str,
    port: u16,
    local: u16,
    client: Option<String>,
) -> Result<()> {
    let tunnel = Tunnel::start(r, instance, host, port, local)?;
    eprintln!("sheltie: {} ready on 127.0.0.1:{local} → {host}:{port}", r.target.name);
    match client {
        Some(c) => {
            let line = fill(&c, local, host, port);
            eprintln!("sheltie: $ {line}");
            tunnel.run_client(Command::new("sh").args(["-c", &line]))
        }
        None => {
            eprintln!("sheltie: holding the tunnel open, ctrl+c to close");
            tunnel.hold()
        }
    }
}

fn eks(r: &Resolved, instance: &str, cluster: &str, local: u16, mode: Mode) -> Result<()> {
    let info = aws::eks_cluster(r, cluster)?;
    let kubeconfig = write_kubeconfig(r, cluster, &info, local)?;
    let tunnel = Tunnel::start(r, instance, &info.endpoint_host, 443, local)?;
    eprintln!("sheltie: {} API on 127.0.0.1:{local}", r.target.name);
    eprintln!("sheltie: KUBECONFIG={}", kubeconfig.display());
    if mode == Mode::TunnelOnly {
        eprintln!("sheltie: holding the tunnel open, ctrl+c to close");
        return tunnel.hold();
    }
    let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    eprintln!("sheltie: kubectl is ready in this shell; exit to close the tunnel");
    tunnel.run_client(
        Command::new(sh)
            .env("KUBECONFIG", &kubeconfig)
            .env("SHELTIE_TARGET", &r.target.name),
    )
}

/// A kubeconfig of its own per target, so your default context is untouched.
fn write_kubeconfig(r: &Resolved, cluster: &str, info: &aws::EksCluster, local: u16) -> Result<PathBuf> {
    let dir = state_dir().join("kube");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.yaml", r.target.name));
    let name = &r.target.name;
    let mut args = format!("        - eks\n        - get-token\n        - --cluster-name\n        - {cluster}\n");
    if let Some(reg) = &r.region {
        args += &format!("        - --region\n        - {reg}\n");
    }
    let env = match &r.profile {
        Some(p) => format!("      env:\n        - name: AWS_PROFILE\n          value: {p}\n"),
        None => String::new(),
    };
    let yaml = format!(
        "apiVersion: v1
kind: Config
clusters:
  - name: {name}
    cluster:
      server: https://127.0.0.1:{local}
      tls-server-name: {host}
      certificate-authority-data: {ca}
users:
  - name: {name}
    user:
      exec:
        apiVersion: client.authentication.k8s.io/v1beta1
        command: aws
        args:
{args}{env}        interactiveMode: Never
contexts:
  - name: {name}
    context:
      cluster: {name}
      user: {name}
current-context: {name}
",
        host = info.endpoint_host,
        ca = info.ca_data,
    );
    fs::write(&path, yaml)?;
    Ok(path)
}

fn fill(template: &str, local: u16, host: &str, port: u16) -> String {
    template
        .replace("{local_port}", &local.to_string())
        .replace("{host}", host)
        .replace("{port}", &port.to_string())
}

/// The SSM port-forward, in its own process group so ctrl+c in the client
/// (cancelling a query) doesn't take the tunnel down with it.
struct Tunnel {
    child: Child,
    log: PathBuf,
    client_running: Arc<AtomicBool>,
}

impl Tunnel {
    fn start(r: &Resolved, instance: &str, host: &str, port: u16, local: u16) -> Result<Tunnel> {
        if TcpListener::bind((Ipv4Addr::LOCALHOST, local)).is_err() {
            bail!("127.0.0.1:{local} is in use (another tunnel? pass --port to pick another)");
        }
        let dir = state_dir().join("logs");
        fs::create_dir_all(&dir)?;
        let log = dir.join(format!("{}.log", r.target.name));
        let out = File::create(&log)?;
        let child = aws::port_forward(r, instance, host, port, local)
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .process_group(0)
            .spawn()
            .context("starting `aws ssm start-session` (is session-manager-plugin installed?)")?;
        let mut t = Tunnel { child, log, client_running: Arc::new(AtomicBool::new(false)) };
        t.watch_signals()?;
        t.wait_ready(local)?;
        Ok(t)
    }

    /// Closing the tab (SIGHUP), SIGTERM, or ctrl+c with no client running
    /// ends the tunnel's whole process group (aws + session-manager-plugin).
    fn watch_signals(&self) -> Result<()> {
        let mut signals = Signals::new([SIGINT, SIGTERM, SIGHUP])?;
        let pgid = self.child.id() as i32;
        let client = self.client_running.clone();
        std::thread::spawn(move || {
            for sig in signals.forever() {
                if sig == SIGINT && client.load(Ordering::SeqCst) {
                    continue; // the client handles its own ctrl+c
                }
                unsafe { libc::kill(-pgid, libc::SIGTERM) };
                std::process::exit(128 + sig);
            }
        });
        Ok(())
    }

    fn wait_ready(&mut self, local: u16) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Some(status) = self.child.try_wait()? {
                bail!("tunnel exited ({status}):\n{}", self.log_tail());
            }
            if TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, local).into(), Duration::from_millis(200)).is_ok() {
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("tunnel not listening on {local} after 45s:\n{}", self.log_tail());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn log_tail(&self) -> String {
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<_> = text.lines().rev().take(15).collect();
        lines.into_iter().rev().collect::<Vec<_>>().join("\n")
    }

    fn run_client(self, cmd: &mut Command) -> Result<()> {
        self.client_running.store(true, Ordering::SeqCst);
        let status = cmd.status().context("starting the client")?;
        self.client_running.store(false, Ordering::SeqCst);
        drop(self);
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }

    fn hold(mut self) -> Result<()> {
        let status = self.child.wait()?;
        bail!("tunnel closed ({status}):\n{}", self.log_tail())
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGTERM) };
        let _ = self.child.wait();
    }
}
