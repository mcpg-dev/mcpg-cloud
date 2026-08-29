//! The tenant-facing managed-service commands behind `mcpg cloud …`.
//!
//! Authenticates with the OIDC id_token stored by `mcpg cloud login`
//! (`<state_dir>/credentials.json`), attached as `Authorization: Bearer`. The
//! CP accepts that bearer token (see the CP's AuthContext). In a loopback CP
//! (`auth_mode=none`) no token is needed and commands still work.
//!
//! Commands hit the CP HTTP API:
//! - `publish`  → `POST /v1/orgs/.../gateways` (SSE phase ladder). Re-publishing
//!   the same `--name` updates the instance in place (the provisioner reuses
//!   its coords) rather than creating a duplicate.
//! - `list`     → `GET  /v1/orgs/:org/operations`.
//! - `delete`   → `DELETE /v1/orgs/.../gateways/:instanceUid` (SSE).

use std::path::Path;

use anyhow::Context;
use mcpg_cli_core::client::{bearer_client as client, bearer_token, cp_error};
use serde::Serialize;

#[derive(Serialize)]
struct PublishBody {
    name: String,
    replicas: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    region: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    isolation_tier: String,
    size: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    custom_hostname: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    config_toml: String,
}

#[allow(clippy::too_many_arguments)]
pub struct PublishArgs {
    pub name: String,
    pub replicas: u32,
    pub region: String,
    pub isolation_tier: String,
    /// Instance size class (`s` | `m` | `l` | `xl`); the CP plan-gates it.
    pub size: String,
    pub custom_hostname: String,
    /// Path to the config file to publish (read verbatim, validated server-side
    /// by the publish guard).
    pub config_file: Option<String>,
}

/// Publish a config → create or update the named instance, streaming the phase
/// ladder. Re-publishing the same name updates in place.
pub async fn publish(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    workspace: &str,
    environment: &str,
    args: PublishArgs,
) -> anyhow::Result<()> {
    let config_toml = match &args.config_file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read config file {path}"))?
        }
        None => String::new(),
    };
    let body = PublishBody {
        name: args.name.clone(),
        replicas: args.replicas,
        region: args.region,
        isolation_tier: args.isolation_tier,
        size: args.size,
        custom_hostname: args.custom_hostname,
        config_toml,
    };

    let resp = client(state_dir)
        .await?
        .post(format!(
            "{cp_url}/v1/orgs/{org}/workspaces/{workspace}/environments/{environment}/gateways"
        ))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("publish '{}'", args.name), resp).await);
    }
    println!("publishing '{}' …", args.name);
    mcpg_cli_core::stream::stream_phases(resp).await
}

pub async fn delete(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    workspace: &str,
    environment: &str,
    instance_uid: &str,
) -> anyhow::Result<()> {
    let resp = client(state_dir).await?
        .delete(format!(
            "{cp_url}/v1/orgs/{org}/workspaces/{workspace}/environments/{environment}/gateways/{instance_uid}"
        ))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("delete {instance_uid}"), resp).await);
    }
    println!("deleting {instance_uid} …");
    mcpg_cli_core::stream::stream_phases(resp).await
}

#[derive(serde::Deserialize)]
struct VersionView {
    version: i64,
    content_sha256: String,
}

#[derive(serde::Deserialize)]
struct VersionRaw {
    version: i64,
    raw_config: String,
}

fn versions_base(cp_url: &str, org: &str, ws: &str, env: &str, name: &str) -> String {
    format!(
        "{cp_url}/v1/orgs/{org}/workspaces/{ws}/environments/{env}/gateways/{name}/config-versions"
    )
}

/// List the published config versions for an instance.
pub async fn versions(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
) -> anyhow::Result<()> {
    let list = fetch_versions(&client(state_dir).await?, cp_url, org, ws, env, name).await?;
    if list.is_empty() {
        println!("(no config versions for '{name}')");
        return Ok(());
    }
    println!("{:<8}  CONTENT-SHA256", "VERSION");
    for v in &list {
        println!("{:<8}  {}", v.version, v.content_sha256);
    }
    Ok(())
}

