use clap::Parser;
use monoplan_cli::commands::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs go to stderr: stdout is command output (`--json` is parsed),
    // and in `daemon --detach` it is the readiness pipe.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    Cli::parse().run().await
}
