//! Owns listener runtime lifetime independently of the desktop UI.
use crate::{
    config::SharedConfig,
    devices::SharedDeviceRegistry,
    input_executor::InputExecutor,
    pairing::{NotifierContext, PairingCoordinator},
    run_server,
    runtime_status::{RuntimeStatus, SharedRuntimeStatus},
};
use std::{
    io,
    sync::{Arc, Mutex as StdMutex},
    thread,
};
use tokio::sync::oneshot;
use tracing::warn;

pub(crate) struct ServerThread {
    notifier: fn(NotifierContext),
    pub(crate) active: bool,
    pub(crate) input: InputExecutor,
    pub(crate) status: SharedRuntimeStatus,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
    /// The pairing coordinator this thread's server was constructed with, so
    /// the tray can mint pairing-session tokens that the *same* running
    /// server instance will actually recognize. Set to `None` after that
    /// server stops so the tray refuses to display an unusable pairing QR.
    pub(crate) pairing: Option<Arc<PairingCoordinator>>,
}

impl ServerThread {
    pub(crate) fn spawn(
        config: SharedConfig,
        registry: SharedDeviceRegistry,
        notifier: fn(NotifierContext),
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let input = InputExecutor::new().map_err(io::Error::other)?;
        let input_for_server = input.clone();
        let status = Arc::new(StdMutex::new(RuntimeStatus::default()));
        let status_for_server = status.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let pairing = Arc::new(PairingCoordinator::new(Some(Arc::new(notifier))));
        let pairing_for_server = pairing.clone();

        let join_handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;

            runtime
                .block_on(async move {
                    run_server(
                        config,
                        pairing_for_server,
                        registry,
                        false,
                        input_for_server,
                        status_for_server,
                        async move {
                            let _ = shutdown_rx.await;
                        },
                    )
                    .await
                })
                .map_err(|error| error.to_string())
        });

        Ok(Self {
            notifier,
            active: true,
            input,
            status,
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
            pairing: Some(pairing),
        })
    }

    pub(crate) fn shutdown(&mut self) {
        self.input.shutdown();
        if let Some(pairing) = &self.pairing {
            pairing.invalidate();
        }
        if !self.active {
            return;
        }
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }

    pub(crate) fn restart(
        &mut self,
        config: SharedConfig,
        registry: SharedDeviceRegistry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.active {
            if let Err(error) = self.finish() {
                warn!(%error, "previous WakeMATE server stopped with an error before restart");
            }
        }
        *self = Self::spawn(config, registry, self.notifier)?;
        Ok(())
    }

    /// Collects a server thread that ended without a tray shutdown request.
    /// This turns bind failures and other listener exits into an explicit tray
    /// error instead of silently treating an arbitrary TCP service as the
    /// WakeMATE server.
    pub(crate) fn take_unexpected_failure(&mut self) -> Option<String> {
        let finished = self
            .join_handle
            .as_ref()
            .is_some_and(|handle| handle.is_finished());
        if !self.active || !finished {
            return None;
        }

        self.active = false;
        self.shutdown_tx.take();
        if let Some(pairing) = &self.pairing {
            pairing.invalidate();
        }
        self.pairing = None;

        let result = match self.join_handle.take()?.join() {
            Ok(Ok(())) => "WakeMATE server stopped unexpectedly".to_string(),
            Ok(Err(error)) => error,
            Err(_) => "WakeMATE server thread panicked".to_string(),
        };
        Some(result)
    }

    pub(crate) fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.active {
            return Ok(());
        }
        self.shutdown();

        let result = if let Some(join_handle) = self.join_handle.take() {
            match join_handle.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(std::io::Error::other(error).into()),
                Err(_) => Err(std::io::Error::other("WakeMATE server thread panicked").into()),
            }
        } else {
            Ok(())
        };

        self.active = false;
        if let Some(pairing) = &self.pairing {
            pairing.invalidate();
        }
        self.pairing = None;
        result
    }
}
