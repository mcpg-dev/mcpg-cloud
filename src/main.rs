//! `mcpg-cloud` — the tenant CLI for mcpg.cloud (the managed service).
//!
//! Everything a tenant does from a terminal: sign in (OIDC PKCE via the
//! federation), publish a config as a managed gateway, inspect instances /
//! operations / logs, manage config versions (diff / rollback), and prove
//! ownership of custom domains.
//!
//! Authenticates with the OIDC id_token stored by `login`
//! (`<state_dir>/credentials.json`), attached as `Authorization: Bearer`.
//! Against a loopback CP (`auth_mode=none`) no login is needed.
//!
//! Coordinates (`--org/--workspace/--env`) resolve flag > env var
//! (`MCPG_ORG`/`MCPG_WORKSPACE`/`MCPG_ENV`) > the context stored by
//! `mcpg cloud use` > an error with a hint — so after `use`, commands take
//! just their primary noun: `mcpg cloud publish edge --config gw.yaml`.
//!
//! Reached as `mcpg cloud …` through the gateway's front-door dispatch, or
//! invoked directly as `mcpg-cloud …`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

mod cloud;

pub(crate) use mcpg_cli_core::{context, login, paths};

#[derive(Parser, Debug)]
#[command(
    name = "mcpg-cloud",
    version,
    about = "mcpg.cloud tenant CLI — publish configs as managed MCP gateways",
    long_about = "Tenant CLI for mcpg.cloud (the managed service): login, publish a \
                  config as a managed gateway, inspect instances/operations/logs, \
                  manage config versions, and verify custom domains."
)]
struct Cli {
    /// Override default state dir (defaults to ~/.mcpg).
    #[arg(long, env = "MCPG_STATE_DIR", global = true)]
    state_dir: Option<PathBuf>,

    /// Emit logs as JSON (default: pretty).
    #[arg(long, env = "MCPG_JSON_LOGS", global = true)]
    json_logs: bool,

