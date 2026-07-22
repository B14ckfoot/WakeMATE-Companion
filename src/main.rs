#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod discovery;
mod error;
mod input;
mod system;
#[cfg(target_os = "windows")]
mod tray;
mod types;

use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use tokio::net::TcpListener;
#[cfg(not(target_os = "windows"))]
use tokio::signal;
#[cfg(not(target_os = "windows"))]
use tracing::error;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{
    app::{router, AppState},
    config::{AppConfig, SharedConfig},
};

const PREPARE_INSTALL_CONFIG_ARG: &str = "--prepare-install-config";
const HEADLESS_SERVER_ARG: &str = "--headless-server";
const CONFIG_PATH_ARG: &str = "--config-path";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = config_path_from_args(&args)?.unwrap_or(AppConfig::path()?);

    if args.iter().any(|arg| arg == PREPARE_INSTALL_CONFIG_ARG) {
        let config = AppConfig::prepare_install_config_at_path(&config_path)?;
        if let Err(error) = system::sync_launch_on_startup(config.launch_on_startup) {
            warn!(%error, "failed to sync the Windows startup preference during install prep");
        }
        #[cfg(target_os = "windows")]
        if let Err(error) =
            system::sync_prelogon_server_task(config.launch_on_startup, &config_path)
        {
            warn!(
                %error,
                path = %config_path.display(),
                "failed to sync the pre-logon WakeMATE server task during install prep"
            );
        }
        info!(
            path = %config_path.display(),
            bind = %config.effective_bind_address(),
            discovery_port = config.discovery_port,
            remote_access = config.allow_remote_connections,
            discovery_enabled = config.discovery_enabled(),
            launch_on_startup = config.launch_on_startup,
            "WakeMATE install config prepared"
        );
        return Ok(());
    }

    if args.iter().any(|arg| arg == HEADLESS_SERVER_ARG) {
        return run_headless_server(config_path);
    }

    let config = Arc::new(Mutex::new(AppConfig::load_or_create_from_path(
        &config_path,
    )?));
    let snapshot = config_snapshot(&config)?;
    let bind_address = snapshot.effective_bind_address();

    #[cfg(target_os = "windows")]
    {
        if system::enable_preferred_dark_mode() {
            info!("WakeMATE enabled the Windows dark tray theme");
        }
    }

    if let Err(error) = system::sync_launch_on_startup(snapshot.launch_on_startup) {
        warn!(%error, "failed to sync the Windows startup preference");
    }

    info!(
        path = %config_path.display(),
        bind = %bind_address,
        discovery_port = snapshot.discovery_port,
        remote_access = snapshot.allow_remote_connections,
        discovery_enabled = snapshot.discovery_enabled(),
        device_name = %snapshot.device_name,
        launch_on_startup = snapshot.launch_on_startup,
        "WakeMATE companion starting"
    );

    #[cfg(target_os = "windows")]
    {
        tray::run(config)?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(run_server(config, shutdown_signal()))?;
        Ok(())
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();
}

pub(crate) async fn run_server<F>(
    config: SharedConfig,
    shutdown: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Future<Output = ()> + Send + 'static,
{
    let snapshot = config_snapshot(&config)?;
    let bind_address = snapshot.effective_bind_address();
    let listener = TcpListener::bind(&bind_address).await?;
    info!(bind = %bind_address, "WakeMATE HTTP listener ready");

    let discovery_task = if snapshot.discovery_enabled() {
        Some(tokio::spawn(discovery::run(config.clone())))
    } else {
        info!("WakeMATE UDP discovery disabled until remote access is enabled");
        None
    };

    axum::serve(listener, router(AppState::new(config)))
        .with_graceful_shutdown(shutdown)
        .await?;

    if let Some(discovery_task) = discovery_task {
        discovery_task.abort();
        let _ = discovery_task.await;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = signal::ctrl_c().await {
            error!(%error, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => {
                error!(%error, "failed to install terminate signal handler");
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received");
}

fn config_snapshot(config: &SharedConfig) -> Result<AppConfig, Box<dyn std::error::Error>> {
    config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| std::io::Error::other("failed to access application config").into())
}

fn config_path_from_args(
    args: &[String],
) -> Result<Option<std::path::PathBuf>, Box<dyn std::error::Error>> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == CONFIG_PATH_ARG {
            let Some(path) = args.get(index + 1) else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "--config-path requires a path value",
                )
                .into());
            };
            return Ok(Some(std::path::PathBuf::from(path)));
        }
        index += 1;
    }

    Ok(None)
}

fn run_headless_server(config_path: std::path::PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Mutex::new(AppConfig::load_or_create_from_path(
        &config_path,
    )?));
    let snapshot = config_snapshot(&config)?;

    info!(
        path = %config_path.display(),
        bind = %snapshot.effective_bind_address(),
        discovery_port = snapshot.discovery_port,
        remote_access = snapshot.allow_remote_connections,
        discovery_enabled = snapshot.discovery_enabled(),
        device_name = %snapshot.device_name,
        "WakeMATE headless server starting"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_server(config, std::future::pending::<()>()))?;
    Ok(())
}
