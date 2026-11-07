//! Thin wrappers over the `aws` CLI, so profiles, SSO and credential
//! helpers behave exactly as they do in your shell.

use crate::config::Resolved;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::process::{Command, Stdio};

pub fn cmd(r: &Resolved) -> Command {
    let mut c = Command::new("aws");
    if let Some(p) = &r.profile {
        c.args(["--profile", p]);
    }
    if let Some(reg) = &r.region {
        c.args(["--region", reg]);
    }
    c
}

/// Run an `aws` command and parse its JSON output.
pub fn json(r: &Resolved, args: &[&str]) -> Result<Value> {
    let out = cmd(r)
        .args(args)
        .args(["--output", "json"])
        .stdin(Stdio::null())
        .output()
        .context("running `aws` (is the AWS CLI installed? `brew install awscli`)")?;
    if !out.status.success() {
        bail!("aws {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("aws returned invalid JSON")
}

/// Make sure the profile has working credentials; run `aws sso login` when
/// an SSO session has expired.
pub fn ensure_login(r: &Resolved) -> Result<()> {
    let ok = cmd(r)
        .args(["sts", "get-caller-identity"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running `aws` (is the AWS CLI installed? `brew install awscli`)")?
        .success();
    if ok {
        return Ok(());
    }
    if !uses_sso(r) {
        bail!(
            "no valid AWS credentials for profile `{}`",
            r.profile.as_deref().unwrap_or("default")
        );
    }
    eprintln!("sheltie: SSO session expired, logging in…");
    let mut login = Command::new("aws");
    login.args(["sso", "login"]);
    if let Some(p) = &r.profile {
        login.args(["--profile", p]);
    }
    if !login.status()?.success() {
        bail!("aws sso login failed");
    }
    Ok(())
}

fn uses_sso(r: &Resolved) -> bool {
    ["sso_session", "sso_start_url"].iter().any(|key| {
        let mut c = Command::new("aws");
        c.args(["configure", "get", key]);
        if let Some(p) = &r.profile {
            c.args(["--profile", p]);
        }
        c.stdin(Stdio::null())
            .output()
            .map(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
            .unwrap_or(false)
    })
}

/// Turn `i-0abc…` or `Key=Value` into a running instance id.
pub fn find_instance(r: &Resolved) -> Result<String> {
    let Some(spec) = r.instance.as_deref() else {
        bail!(
            "target `{}` has no `instance` (set it, or `instance` under [defaults])",
            r.target.name
        );
    };
    if spec.starts_with("i-") {
        return Ok(spec.to_string());
    }
    let Some((key, value)) = spec.split_once('=') else {
        bail!("instance `{spec}` should be an id (i-…) or a tag filter (Name=bastion)");
    };
    let v = json(
        r,
        &[
            "ec2",
            "describe-instances",
            "--filters",
            &format!("Name=tag:{key},Values={value}"),
            "Name=instance-state-name,Values=running",
            "--query",
            "Reservations[].Instances[].InstanceId",
        ],
    )?;
    let ids: Vec<&str> = v.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    match ids.first() {
        Some(id) => Ok(id.to_string()),
        None => bail!("no running instance with tag {key}={value}"),
    }
}

/// An RDS cluster or instance endpoint and its port.
pub fn rds_endpoint(r: &Resolved, cluster: Option<&str>, instance: Option<&str>) -> Result<(String, u16)> {
    let (host, port) = if let Some(c) = cluster {
        let v = json(r, &["rds", "describe-db-clusters", "--db-cluster-identifier", c])?;
        let c = &v["DBClusters"][0];
        (c["Endpoint"].as_str().map(String::from), c["Port"].as_u64())
    } else if let Some(i) = instance {
        let v = json(r, &["rds", "describe-db-instances", "--db-instance-identifier", i])?;
        let e = &v["DBInstances"][0]["Endpoint"];
        (e["Address"].as_str().map(String::from), e["Port"].as_u64())
    } else {
        bail!("rds target `{}` needs `host`, `cluster` or `db_instance`", r.target.name);
    };
    match (host, port) {
        (Some(h), Some(p)) => Ok((h, p as u16)),
        _ => bail!("could not read the endpoint for `{}`", r.target.name),
    }
}

pub struct EksCluster {
    pub endpoint_host: String,
    pub ca_data: String,
}

pub fn eks_cluster(r: &Resolved, name: &str) -> Result<EksCluster> {
    let v = json(r, &["eks", "describe-cluster", "--name", name])?;
    let c = &v["cluster"];
    let endpoint = c["endpoint"].as_str().context("cluster has no endpoint")?;
    let ca = c["certificateAuthority"]["data"].as_str().context("cluster has no CA data")?;
    Ok(EksCluster {
        endpoint_host: endpoint.trim_start_matches("https://").trim_end_matches('/').to_string(),
        ca_data: ca.to_string(),
    })
}

/// `aws ssm start-session` forwarding `127.0.0.1:local` to `host:port` via the instance.
pub fn port_forward(r: &Resolved, instance: &str, host: &str, port: u16, local: u16) -> Command {
    let mut c = cmd(r);
    c.args([
        "ssm",
        "start-session",
        "--target",
        instance,
        "--document-name",
        "AWS-StartPortForwardingSessionToRemoteHost",
        "--parameters",
        &serde_json::json!({
            "host": [host],
            "portNumber": [port.to_string()],
            "localPortNumber": [local.to_string()],
        })
        .to_string(),
    ]);
    c
}
