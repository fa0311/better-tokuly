//! The `tokuly` binary.
//!
//! One image, several roles — pick one per container:
//!
//! - `tokuly api`       Connect RPC API (Connect, gRPC and gRPC-Web on one port)
//! - `tokuly realtime`  realtime gateway (WebTransport + WebSocket)
//! - `tokuly worker`    background work: outbox relay, event consumers, jobs
//! - `tokuly migrate`   database migrations and one-time infrastructure setup
//! - `tokuly all`       everything in one process (development)
//!
//! `tokuly healthcheck <url>` is the container health probe for these roles.
//!
//! Settings come from the command line, the `TOKULY_*` environment, the
//! `tokuly.toml` named by `TOKULY_CONFIG` and the staff console, in that
//! order (see `settings.rs`); `tokuly config example` prints every one.
use std::ffi::OsString;

use clap::{Parser, Subcommand};
use tokuly_core::{Core, config::Config, settings::Settings, telemetry};

mod app;
mod bridges;
mod healthcheck;
mod settings;

#[derive(Parser)]
#[command(name = "tokuly", version, about = "Tokuly backend")]
struct Cli {
    #[command(flatten)]
    core: Config,
    #[command(flatten)]
    api: settings::ApiConfig,
    #[command(flatten)]
    domains: app::DomainConfigs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the Connect RPC API.
    Api,
    /// Run the realtime gateway.
    Realtime,
    /// Run background workers.
    Worker,
    /// Apply database migrations and prepare buckets, streams and indexes.
    Migrate,
    /// Run the API, the realtime gateway and the workers in one process (development).
    All,
    /// Give an existing account the `admin` role (the operator's way back in,
    /// e.g. after the bootstrap admin lost access). Prints whether it changed.
    PromoteAdmin {
        /// Handle or e-mail address of the account.
        account: String,
    },
    /// Settings files: `example` prints every setting with its default (the
    /// source of `tokuly.example.toml`); `show` prints what the command line,
    /// the environment and `tokuly.toml` set (console values not included).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Exit with 0 if a health endpoint of this container answers 200 (container health checks).
    Healthcheck {
        /// For example `http://127.0.0.1:8081/readyz`.
        url: String,
    },
}

#[derive(Subcommand, Clone, Copy)]
enum ConfigAction {
    /// Print `tokuly.example.toml`: every setting, commented out at its default.
    Example,
    /// Check `tokuly.toml` and print the settings it, the environment and the command line set.
    Show,
}

/// What the first word of the command line asks for, read before the
/// settings are (some commands need none of them).
fn early_command(argv: &[OsString]) -> Option<Command> {
    let lenient = <Cli as clap::CommandFactory>::command().mut_args(|a| a.required(false)).ignore_errors(true);
    let matches = lenient.try_get_matches_from(argv).ok()?;
    match matches.subcommand() {
        Some(("healthcheck", sub)) => sub.get_one::<String>("url").map(|url| Command::Healthcheck { url: url.clone() }),
        Some(("config", sub)) => match sub.subcommand_name() {
            Some("example") => Some(Command::Config { action: ConfigAction::Example }),
            Some("show") => Some(Command::Config { action: ConfigAction::Show }),
            _ => None,
        },
        _ => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    match early_command(&argv) {
        // The probe talks to a server that is already running: it needs none of the infrastructure.
        Some(Command::Healthcheck { url }) => return healthcheck::run(&url).await,
        Some(Command::Config { action: ConfigAction::Example }) => {
            print!("{}", tokuly_core::settings::file::example(&settings::registry()?));
            return Ok(());
        }
        Some(Command::Config { action: ConfigAction::Show }) => {
            print!("{}", settings::Resolved::read(&argv)?.show());
            return Ok(());
        }
        _ => {}
    }
    let mut resolved = settings::Resolved::read(&argv)?;
    resolved.read_console().await?;
    let cli = resolved.parse(&argv);
    let service = match &cli.command {
        Command::Api => "tokuly-api",
        Command::Realtime => "tokuly-realtime",
        Command::Worker => "tokuly-worker",
        Command::Migrate => "tokuly-migrate",
        Command::All => "tokuly-all",
        Command::PromoteAdmin { .. } => "tokuly-admin",
        Command::Healthcheck { .. } | Command::Config { .. } => unreachable!("handled above"),
    };
    let _telemetry = telemetry::init(&cli.core, service);
    if let Some(file) = &resolved.file {
        tracing::info!(file = %file.display(), "settings file read");
    }
    let settings = Settings::new(resolved.into_state(cli.core.is_production()));

    let core = Core::connect_with(cli.core, settings).await?;
    spawn_shutdown_listener(core.clone());

    let domains = app::Domains::new(core.clone(), cli.domains)?;
    match cli.command {
        Command::Migrate => app::migrate(&core, &domains).await?,
        Command::PromoteAdmin { account } => {
            let granted = domains.admin.ensure_admin(&account).await.map_err(|e| anyhow::anyhow!("promote-admin: {e}"))?;
            if granted {
                println!("{account} is an admin now (from its next sign-in or token refresh)");
            } else {
                anyhow::bail!("{account}: no such account, or it already is an admin");
            }
        }
        Command::Api => app::serve_api(core, domains, cli.api.listen, cli.api.cors_origins).await?,
        Command::Realtime => domains.gateway.run().await?,
        Command::Worker => {
            app::spawn_workers(&core, &domains);
            core.shutdown.cancelled().await;
        }
        Command::All => {
            app::migrate(&core, &domains).await?;
            app::spawn_workers(&core, &domains);
            let gateway = domains.gateway.clone();
            let shutdown = core.shutdown.clone();
            tokio::spawn(async move {
                if let Err(e) = gateway.run().await {
                    tracing::error!(error = %e, "realtime gateway stopped");
                    shutdown.cancel();
                }
            });
            app::serve_api(core, domains, cli.api.listen, cli.api.cors_origins).await?;
        }
        Command::Healthcheck { .. } | Command::Config { .. } => unreachable!("handled above"),
    }
    Ok(())
}

/// Cancels `core.shutdown` on Ctrl-C or SIGTERM so servers and workers stop cleanly.
fn spawn_shutdown_listener(core: Core) {
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
        tracing::info!("shutdown requested");
        core.shutdown.cancel();
    });
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::Cli;

    /// Domain configs are flattened into one command line; this catches two
    /// domains using the same argument name before the binary ever starts.
    #[test]
    fn command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    /// Without a file or console values, the layered parse reads exactly
    /// what clap alone read before (an environment-only deployment).
    #[test]
    fn without_file_or_console_nothing_changes() {
        let argv: Vec<std::ffi::OsString> =
            ["tokuly", "--secret", "0123456789abcdef0123456789abcdef", "--max-video-hours", "4", "api"].iter().map(Into::into).collect();
        let plain = Cli::command().try_get_matches_from(&argv).unwrap();
        let resolved = crate::settings::Resolved {
            registry: std::sync::Arc::new(crate::settings::registry().unwrap()),
            layers: Default::default(),
            file: None,
        };
        let layered = tokuly_core::settings::apply(Cli::command(), &resolved.registry, &resolved.layers, false).try_get_matches_from(&argv).unwrap();
        for def in resolved.registry.defs() {
            let raw = |m: &clap::ArgMatches| m.get_raw(&def.id).map(|v| v.map(|s| s.to_owned()).collect::<Vec<_>>());
            assert_eq!(raw(&plain), raw(&layered), "{}", def.env);
            assert_eq!(plain.value_source(&def.id), layered.value_source(&def.id), "{}", def.env);
        }
    }
}
