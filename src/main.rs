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
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{
    app::{router, AppState},
    config::{AppConfig, SharedConfig},
};

const PREPARE_INSTALL_CONFIG_ARG: &str = "--prepare-install-config";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    if std::env::args()
        .skip(1)
        .any(|arg| arg == PREPARE_INSTALL_CONFIG_ARG)
    {
        let config_path = AppConfig::path()?;
        let config = AppConfig::prepare_install_config()?;
        info!(
            path = %config_path.display(),
            bind = %config.effective_bind_address(),
            discovery_port = config.discovery_port,
            remote_access = config.allow_remote_connections,
            discovery_enabled = config.discovery_enabled(),
            "WakeMATE install config prepared"
        );
        return Ok(());
    }

    let config_path = AppConfig::path()?;
    let config = Arc::new(Mutex::new(AppConfig::load_or_create()?));
    let snapshot = config_snapshot(&config)?;
    let bind_address = snapshot.effective_bind_address();

    info!(
        path = %config_path.display(),
        bind = %bind_address,
        discovery_port = snapshot.discovery_port,
        remote_access = snapshot.allow_remote_connections,
        discovery_enabled = snapshot.discovery_enabled(),
        device_name = %snapshot.device_name,
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
