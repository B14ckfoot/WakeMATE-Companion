#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
#[cfg(target_os = "windows")]
mod desktop;
mod devices;
mod discovery;
mod error;
mod input;
mod input_executor;
mod network;
mod pairing;
mod pairing_qr;
mod request_limits;
mod runtime_status;
mod secure_attention;
mod security;
#[cfg(target_os = "windows")]
mod server_thread;
mod system;
mod theme;
mod tls;
#[cfg(target_os = "windows")]
mod tray;
mod types;

use std::{
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use tokio::net::TcpListener;
#[cfg(not(target_os = "windows"))]
use tokio::signal;
use tokio::sync::watch;
#[cfg(not(target_os = "windows"))]
use tracing::error;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{
    app::{router, AppState},
    config::{AppConfig, SharedConfig},
    devices::SharedDeviceRegistry,
    pairing::PairingCoordinator,
    tls::TlsIdentity,
};

#[cfg(not(target_os = "windows"))]
use crate::devices::DeviceRegistry;

const PREPARE_INSTALL_CONFIG_ARG: &str = "--prepare-install-config";
const HEADLESS_SERVER_ARG: &str = "--headless-server";
const CONFIG_PATH_ARG: &str = "--config-path";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let result = start();
    #[cfg(target_os = "windows")]
    if let Err(error) = &result {
        system::show_error_dialog("WakeMATE could not start", &error.to_string());
    }
    result
}

