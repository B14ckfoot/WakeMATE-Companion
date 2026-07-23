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

/// Where the most recent pairing-activation prompt stands, so the phone can
/// poll `/v1/pairing/status` and show a truthful pending/approved/denied
/// state instead of assuming success. A denial is per-prompt, not permanent:
/// the next activation request shows a fresh prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalState {
    #[default]
    Idle,
    Pending,
    Approved,
    Denied,
}

impl ApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalState::Idle => "idle",
            ApprovalState::Pending => "pending",
            ApprovalState::Approved => "approved",
            ApprovalState::Denied => "denied",
        }
    }
}

/// Shared between the coordinator (which marks a prompt as pending), the
/// platform notifier (which records the human's Yes/No), and the HTTP status
/// endpoint (which reports it).
#[derive(Default)]
pub struct PairingStatusHandle {
    state: Mutex<ApprovalState>,
}

impl PairingStatusHandle {
    pub fn get(&self) -> ApprovalState {
        *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn set(&self, state: ApprovalState) {
        *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = state;
    }

    pub fn record_outcome(&self, approved: bool) {
        self.set(if approved {
            ApprovalState::Approved
        } else {
            ApprovalState::Denied
        });
    }
}

type Notifier = dyn Fn(PairingRequestInfo, SharedConfig, Arc<PairingStatusHandle>) + Send + Sync;

/// Bridges the HTTP pairing-activation handler (which must respond quickly)
/// to whatever platform-specific "Approve this device?" UI is available, if
/// any. Granting `allow_input_commands` / `allow_power_commands` happens
/// only inside the notifier's callback once a human approves -- never
/// synchronously inside the HTTP handler.
pub struct PairingCoordinator {
    notifier: Option<Arc<Notifier>>,
    status: Arc<PairingStatusHandle>,
    last_prompt_at: Mutex<Option<Instant>>,
}

impl PairingCoordinator {
    pub fn new(notifier: Option<Arc<Notifier>>) -> Self {
        Self {
            notifier,
            status: Arc::new(PairingStatusHandle::default()),
            last_prompt_at: Mutex::new(None),
        }
    }

    /// No desktop UI is available to confirm pairing (headless / pre-logon
    /// service mode, or a platform build without a tray yet).
    pub fn unavailable() -> Self {
        Self::new(None)
    }

    pub fn approval_state(&self) -> ApprovalState {
        self.status.get()
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
            self.status.set(ApprovalState::Pending);
            info!(peer = %info.peer, "WakeMATE showing a pairing confirmation prompt on the desktop");
            notifier(info, config, self.status.clone());
        }

        PairingOutcome::AwaitingApproval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn shared_config() -> SharedConfig {
        Arc::new(Mutex::new(AppConfig::default()))
    }

    #[test]
    fn unavailable_coordinator_reports_idle_and_refuses_requests() {
        let coordinator = PairingCoordinator::unavailable();
        assert_eq!(coordinator.approval_state(), ApprovalState::Idle);

        let outcome = coordinator.request(
            PairingRequestInfo {
                peer: "127.0.0.1".parse().unwrap(),
            },
            shared_config(),
        );
        assert_eq!(outcome, PairingOutcome::Unavailable);
    }

    #[test]
    fn request_marks_pending_and_notifier_outcome_is_reported() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(
            |_info, _config, status: Arc<PairingStatusHandle>| {
                // The real notifier spawns a thread and records the human's
                // answer later; recording synchronously here exercises the
                // same path.
                assert_eq!(status.get(), ApprovalState::Pending);
                status.record_outcome(true);
            },
        )));

        let outcome = coordinator.request(
            PairingRequestInfo {
                peer: "192.168.1.20".parse().unwrap(),
            },
            shared_config(),
        );

        assert_eq!(outcome, PairingOutcome::AwaitingApproval);
        assert_eq!(coordinator.approval_state(), ApprovalState::Approved);
    }

    #[test]
    fn denial_is_reported_until_the_next_prompt() {
        let handle = PairingStatusHandle::default();
        handle.set(ApprovalState::Pending);
        handle.record_outcome(false);
        assert_eq!(handle.get(), ApprovalState::Denied);
        assert_eq!(handle.get().as_str(), "denied");
    }
}
