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
use std::net::SocketAddr;

use clap::{Args, Parser, Subcommand};
use tokuly_core::{Core, config::Config, telemetry};

mod app;
mod healthcheck;

#[derive(Parser)]
#[command(name = "tokuly", version, about = "Tokuly backend")]
struct Cli {
    #[command(flatten)]
    core: Config,
    #[command(flatten)]
    domains: app::DomainConfigs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the Connect RPC API.
    Api(ApiArgs),
    /// Run the realtime gateway.
    Realtime,
    /// Run background workers.
    Worker,
    /// Apply database migrations and prepare buckets, streams and indexes.
    Migrate,
    /// Run the API, the realtime gateway and the workers in one process (development).
    All(ApiArgs),
    /// Exit with 0 if a health endpoint of this container answers 200 (container health checks).
    Healthcheck {
        /// For example `http://127.0.0.1:8081/readyz`.
        url: String,
    },
}

#[derive(Args, Clone)]
struct ApiArgs {
    /// Address the API listens on.
    #[arg(long, env = "TOKULY_API_LISTEN", default_value = "0.0.0.0:8081")]
    listen: SocketAddr,
    /// Extra origins allowed to call the API from browsers (comma separated).
    /// The public URL is always allowed; in development so is any localhost port.
    #[arg(long, env = "TOKULY_CORS_ORIGINS", value_delimiter = ',', default_value = "")]
    cors_origins: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // The probe talks to a server that is already running: it needs none of the infrastructure.
    if let Command::Healthcheck { url } = &cli.command {
        return healthcheck::run(url).await;
    }
    let service = match &cli.command {
        Command::Api(_) => "tokuly-api",
        Command::Realtime => "tokuly-realtime",
        Command::Worker => "tokuly-worker",
        Command::Migrate => "tokuly-migrate",
        Command::All(_) => "tokuly-all",
        Command::Healthcheck { .. } => unreachable!("handled above"),
    };
    let _telemetry = telemetry::init(&cli.core, service);

    let core = Core::connect(cli.core).await?;
    spawn_shutdown_listener(core.clone());

    let domains = app::Domains::new(core.clone(), cli.domains)?;
    match cli.command {
        Command::Migrate => app::migrate(&core, &domains).await?,
        Command::Api(args) => app::serve_api(core, domains, args.listen, args.cors_origins).await?,
        Command::Realtime => domains.gateway.run().await?,
        Command::Worker => {
            app::spawn_workers(&core, &domains);
            core.shutdown.cancelled().await;
        }
        Command::All(args) => {
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
            app::serve_api(core, domains, args.listen, args.cors_origins).await?;
        }
        Command::Healthcheck { .. } => unreachable!("handled above"),
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
}