fn start() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    tls::install_crypto_provider();

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == HEADLESS_SERVER_ARG) {
        // Versions before 0.2.3 installed an ONSTART task under SYSTEM. Its
        // credential store, TLS identity, and paired-device registry were a
        // different security principal's state, and the process retained the
        // tray's ports after logon. Keep accepting the legacy argument only
        // to retire stale tasks safely; never touch user state or bind a
        // server in that context.
        warn!(
            "WakeMATE pre-logon server mode is disabled; direct phone Wake-on-LAN remains available"
        );
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    if args.iter().any(|arg| arg == "--request-exit") {
        system::request_exit_for_current_executable().map_err(std::io::Error::other)?;
        return Ok(());
    }
    system::ensure_normal_user_runtime().map_err(std::io::Error::other)?;

    #[cfg(target_os = "windows")]
    let _single_instance_lock = match system::acquire_single_instance_lock() {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            info!("WakeMATE is already running; exiting this second instance");
            return Ok(());
        }
        Err(error) => return Err(std::io::Error::other(error).into()),
    };

    let config_path = match config_path_from_args(&args)? {
        Some(path) => path,
        None => AppConfig::path()?,
    };

    if args.iter().any(|arg| arg == PREPARE_INSTALL_CONFIG_ARG) {
        let config = AppConfig::prepare_install_config_at_path(&config_path)?;
        if let Err(error) = system::sync_launch_on_startup(config.launch_on_startup) {
            warn!(%error, "failed to sync the Windows startup preference during install prep");
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

    if args.iter().any(|arg| arg == "--installed") && !config_path.try_exists()? {
        AppConfig::prepare_install_config_at_path(&config_path)?;
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

        // No desktop confirmation surface exists on this platform yet, so
        // pairing activation is refused rather than silently auto-approved.
        let pairing = Arc::new(PairingCoordinator::unavailable());
        let registry: SharedDeviceRegistry =
            Arc::new(Mutex::new(DeviceRegistry::load_or_default()));
        runtime.block_on(run_server(
            config,
            pairing,
            registry,
            true,
            input_executor::InputExecutor::new().map_err(std::io::Error::other)?,
            Arc::new(Mutex::new(runtime_status::RuntimeStatus::default())),
            shutdown_signal(),
        ))?;
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
    pairing: Arc<PairingCoordinator>,
    registry: SharedDeviceRegistry,
    headless: bool,
    input: input_executor::InputExecutor,
    status: runtime_status::SharedRuntimeStatus,
    shutdown: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Future<Output = ()> + Send + 'static,
{
    let snapshot = config_snapshot(&config)?;
    let tls_identity = TlsIdentity::load_or_create()?;
    let tls_fingerprint = tls_identity.fingerprint().to_string();
    let tls_bind_address = snapshot.effective_tls_bind_address();
    let tls_socket_address: SocketAddr = tls_bind_address.parse()?;
    let rustls_config = tls_identity.rustls_config().await?;
    // Bind before advertising readiness. Keep every owned task and input worker
    // under this supervisor, including all failure paths.
    let tls_listener = std::net::TcpListener::bind(tls_socket_address)?;
    tls_listener.set_nonblocking(true)?;
    let tls_server = axum_server::from_tcp_rustls(tls_listener, rustls_config)?;
    let http_listener = if snapshot.allow_insecure_http {
        Some(TcpListener::bind(snapshot.effective_bind_address()).await?)
    } else {
        None
    };
    let discovery_socket = if snapshot.discovery_enabled() {
        match discovery::bind(&config).await {
            Ok(socket) => Some(socket),
            Err(error) => {
                status.lock().unwrap_or_else(|p| p.into_inner()).error =
                    Some(format!("HTTPS ready, discovery unavailable: {error}"));
                None
            }
        }
    } else {
        None
    };
    {
        let mut current = status.lock().unwrap_or_else(|p| p.into_inner());
        current.https_ready = true;
        current.discovery_ready = discovery_socket.is_some();
    }
    info!(bind = %tls_bind_address, "WakeMATE HTTPS listener bound");
    let discovery_config = config.clone();
    let discovery_task = tokio::spawn(async move {
        match discovery_socket {
            Some(socket) => discovery::run(discovery_config, tls_fingerprint, socket).await,
            None => std::future::pending::<std::io::Result<()>>().await,
        }
    });
    let state = AppState::with_input(config, pairing, registry, headless, input.clone());
    let tls_handle = axum_server::Handle::new();
    let shutdown_tls = tls_handle.clone();
    let (http_shutdown_tx, http_shutdown_rx) = watch::channel(false);
    let stop_input = input.clone();
    let shutdown_task = tokio::spawn(async move {
        shutdown.await;
        stop_input.shutdown();
        shutdown_tls.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
        let _ = http_shutdown_tx.send(true);
    });
    let tls_server = tls_server
        .handle(tls_handle.clone())
        .serve(router(state.clone()).into_make_service_with_connect_info::<SocketAddr>());
    let http_server = async move {
        match http_listener {
            Some(listener) => {
                axum::serve(
                    listener,
                    app::public_router(state).into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(wait_for_shutdown(http_shutdown_rx))
                .await
            }
            None => std::future::pending::<std::io::Result<()>>().await,
        }
    };
    let mut discovery_task = discovery_task;
    let result = tokio::select! {
        result = tls_server => result,
        result = http_server => result,
        result = &mut discovery_task => match result {
            Ok(result) => result,
            Err(error) => Err(std::io::Error::other(error)),
        },
    };
    input.shutdown();
    tls_handle.shutdown();
    discovery_task.abort();
    shutdown_task.abort();
    let _ = shutdown_task.await;
    let mut current = status.lock().unwrap_or_else(|p| p.into_inner());
    current.https_ready = false;
    current.discovery_ready = false;
    if let Err(error) = &result {
        current.error = Some(error.to_string());
    }
    result.map_err(Into::into)
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
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
            if !args.iter().any(|arg| arg == PREPARE_INSTALL_CONFIG_ARG) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "--config-path is supported only with --prepare-install-config",
                )
                .into());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_config_path_is_only_accepted_for_installer_preparation() {
        let args = vec![CONFIG_PATH_ARG.to_string(), "custom.json".to_string()];
        assert!(config_path_from_args(&args).is_err());
        let mut install_args = vec![PREPARE_INSTALL_CONFIG_ARG.to_string()];
        install_args.extend(args);
        assert_eq!(
            config_path_from_args(&install_args).unwrap(),
            Some("custom.json".into())
        );
        assert!(config_path_from_args(&[
            PREPARE_INSTALL_CONFIG_ARG.into(),
            CONFIG_PATH_ARG.into()
        ])
        .is_err());
    }
}
