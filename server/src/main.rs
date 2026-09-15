use std::io::{self, IsTerminal};
use std::net::SocketAddr;
use std::path::PathBuf;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHasher, SaltString};
use clap::{Parser, Subcommand};
use monoplan_server::{AppState, Config, build_info, router};

#[derive(Parser, Debug)]
#[command(name = "monoplan-server", version, about = "Monoplan relay server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to a TOML config file. Defaults to ./config.toml; missing file → defaults.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to the sqlite database file. Overrides the config file.
    #[arg(long)]
    db: Option<PathBuf>,

    /// Address to bind. Overrides the config file.
    #[arg(long)]
    bind: Option<SocketAddr>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Read one password line from stdin and print an Argon2id PHC hash.
    HashAdminPassword,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::HashAdminPassword)) {
        return hash_admin_password();
    }
    let (mut cfg, cfg_source) = Config::load(cli.config.as_deref());

    if let Some(db) = cli.db {
        cfg.db = db;
    }
    if let Some(bind) = cli.bind {
        cfg.bind = bind.to_string();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cfg.log_level)),
        )
        .init();

    cfg_source.log();

    if let Some(parent) = cfg.db.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut state = AppState::open(&cfg.db)
        .await?
        .with_secure_cookies(cfg.secure_cookies)
        .with_snapshot_threshold_blobs(cfg.snapshot_threshold_blobs);
    if let Some(password_hash) = cfg.admin_password_hash.as_deref() {
        state = state.with_admin_password_hash(password_hash)?;
    }
    let app = router(state);

    let addr = cfg.bind_addr()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        addr = %listener.local_addr()?,
        build.git_sha = build_info::GIT_SHA,
        "monoplan-server listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn hash_admin_password() -> anyhow::Result<()> {
    if io::stdin().is_terminal() {
        anyhow::bail!(
            "refusing to read an echoed password from a terminal; pipe one password line on stdin"
        );
    }

    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    while matches!(password.as_bytes().last(), Some(b'\n' | b'\r')) {
        password.pop();
    }
    if password.is_empty() {
        anyhow::bail!("admin password must not be empty");
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash admin password: {e}"))?;
    println!("{hash}");
    Ok(())
}