async fn fetch_versions(
    client: &reqwest::Client,
    cp_url: &str,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
) -> anyhow::Result<Vec<VersionView>> {
    let resp = client
        .get(versions_base(cp_url, org, ws, env, name))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("versions '{name}'"), resp).await);
    }
    Ok(resp.json().await?)
}

async fn fetch_version_raw(
    client: &reqwest::Client,
    cp_url: &str,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
    version: i64,
) -> anyhow::Result<VersionRaw> {
    let resp = client
        .get(format!(
            "{}/{version}",
            versions_base(cp_url, org, ws, env, name)
        ))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("version '{name}@{version}'"), resp).await);
    }
    Ok(resp.json().await?)
}

/// Show a line diff between two published config versions.
#[allow(clippy::too_many_arguments)]
pub async fn diff(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
    from: i64,
    to: i64,
) -> anyhow::Result<()> {
    let c = client(state_dir).await?;
    let a = fetch_version_raw(&c, cp_url, org, ws, env, name, from).await?;
    let b = fetch_version_raw(&c, cp_url, org, ws, env, name, to).await?;
    println!("--- {name} v{} \n+++ {name} v{}", a.version, b.version);
    print!("{}", line_diff(&a.raw_config, &b.raw_config));
    Ok(())
}

/// Roll an instance back to a prior config version by re-publishing it. The
/// re-publish reuses the instance's coords (in-place update) — same machinery
/// as `publish` of the same name.
#[allow(clippy::too_many_arguments)]
pub async fn rollback(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
    to: i64,
    replicas: u32,
    region: String,
    isolation_tier: String,
    size: String,
) -> anyhow::Result<()> {
    let raw = fetch_version_raw(&client(state_dir).await?, cp_url, org, ws, env, name, to)
        .await?
        .raw_config;
    let body = PublishBody {
        name: name.to_owned(),
        replicas,
        region,
        isolation_tier,
        size,
        custom_hostname: String::new(),
        config_toml: raw,
    };
    let resp = client(state_dir)
        .await?
        .post(format!(
            "{cp_url}/v1/orgs/{org}/workspaces/{ws}/environments/{env}/gateways"
        ))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("rollback '{name}'"), resp).await);
    }
    println!("rolling '{name}' back to config version {to} …");
    mcpg_cli_core::stream::stream_phases(resp).await
}

/// Re-publish the NEWEST config version in place, so the instance is
/// re-provisioned at the platform's current gateway release. The same
/// parameter caveat as `rollback` applies.
#[allow(clippy::too_many_arguments)]
pub async fn redeploy(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
    replicas: u32,
    region: String,
    isolation_tier: String,
    size: String,
) -> anyhow::Result<()> {
    let versions = fetch_versions(&client(state_dir).await?, cp_url, org, ws, env, name).await?;
    let Some(newest) = versions.iter().map(|v| v.version).max() else {
        anyhow::bail!("'{name}' has no published config versions to redeploy");
    };
    let raw = fetch_version_raw(
        &client(state_dir).await?,
        cp_url,
        org,
        ws,
        env,
        name,
        newest,
    )
    .await?
    .raw_config;
    let body = PublishBody {
        name: name.to_owned(),
        replicas,
        region,
        isolation_tier,
        size,
        custom_hostname: String::new(),
        config_toml: raw,
    };
    let resp = client(state_dir)
        .await?
        .post(format!(
            "{cp_url}/v1/orgs/{org}/workspaces/{ws}/environments/{env}/gateways"
        ))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("redeploy '{name}'"), resp).await);
    }
    println!("redeploying '{name}' at the platform's current release …");
    mcpg_cli_core::stream::stream_phases(resp).await
}