    /// Control-plane HTTP base URL.
    #[arg(
        long,
        alias = "control-plane-url",
        env = "MCPG_CP_URL",
        default_value = "http://127.0.0.1:7843",
        global = true
    )]
    cp_url: String,

    /// Org slug. Defaults from MCPG_ORG, then the `use` context.
    #[arg(long, env = "MCPG_ORG", global = true)]
    org: Option<String>,

    /// Workspace. Defaults from MCPG_WORKSPACE, then the `use` context.
    #[arg(long, env = "MCPG_WORKSPACE", global = true)]
    workspace: Option<String>,

    /// Environment. Defaults from MCPG_ENV, then the `use` context.
    #[arg(long, env = "MCPG_ENV", global = true)]
    env: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Sign in via OIDC PKCE. Stores the resulting tokens + license to
    /// `<state_dir>/credentials.json`; every other command attaches them
    /// as a Bearer token automatically.
    Login {
        /// OIDC issuer URL — typically `https://auth.mcpg.dev`
        /// for mcpg.cloud, or any RFC-conformant provider.
        #[arg(long, env = "MCPG_FED_ISSUER")]
        issuer: String,
        /// OAuth client id. Defaults to the registered public client id.
        #[arg(long, default_value = "mcpg-ctl")]
        client_id: String,
        /// Skip opening the browser; print the authorize URL
        /// instead. Useful for headless servers / SSH sessions.
        #[arg(long)]
        no_browser: bool,
    },

    /// Clear stored credentials.
    Logout,

    /// Who am I: the orgs this login can act on (pass a slug as --org).
    Whoami,

    /// Set the default org/workspace/env context for every other command
    /// (stored in the state dir). With no flags, prints the current context.
    /// Flags and MCPG_ORG/MCPG_WORKSPACE/MCPG_ENV still override it.
    Use,

    /// Publish a config → create or update the named instance. Re-publishing
    /// the same NAME updates it in place (no duplicate). Streams the phase
    /// ladder.
    Publish {
        /// Gateway name (globally-unique slug; becomes `<NAME>.<zone>`).
        name: String,
        /// Path to the config file to publish (validated server-side).
        #[arg(long)]
        config: Option<String>,
        /// Pin the gateway image tag. Omitted, the platform's default
        /// gateway version applies.
        #[arg(long)]
        image_tag: Option<String>,
        #[arg(long, default_value_t = 1)]
        replicas: u32,
        #[arg(long, default_value = "")]
        region: String,
        #[arg(long, default_value = "")]
        isolation_tier: String,
        /// Instance size class. Your plan caps the maximum
        /// (community: s, pro: m, team: l, enterprise: xl).
        #[arg(long, default_value = "s", value_parser = ["s", "m", "l", "xl"])]
        size: String,
        #[arg(long, default_value = "")]
        custom_hostname: String,
    },

    /// Recent provisioning operations for the org.
    Operations,

    /// Tear down an instance, by published NAME or by instance UID
    /// (UUID-shaped targets are treated as uids; anything else resolves as a
    /// name via the published endpoint).
    Delete {
        /// NAME or instance UID.
        target: String,
    },

    /// Running gateways with their endpoint URLs + instance uids.
    Instances,

    /// List the published config versions for an instance.
    Versions {
        /// Gateway name.
        name: String,
    },

    /// Diff two published config versions of an instance.
    Diff {
        /// Gateway name.
        name: String,
        #[arg(long)]
        from: i64,
        #[arg(long)]
        to: i64,
    },

    /// Show an instance's recent gateway logs (tail snapshot, or stream with
    /// --follow).
    Logs {
        /// Gateway name.
        name: String,
        #[arg(long)]
        follow: bool,
    },

    /// Roll an instance back to a prior config version (re-publishes it
    /// in place).
    ///
    /// NOTE: the deployment parameters below do NOT yet default from the
    /// target version's record — omitting them resets to these defaults
    /// (e.g. replicas back to 1). Pass them explicitly if the instance
    /// deviates.
    Rollback {
        /// Gateway name.
        name: String,
        #[arg(long)]
        to: i64,
        /// Pin the gateway image tag. Omitted, the platform's default
        /// gateway version applies.
        #[arg(long)]
        image_tag: Option<String>,
        #[arg(long, default_value_t = 1)]
        replicas: u32,
        #[arg(long, default_value = "")]
        region: String,
        #[arg(long, default_value = "")]
        isolation_tier: String,
        /// Instance size class. Your plan caps the maximum
        /// (community: s, pro: m, team: l, enterprise: xl).
        #[arg(long, default_value = "s", value_parser = ["s", "m", "l", "xl"])]
        size: String,
    },

    /// Custom domains: prove DNS ownership of a hostname before publishing
    /// with `--custom-hostname` (the CP refuses unverified domains).
    Domains {
        #[command(subcommand)]
        cmd: DomainsCmd,
    },

    /// Service tokens: non-interactive machine credentials for CI / automation
    /// (an `Authorization: Bearer mcpgst_…` that isn't a human's OIDC login).
    #[command(alias = "service-tokens")]
    ServiceToken {
        #[command(subcommand)]
        cmd: ServiceTokenCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceTokenCmd {
    /// Mint a token (owner only). The secret is printed ONCE — store it now.
    Create {
        /// Human label, e.g. "ci-deploy".
        name: String,
        /// Role to grant (repeatable). Defaults to `member`.
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Term in days; `0` = never expires. Defaults to 90.
        #[arg(long, default_value_t = 90)]
        expires_days: i64,
    },
    /// List the org's tokens (no secrets are stored, so none are shown).
    List,
    /// Revoke a token by id (owner only).
    Revoke {
        /// Token id (from `list`).
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum DomainsCmd {
    /// Claim a hostname for the org — prints the DNS TXT record to create.
    Add {
        /// The fully-qualified domain, e.g. mcp.example.com.
        hostname: String,
    },
    /// Check the DNS TXT challenge now and mark the domain verified on match.
    Verify { hostname: String },
    /// List the org's domain claims and their statuses.
    List,
    /// Release a claim (also how a domain moves to another org).
    Remove { hostname: String },
}

/// Resolved org/workspace/env after flag > env > context precedence (clap
/// folds the env-var step into the flag value).
#[derive(Debug)]
struct Coords {
    org: String,
    workspace: String,
    env: String,
}

fn resolve_coord(
    flag: Option<String>,
    ctx_value: Option<&String>,
    name: &str,
    flag_name: &str,
    env_name: &str,
) -> anyhow::Result<String> {
    flag.or_else(|| ctx_value.cloned()).ok_or_else(|| {
        anyhow::anyhow!(
            "no {name} given — pass --{flag_name}, set {env_name}, or store a default \
             with `mcpg cloud use --{flag_name} <value>`"
        )
    })
}

fn resolve_org(flag: Option<String>, ctx: &context::Context) -> anyhow::Result<String> {
    resolve_coord(flag, ctx.org.as_ref(), "org", "org", "MCPG_ORG")
}

fn resolve_coords(
    org: Option<String>,
    workspace: Option<String>,
    env: Option<String>,
    ctx: &context::Context,
) -> anyhow::Result<Coords> {
    Ok(Coords {
        org: resolve_org(org, ctx)?,
        workspace: resolve_coord(
            workspace,
            ctx.workspace.as_ref(),
            "workspace",
            "workspace",
            "MCPG_WORKSPACE",
        )?,
        env: resolve_coord(env, ctx.env.as_ref(), "environment", "env", "MCPG_ENV")?,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.json_logs);

    let state_dir = cli
        .state_dir
        .clone()
        .unwrap_or_else(paths::default_state_dir);
    paths::ensure_dir(&state_dir)?;

    let cp_url = cli.cp_url;
    let ctx = context::load(&state_dir);

    match cli.command {
        Command::Login {
            issuer,
            client_id,
            no_browser,
        } => login::run(&state_dir, &issuer, &client_id, no_browser).await,
        Command::Logout => login::logout(&state_dir),
        Command::Whoami => cloud::whoami(&cp_url, &state_dir).await,
        Command::Use => {
            // The global --org/--workspace/--env flags double as the values
            // to store; with none given, just print what's in effect.
            if cli.org.is_none() && cli.workspace.is_none() && cli.env.is_none() {
                println!("context ({}):", state_dir.join("context.json").display());
                println!("  org:       {}", ctx.org.as_deref().unwrap_or("(unset)"));
                println!(
                    "  workspace: {}",
                    ctx.workspace.as_deref().unwrap_or("(unset)")
                );
                println!("  env:       {}", ctx.env.as_deref().unwrap_or("(unset)"));
                return Ok(());
            }
            let next = context::Context {
                org: cli.org.clone().or(ctx.org),
                workspace: cli.workspace.clone().or(ctx.workspace),
                env: cli.env.clone().or(ctx.env),
            };
            context::save(&state_dir, &next)?;
            println!(
                "✓ context set: org={} workspace={} env={}",
                next.org.as_deref().unwrap_or("(unset)"),
                next.workspace.as_deref().unwrap_or("(unset)"),
                next.env.as_deref().unwrap_or("(unset)"),
            );
            Ok(())
        }
        Command::Publish {
            name,
            config,
            image_tag,
            replicas,
            region,
            isolation_tier,
            size,
            custom_hostname,
        } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            cloud::publish(
                &cp_url,
                &state_dir,
                &c.org,
                &c.workspace,
                &c.env,
                cloud::PublishArgs {
                    name,
                    image_tag: image_tag.unwrap_or_default(),
                    replicas,
                    region,
                    isolation_tier,
                    size,
                    custom_hostname,
                    config_file: config,
                },
            )
            .await
        }
        Command::Operations => {
            let org = resolve_org(cli.org, &ctx)?;
            cloud::list(&cp_url, &state_dir, &org).await
        }
        Command::Delete { target } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            // UUID-shaped → it's an instance uid (uids are UUIDv7); anything
            // else is a published name (DNS-safe slug — never UUID-shaped).
            let uid = if uuid::Uuid::parse_str(&target).is_ok() {
                target
            } else {
                cloud::resolve_name(&cp_url, &state_dir, &c.org, &target).await?
            };
            cloud::delete(&cp_url, &state_dir, &c.org, &c.workspace, &c.env, &uid).await
        }
        Command::Instances => {
            let org = resolve_org(cli.org, &ctx)?;
            cloud::instances(&cp_url, &state_dir, &org).await
        }
        Command::Versions { name } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            cloud::versions(&cp_url, &state_dir, &c.org, &c.workspace, &c.env, &name).await
        }
        Command::Diff { name, from, to } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            cloud::diff(
                &cp_url,
                &state_dir,
                &c.org,
                &c.workspace,
                &c.env,
                &name,
                from,
                to,
            )
            .await
        }
        Command::Logs { name, follow } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            cloud::logs(
                &cp_url,
                &state_dir,
                &c.org,
                &c.workspace,
                &c.env,
                &name,
                follow,
            )
            .await
        }
        Command::Rollback {
            name,
            to,
            image_tag,
            replicas,
            region,
            isolation_tier,
            size,
        } => {
            let c = resolve_coords(cli.org, cli.workspace, cli.env, &ctx)?;
            cloud::rollback(
                &cp_url,
                &state_dir,
                &c.org,
                &c.workspace,
                &c.env,
                &name,
                to,
                image_tag.unwrap_or_default(),
                replicas,
                region,
                isolation_tier,
                size,
            )
            .await
        }
        Command::Domains { cmd } => {
            let org = resolve_org(cli.org, &ctx)?;
            match cmd {
                DomainsCmd::Add { hostname } => {
                    cloud::domain_add(&cp_url, &state_dir, &org, &hostname).await
                }
                DomainsCmd::Verify { hostname } => {
                    cloud::domain_verify(&cp_url, &state_dir, &org, &hostname).await
                }
                DomainsCmd::List => cloud::domain_list(&cp_url, &state_dir, &org).await,
                DomainsCmd::Remove { hostname } => {
                    cloud::domain_remove(&cp_url, &state_dir, &org, &hostname).await
                }
            }
        }
        Command::ServiceToken { cmd } => {
            let org = resolve_org(cli.org, &ctx)?;
            match cmd {
                ServiceTokenCmd::Create {
                    name,
                    roles,
                    expires_days,
                } => {
                    cloud::service_token_create(
                        &cp_url,
                        &state_dir,
                        &org,
                        &name,
                        &roles,
                        expires_days,
                    )
                    .await
                }
                ServiceTokenCmd::List => cloud::service_token_list(&cp_url, &state_dir, &org).await,
                ServiceTokenCmd::Revoke { id } => {
                    cloud::service_token_revoke(&cp_url, &state_dir, &org, &id).await
                }
            }
        }
    }
}

