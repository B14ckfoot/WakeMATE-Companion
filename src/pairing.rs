use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tracing::info;

use crate::config::SharedConfig;

/// Minimum time between desktop confirmation prompts. A device that already
/// holds a valid token can still hammer `/v1/pairing/activate`; this keeps
/// that from turning into a notification-spam nuisance (or a way to bury a
/// legitimate prompt under repeats) without needing a full session concept.
const PROMPT_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub struct PairingRequestInfo {
    pub peer: IpAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingOutcome {
    /// A confirmation prompt was shown (or a very recent one is still
    /// outstanding); the phone should treat this as "ask again shortly".
    AwaitingApproval,
    /// No desktop confirmation surface is available in this run mode (for
    /// example the pre-logon headless service, which has no interactive
    /// session to prompt). Pairing must be completed from the tray app.
    Unavailable,
}

type Notifier = dyn Fn(PairingRequestInfo, SharedConfig) + Send + Sync;

/// Bridges the HTTP pairing-activation handler (which must respond quickly)
/// to whatever platform-specific "Approve this device?" UI is available, if
/// any. Granting `allow_input_commands` / `allow_power_commands` happens
/// only inside the notifier's callback once a human approves -- never
/// synchronously inside the HTTP handler.
pub struct PairingCoordinator {
    notifier: Option<Arc<Notifier>>,
    last_prompt_at: Mutex<Option<Instant>>,
}

impl PairingCoordinator {
    pub fn new(notifier: Option<Arc<Notifier>>) -> Self {
        Self {
            notifier,
            last_prompt_at: Mutex::new(None),
        }
    }

    /// No desktop UI is available to confirm pairing (headless / pre-logon
    /// service mode, or a platform build without a tray yet).
    pub fn unavailable() -> Self {
        Self::new(None)
    }

    pub fn request(&self, info: PairingRequestInfo, config: SharedConfig) -> PairingOutcome {
        let Some(notifier) = self.notifier.clone() else {
            return PairingOutcome::Unavailable;
        };

        let mut last_prompt_at = self
            .last_prompt_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let now = Instant::now();
        let should_prompt = last_prompt_at
            .map(|previous| now.duration_since(previous) >= PROMPT_COOLDOWN)
            .unwrap_or(true);

        if should_prompt {
            *last_prompt_at = Some(now);
            drop(last_prompt_at);
            info!(peer = %info.peer, "WakeMATE showing a pairing confirmation prompt on the desktop");
            notifier(info, config);
        }

        PairingOutcome::AwaitingApproval
    }
}