/// Minimal LCS-based line diff (`- ` removed, `+ ` added, `  ` unchanged). No
/// external diff dep; sufficient for a CLI config diff.
fn line_diff(from: &str, to: &str) -> String {
    let a: Vec<&str> = from.lines().collect();
    let b: Vec<&str> = to.lines().collect();
    let (n, m) = (a.len(), b.len());
    // dp[i][j] = LCS length of a[i..] and b[j..].
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push_str(&format!("  {}\n", a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str(&format!("- {}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        }
    }
    for line in &a[i..] {
        out.push_str(&format!("- {line}\n"));
    }
    for line in &b[j..] {
        out.push_str(&format!("+ {line}\n"));
    }
    out
}

// ───────────── custom domains ─────────────

#[derive(serde::Deserialize)]
struct DomainView {
    hostname: String,
    status: String,
    record_name: String,
    record_value: String,
}

fn print_challenge(d: &DomainView) {
    println!("domain:  {}  [{}]", d.hostname, d.status);
    if d.status != "verified" {
        println!("\nTo verify ownership, create this DNS TXT record:\n");
        println!("  name:  {}", d.record_name);
        println!("  value: {}", d.record_value);
        println!(
            "\nThen run: mcpg cloud domains verify --org <org> {}",
            d.hostname
        );
    }
}

/// Claim a hostname for the org; prints the TXT challenge to create.
pub async fn domain_add(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    hostname: &str,
) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .post(format!("{cp_url}/v1/orgs/{org}/domains"))
        .json(&serde_json::json!({ "hostname": hostname }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("claim domain '{hostname}'"), resp).await);
    }
    let d: DomainView = resp.json().await?;
    print_challenge(&d);
    Ok(())
}

/// Run the DNS TXT check now.
pub async fn domain_verify(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    hostname: &str,
) -> anyhow::Result<()> {
    #[derive(serde::Deserialize)]
    struct VerifyResp {
        hostname: String,
        matched: bool,
        #[serde(default)]
        found: Vec<String>,
        queried: String,
        #[serde(default)]
        instructions: Option<String>,
    }
    let resp = client(state_dir)
        .await?
        .post(format!("{cp_url}/v1/orgs/{org}/domains/{hostname}/verify"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("verify domain '{hostname}'"), resp).await);
    }
    let v: VerifyResp = resp.json().await?;
    if v.matched {
        println!(
            "✓ {} is verified — publish with --custom-hostname {}",
            v.hostname, v.hostname
        );
    } else {
        println!(
            "✗ {} is NOT verified yet (queried {})",
            v.hostname, v.queried
        );
        if v.found.is_empty() {
            println!("  no TXT records found at the challenge name");
        } else {
            println!("  TXT records found (none matched):");
            for r in &v.found {
                println!("    {r}");
            }
        }
        if let Some(i) = v.instructions {
            println!("  → {i}");
        }
        anyhow::bail!("domain '{}' not verified", v.hostname);
    }
    Ok(())
}

/// List the org's domain claims.
pub async fn domain_list(cp_url: &str, state_dir: &Path, org: &str) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .get(format!("{cp_url}/v1/orgs/{org}/domains"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error("list domains", resp).await);
    }
    let domains: Vec<DomainView> = resp.json().await?;
    if domains.is_empty() {
        println!("(no custom domains claimed — `mcpg cloud domains add --org {org} <hostname>`)");
        return Ok(());
    }
    println!("{:<42}  {:<10}  CHALLENGE RECORD", "HOSTNAME", "STATUS");
    for d in &domains {
        let challenge = if d.status == "verified" {
            String::new()
        } else {
            format!("{} TXT {}", d.record_name, d.record_value)
        };
        println!("{:<42}  {:<10}  {challenge}", d.hostname, d.status);
    }
    Ok(())
}

/// Release a claim.
pub async fn domain_remove(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    hostname: &str,
) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .delete(format!("{cp_url}/v1/orgs/{org}/domains/{hostname}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("remove domain '{hostname}'"), resp).await);
    }
    println!("✓ released {hostname}");
    Ok(())
}

#[derive(serde::Deserialize)]
struct CreatedToken {
    id: String,
    name: String,
    token: String,
    #[serde(default)]
    org_roles: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(serde::Deserialize)]