fn init_tracing(json: bool) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,mcpg_cloud=info"));
    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry.with(fmt::layer().json()).init();
    } else {
        registry
            .with(fmt::layer().with_target(true).with_level(true))
            .init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_tree_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn coordinate_precedence_flag_then_context_then_error() {
        let ctx = context::Context {
            org: Some("ctx-org".into()),
            workspace: None,
            env: Some("ctx-env".into()),
        };
        // Flag wins over context.
        assert_eq!(
            resolve_org(Some("flag-org".into()), &ctx).unwrap(),
            "flag-org"
        );
        // Context fills an absent flag.
        assert_eq!(resolve_org(None, &ctx).unwrap(), "ctx-org");
        // Nothing anywhere → error that teaches all three sources.
        let err = resolve_coords(None, None, None, &context::Context::default()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("--org") && msg.contains("MCPG_ORG") && msg.contains("use"),
            "{msg}"
        );
        // Partial context: org resolves, workspace errors.
        let err = resolve_coords(None, None, None, &ctx).unwrap_err();
        assert!(err.to_string().contains("workspace"), "{err}");
    }

    #[test]
    fn delete_target_discrimination_uuid_vs_name() {
        // The exact rule `Delete` applies: uids are UUIDs, names are DNS
        // slugs (which can never parse as a UUID).
        assert!(uuid::Uuid::parse_str("0190a6e2-4b1c-7def-8a3b-2c4d5e6f7a8b").is_ok());
        assert!(uuid::Uuid::parse_str("edge-cloud").is_err());
        assert!(uuid::Uuid::parse_str("edge-0190a6e2").is_err());
    }

    #[test]
    fn publish_size_accepts_the_ladder_and_rejects_the_rest() {
        for size in ["s", "m", "l", "xl"] {
            let cli = Cli::try_parse_from(["mcpg-cloud", "publish", "edge", "--size", size])
                .unwrap_or_else(|e| panic!("--size {size} must parse: {e}"));
            match cli.command {
                Command::Publish { size: got, .. } => assert_eq!(got, size),
                other => panic!("expected Publish, got {other:?}"),
            }
        }
        for bad in ["xxl", "S", "medium", ""] {
            assert!(
                Cli::try_parse_from(["mcpg-cloud", "publish", "edge", "--size", bad]).is_err(),
                "--size {bad:?} must be rejected"
            );
        }
        // Omitted → the `s` default.
        let cli = Cli::try_parse_from(["mcpg-cloud", "publish", "edge"]).unwrap();
        match cli.command {
            Command::Publish { size, .. } => assert_eq!(size, "s"),
            other => panic!("expected Publish, got {other:?}"),
        }
    }

    #[test]
    fn rollback_size_mirrors_publish() {
        let cli = Cli::try_parse_from([
            "mcpg-cloud",
            "rollback",
            "edge",
            "--to",
            "1",
            "--size",
            "xl",
        ])
        .unwrap();
        match cli.command {
            Command::Rollback { size, .. } => assert_eq!(size, "xl"),
            other => panic!("expected Rollback, got {other:?}"),
        }
        assert!(
            Cli::try_parse_from([
                "mcpg-cloud",
                "rollback",
                "edge",
                "--to",
                "1",
                "--size",
                "huge"
            ])
            .is_err()
        );
    }
}