struct TokenView {
    id: String,
    name: String,
    #[serde(default)]
    org_roles: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    last_used_at: Option<String>,
    active: bool,
}

/// Mint a service token. The plaintext is shown ONCE — there's no way to read it
/// back (only its hash is stored), so we print it prominently.
pub async fn service_token_create(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    name: &str,
    roles: &[String],
    expires_days: i64,
) -> anyhow::Result<()> {
    let mut body = serde_json::json!({ "name": name, "expires_days": expires_days });
    if !roles.is_empty() {
        body["roles"] = serde_json::json!(roles);
    }
    let resp = client(state_dir)
        .await?
        .post(format!("{cp_url}/v1/orgs/{org}/service-tokens"))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("create service token '{name}'"), resp).await);
    }
    let t: CreatedToken = resp.json().await?;
    println!("✓ created service token '{}' ({})", t.name, t.id);
    println!("  roles:   {}", t.org_roles.join(", "));
    println!("  expires: {}", t.expires_at.as_deref().unwrap_or("never"));
    println!();
    println!("  Store this token now — it will NOT be shown again:");
    println!();
    println!("    {}", t.token);
    println!();
    println!("  Use it as: Authorization: Bearer {}", t.token);
    Ok(())
}

/// List the org's service tokens (never shows secrets).
pub async fn service_token_list(cp_url: &str, state_dir: &Path, org: &str) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .get(format!("{cp_url}/v1/orgs/{org}/service-tokens"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error("list service tokens", resp).await);
    }
    let tokens: Vec<TokenView> = resp.json().await?;
    if tokens.is_empty() {
        println!("(no service tokens — `mcpg cloud service-token create --org {org} <name>`)");
        return Ok(());
    }
    println!(
        "{:<38}  {:<16}  {:<8}  {:<20}  {:<20}  ID",
        "NAME", "ROLES", "ACTIVE", "EXPIRES", "LAST USED"
    );
    for t in &tokens {
        println!(
            "{:<38}  {:<16}  {:<8}  {:<20}  {:<20}  {}",
            t.name,
            t.org_roles.join(","),
            if t.active { "yes" } else { "no" },
            t.expires_at.as_deref().unwrap_or("never"),
            t.last_used_at.as_deref().unwrap_or("never"),
            t.id,
        );
    }
    Ok(())
}

/// Revoke a service token by id.
pub async fn service_token_revoke(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    id: &str,
) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .delete(format!("{cp_url}/v1/orgs/{org}/service-tokens/{id}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error(&format!("revoke service token '{id}'"), resp).await);
    }
    println!("✓ revoked service token {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::line_diff;

    #[test]
    fn diff_marks_added_removed_unchanged() {
        let d = line_diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert!(d.contains("  a\n"), "{d}");
        assert!(d.contains("- b\n"), "{d}");
        assert!(d.contains("+ B\n"), "{d}");
        assert!(d.contains("  c\n"), "{d}");
        assert!(d.contains("+ d\n"), "{d}");
    }

    #[test]
    fn identical_is_all_context() {
        let d = line_diff("x\ny\n", "x\ny\n");
        assert_eq!(d, "  x\n  y\n");
    }
}

#[derive(serde::Deserialize)]
struct LogLineView {
    at: Option<chrono::DateTime<chrono::Utc>>,
    level: String,
    target: String,
    message: String,
    plugin_id: String,
}

fn print_log_line(v: &LogLineView) {
    let ts = v.at.map(|t| t.to_rfc3339()).unwrap_or_else(|| "-".into());
    let plugin = if v.plugin_id.is_empty() {
        String::new()
    } else {
        format!(" [{}]", v.plugin_id)
    };
    println!(
        "{ts}  {:<5}  {}{}  {}",
        v.level, v.target, plugin, v.message
    );
}

/// Show an instance's recent gateway logs. Default: a tail snapshot. `--follow`:
/// stream live lines (SSE) until interrupted.
pub async fn logs(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    ws: &str,
    env: &str,
    name: &str,
    follow: bool,
) -> anyhow::Result<()> {
    let url =
        format!("{cp_url}/v1/orgs/{org}/workspaces/{ws}/environments/{env}/gateways/{name}/logs");
    let c = client(state_dir).await?;
    if follow {
        let resp = c.get(format!("{url}?follow=true")).send().await?;
        if !resp.status().is_success() {
            return Err(cp_error(&format!("logs '{name}'"), resp).await);
        }
        stream_log_sse(resp).await
    } else {
        let resp = c.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(cp_error(&format!("logs '{name}'"), resp).await);
        }
        let lines: Vec<LogLineView> = resp.json().await?;
        if lines.is_empty() {
            println!("(no recent logs for '{name}' on this control-plane replica)");
        }
        for l in &lines {
            print_log_line(l);
        }
        Ok(())
    }
}

/// Drain a logs SSE stream, printing each `data:` line as a rendered log line.
async fn stream_log_sse(resp: reqwest::Response) -> anyhow::Result<()> {
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
        while let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
            let block = buf[..pos].to_vec();
            buf.drain(..pos + 2);
            if let Ok(s) = std::str::from_utf8(&block) {
                for line in s.lines() {
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(v) = serde_json::from_str::<LogLineView>(data)
                    {
                        print_log_line(&v);
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct OpView {
    kind: String,
    final_phase: String,
    error: String,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List the tenant's recent provisioning operations.
pub async fn list(cp_url: &str, state_dir: &Path, org: &str) -> anyhow::Result<()> {
    let resp = client(state_dir)
        .await?
        .get(format!("{cp_url}/v1/orgs/{org}/operations"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error("list", resp).await);
    }
    let ops: Vec<OpView> = resp.json().await?;
    if ops.is_empty() {
        println!("(no instances published in org {org} yet)");
        return Ok(());
    }
    println!("{:<16}  {:<12}  {:<24}", "KIND", "PHASE", "STARTED");
    for op in &ops {
        let started = op
            .started_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "?".into());
        println!("{:<16}  {:<12}  {:<24}", op.kind, op.final_phase, started);
        if !op.error.is_empty() {
            println!("    error: {}", op.error);
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct OrgListView {
    slug: String,
    name: String,
    #[serde(default)]
    plan_tier: String,
    #[serde(default)]
    status: String,
}

/// `mcpg cloud whoami` — which orgs this login can act on (and therefore
/// what to pass as `--org`). For a generic-IdP tenant the slug is the
/// DERIVED `<prefix>-<12hex>` form, which is otherwise hard to discover.
pub async fn whoami(cp_url: &str, state_dir: &Path) -> anyhow::Result<()> {
    let has_creds = bearer_token(state_dir).is_some();
    println!(
        "credentials: {}",
        if has_creds {
            "id_token present (from `mcpg cloud login`)"
        } else {
            "none — loopback CP only (run `mcpg cloud login --issuer <url>` for managed)"
        }
    );
    let resp = client(state_dir)
        .await?
        .get(format!("{cp_url}/v1/orgs"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error("whoami", resp).await);
    }
    let orgs: Vec<OrgListView> = resp.json().await?;
    if orgs.is_empty() {
        println!("orgs: none — your first publish/login onboards one automatically");
        return Ok(());
    }
    println!("orgs ({} — pass the slug as --org):", orgs.len());
    for o in &orgs {
        let status = if o.status.is_empty() || o.status == "active" {
            String::new()
        } else {
            format!("  [{}]", o.status)
        };
        println!("  {:<42}  {}  ({}){status}", o.slug, o.plan_tier, o.name);
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct InstanceView {
    instance_uid: String,
    state: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    update_available: Option<String>,
    #[serde(default)]
    addressable: Vec<String>,
    #[serde(default)]
    last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn fetch_instances(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
) -> anyhow::Result<Vec<InstanceView>> {
    let resp = client(state_dir)
        .await?
        .get(format!("{cp_url}/v1/orgs/{org}/instances"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(cp_error("instances", resp).await);
    }
    Ok(resp.json().await?)
}

/// `mcpg cloud instances` — the running gateways with the two things a user
/// actually needs: the endpoint URL and the uid (for delete/logs).
pub async fn instances(cp_url: &str, state_dir: &Path, org: &str) -> anyhow::Result<()> {
    let list = fetch_instances(cp_url, state_dir, org).await?;
    if list.is_empty() {
        println!("(no instances in org {org} — `mcpg cloud publish` to create one)");
        return Ok(());
    }
    println!(
        "{:<38}  {:<10}  {:<18}  ENDPOINT",
        "INSTANCE_UID", "STATE", "VERSION"
    );
    for i in &list {
        let ep = i.addressable.first().map(String::as_str).unwrap_or("-");
        let version = match &i.update_available {
            Some(target) => format!("{} → {target}", i.version),
            None => i.version.clone(),
        };
        println!(
            "{:<38}  {:<10}  {:<18}  {}",
            i.instance_uid, i.state, version, ep
        );
        if i.update_available.is_some() {
            println!(
                "{:<38}  {:<10}  update available — `mcpg cloud redeploy <name>` to adopt",
                "", ""
            );
        }
        if let Some(seen) = i.last_seen_at {
            println!("{:<38}  {:<10}  last seen {}", "", "", seen.to_rfc3339());
        }
    }
    Ok(())
}

/// Resolve a published gateway NAME to its instance_uid by matching the
/// addressable endpoint's host label (`https://<name>.<zone>/mcp`). Names
/// are globally-unique slugs, so at most one instance matches.
pub async fn resolve_name(
    cp_url: &str,
    state_dir: &Path,
    org: &str,
    name: &str,
) -> anyhow::Result<String> {
    let list = fetch_instances(cp_url, state_dir, org).await?;
    match_name(&list, name).map_err(|e| anyhow::anyhow!("{e} (org {org})"))
}

/// Pure name→instance matcher over the instances list. Names are
/// globally-unique slugs, so at most one instance should match.
fn match_name(list: &[InstanceView], name: &str) -> Result<String, String> {
    let needle = format!("{name}.");
    let matched: Vec<&InstanceView> = list
        .iter()
        .filter(|i| {
            i.addressable.iter().any(|ep| {
                url::Url::parse(ep)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.starts_with(&needle)))
                    .unwrap_or(false)
            })
        })
        .collect();
    match matched.as_slice() {
        [one] => Ok(one.instance_uid.clone()),
        [] => Err(format!(
            "no instance named `{name}` found — names resolve via the published \
             endpoint, so an instance that never reached READY must be deleted by \
             uid (`mcpg cloud instances` lists uids)"
        )),
        many => Err(format!(
            "{} instances match `{name}` — delete by uid instead (`mcpg cloud instances`)",
            many.len()
        )),
    }
}

#[cfg(test)]
mod name_resolution_tests {
    use super::*;

    fn iv(uid: &str, eps: &[&str]) -> InstanceView {
        InstanceView {
            instance_uid: uid.into(),
            state: "online".into(),
            version: "0.1.0-beta.23".into(),
            update_available: None,
            addressable: eps.iter().map(|s| s.to_string()).collect(),
            last_seen_at: None,
        }
    }

    #[test]
    fn matches_exactly_one_by_endpoint_host_label() {
        let list = vec![
            iv("uid-a", &["https://edge-one.mcpg.cloud/mcp"]),
            iv("uid-b", &["https://edge-two.mcpg.cloud/mcp"]),
        ];
        assert_eq!(match_name(&list, "edge-one").unwrap(), "uid-a");
        // `edge` must NOT prefix-match `edge-one` (label boundary is the dot).
        assert!(match_name(&list, "edge").is_err());
    }

    #[test]
    fn unaddressable_instances_resolve_to_a_helpful_error() {
        let list = vec![iv("uid-a", &[])];
        let e = match_name(&list, "edge-one").unwrap_err();
        assert!(e.contains("never reached READY"), "{e}");
    }
}
