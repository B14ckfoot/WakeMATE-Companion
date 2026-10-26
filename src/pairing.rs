use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tracing::info;

use crate::{
    config::SharedConfig,
    devices::{Permissions, SharedDeviceRegistry},
};

/// Minimum time between desktop confirmation prompts. A device that already
/// holds a valid token can still hammer `/v1/pairing/activate`; this keeps
/// that from turning into a notification-spam nuisance (or a way to bury a
/// legitimate prompt under repeats) without needing a full session concept.
const PROMPT_COOLDOWN: Duration = Duration::from_secs(10);

/// How long a pending enrollment stays claimable. If nobody answers the
/// desktop prompt within this window, approving a *later* prompt must not
/// silently register the stale device.
const ENROLLMENT_TTL: Duration = Duration::from_secs(120);

/// How long a one-time pairing-session token (the credential embedded in the
/// structured pairing QR code, as opposed to the permanent shared `api_token`)
/// stays claimable after being minted.
const PAIRING_SESSION_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
pub struct PairingRequestInfo {
    pub peer: IpAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingOutcome {
    /// A confirmation prompt was shown (or a very recent one is still
    /// outstanding); the phone should treat this as "ask again shortly".
    AwaitingApproval,
    /// An enrollment was requested while another prompt is still on screen
    /// (or inside the prompt cooldown). The phone should retry shortly;
    /// nothing was queued, so the visible prompt cannot grant the wrong
    /// device.
    Busy,
    /// No desktop confirmation surface is available in this run mode (for
    /// example the macOS headless developer preview). Pairing must be
    /// completed from an interactive tray app.
    Unavailable,
    /// The one-time QR credential was replaced, expired, or consumed between
    /// authentication and reserving a new desktop prompt. No prompt was shown.
    InvalidSession,
}

/// A device that has requested enrollment and is waiting for the person at
/// the desktop to answer the prompt. The token was already returned to the
/// phone; it becomes valid only if this record is committed to the registry
/// by an explicit "Yes".
#[derive(Debug, Clone)]
pub struct PendingEnrollment {
    pub device_id: String,
    pub device_name: String,
    pub token: String,
    pub expires_at: Instant,
}

impl PendingEnrollment {
    pub fn new(device_name: String) -> Self {
        Self {
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name,
            token: uuid::Uuid::new_v4().to_string(),
            expires_at: Instant::now() + ENROLLMENT_TTL,
        }
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Holds at most one unanswered enrollment. Shared between the coordinator
/// (which fills it when a prompt is shown), the platform notifier (which
/// takes it on "Yes" / clears it on "No"), and the status endpoint (which
/// reports whether a given device id is still pending).
#[derive(Default)]
pub struct EnrollmentSlot {
    slot: Mutex<Option<PendingEnrollment>>,
}

impl EnrollmentSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PendingEnrollment>> {
        self.slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn set(&self, pending: PendingEnrollment) {
        *self.lock() = Some(pending);
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// Takes the pending enrollment if it has not expired. An expired entry
    /// is discarded either way.
    #[cfg(test)]
    pub fn take_valid(&self) -> Option<PendingEnrollment> {
        let pending = self.lock().take()?;
        if pending.expired() {
            None
        } else {
            Some(pending)
        }
    }

    /// Commits a valid enrollment without an observable gap where it is in
    /// neither the pending slot nor the paired-device registry. Status reads
    /// hold the registry lock through their enrollment check, so they see
    /// either pending (before registration) or approved (after it), never a
    /// false denial between.
    #[cfg(test)]
    pub fn commit_valid(
        &self,
        registry: &SharedDeviceRegistry,
    ) -> std::io::Result<Option<PendingEnrollment>> {
        self.commit_with_permissions(registry, Permissions::default())
    }

    pub fn commit_with_permissions(
        &self,
        registry: &SharedDeviceRegistry,
        permissions: Permissions,
    ) -> std::io::Result<Option<PendingEnrollment>> {
        let mut registry = registry.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut slot = self.lock();
        let Some(pending) = slot.as_ref().filter(|pending| !pending.expired()).cloned() else {
            *slot = None;
            return Ok(None);
        };
        registry.register_with_permissions(
            &pending.device_id,
            &pending.device_name,
            &pending.token,
            permissions,
        )?;
        *slot = None;
        Ok(Some(pending))
    }

    /// Device name for the desktop prompt, if an enrollment is waiting.
    pub fn pending_device_name(&self) -> Option<String> {
        let guard = self.lock();
        guard
            .as_ref()
            .filter(|pending| !pending.expired())
            .map(|pending| pending.device_name.clone())
    }

    /// Whether the given device id is the one currently awaiting approval.
    pub fn holds(&self, device_id: &str) -> bool {
        let guard = self.lock();
        guard
            .as_ref()
            .filter(|pending| !pending.expired())
            .map(|pending| pending.device_id == device_id)
            .unwrap_or(false)
    }
}

/// A one-time credential embedded in the structured pairing QR code instead
/// of the permanent shared `api_token`, so a photo of the QR code cannot be
/// replayed indefinitely to enroll new devices. Read-only checks (`matches`,
/// used by `/v1/pairing/status`) never expire it early; only a state-changing
/// pairing action (`try_consume`, used by `/v1/pairing/enroll` and
/// `/v1/pairing/activate`) burns it, so a phone can still poll for approval
/// after the token that started the enrollment has already been consumed.
struct PairingSession {
    token: String,
    expires_at: Instant,
    consumed: bool,
    device_id: Option<String>,
}

impl PairingSession {
    fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Holds at most one live pairing-session token. Minted fresh each time the
/// desktop shows a new pairing QR code (see `tray.rs`); a stale token from a
/// previous QR display stops matching as soon as a new one is minted.
#[derive(Default)]
pub struct PairingSessionSlot {
    slot: Mutex<Option<PairingSession>>,
}

impl PairingSessionSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PairingSession>> {
        self.slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Mints a fresh one-time token, replacing any previous one.
    pub fn mint(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        *self.lock() = Some(PairingSession {
            token: token.clone(),
            expires_at: Instant::now() + PAIRING_SESSION_TTL,
            consumed: false,
            device_id: None,
        });
        token
    }

    /// Non-consuming check: true while the current token is unexpired,
    /// regardless of whether it has already been used to start an
    /// enrollment. Used for `/v1/pairing/status`, which a phone polls
    /// repeatedly with the same token while waiting for desktop approval.
    pub fn matches(&self, candidate: &str) -> bool {
        let guard = self.lock();
        guard
            .as_ref()
            .filter(|session| !session.expired())
            .map(|session| crate::security::tokens_match(candidate, &session.token))
            .unwrap_or(false)
    }

    /// Non-consuming preflight for a state-changing pairing request. Unlike
    /// `matches`, a token that already started a prompt is no longer
    /// claimable. The actual burn remains atomic in `try_consume`.
    pub fn claimable(&self, candidate: &str) -> bool {
        let guard = self.lock();
        guard
            .as_ref()
            .filter(|session| !session.consumed && !session.expired())
            .map(|session| crate::security::tokens_match(candidate, &session.token))
            .unwrap_or(false)
    }

    /// Consumes the token for a state-changing pairing action. Returns
    /// `true` at most once per minted token, so a leaked QR code or link
    /// cannot be replayed to enroll unlimited devices after the first use.
    #[cfg(test)]
    pub fn try_consume(&self, candidate: &str) -> bool {
        self.try_consume_for(candidate, None)
    }

    fn try_consume_for(&self, candidate: &str, device_id: Option<&str>) -> bool {
        let mut guard = self.lock();
        let Some(session) = guard.as_mut() else {
            return false;
        };
        if session.consumed || session.expired() {
            return false;
        }
        if !crate::security::tokens_match(candidate, &session.token) {
            return false;
        }
        session.consumed = true;
        session.device_id = device_id.map(str::to_owned);
        true
    }

    fn enrolled_device(&self, candidate: &str) -> Option<String> {
        self.lock()
            .as_ref()
            .filter(|s| !s.expired() && crate::security::tokens_match(candidate, &s.token))
            .and_then(|s| s.device_id.clone())
    }
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

/// Everything the platform-specific approval UI needs: the request origin,
/// live config (to persist granted capabilities), the status handle (to
/// record the human's answer), the enrollment slot (to commit or discard the
/// pending device), and the registry (where a committed device lands).
pub struct NotifierContext {
    pub info: PairingRequestInfo,
    pub status: Arc<PairingStatusHandle>,
    pub enrollment: Arc<EnrollmentSlot>,
    pub registry: SharedDeviceRegistry,
    lifecycle: Arc<Mutex<bool>>,
    active_prompt: Arc<Mutex<Option<(uuid::Uuid, Instant)>>>,
    prompt_id: uuid::Uuid,
    expires_at: Instant,
    is_enrollment: bool,
}

impl NotifierContext {
    /// Resolve a desktop prompt under the same gate used by reset/restart.
    /// A stale window can still be dismissed, but can no longer grant access.
    pub fn is_active(&self) -> bool {
        let cancelled = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        !*cancelled
            && Instant::now() < self.expires_at
            && self
                .active_prompt
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .is_some_and(|(id, _)| *id == self.prompt_id)
    }

    pub fn complete_permissions(&self, permissions: Option<Permissions>) -> Result<bool, String> {
        let cancelled = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        let mut active = self.active_prompt.lock().unwrap_or_else(|p| p.into_inner());
        if *cancelled || active.as_ref().is_none_or(|(id, _)| *id != self.prompt_id) {
            return Ok(false);
        }
        if permissions.is_none()
            || Instant::now() >= self.expires_at
            || (self.is_enrollment && self.enrollment.pending_device_name().is_none())
        {
            self.enrollment.clear();
            self.status.record_outcome(false);
            *active = None;
            return Ok(false);
        }
        // Grant and token hash share a single atomic registry write. Approval
        // never edits global config and cannot elevate other phones.
        let result = if self.is_enrollment {
            self.enrollment
                .commit_with_permissions(&self.registry, permissions.unwrap_or_default())
                .map_err(|e| e.to_string())
                .and_then(|pending| {
                    pending
                        .map(|_| ())
                        .ok_or_else(|| "Pairing request expired".into())
                })
        } else {
            Err("Legacy activation is retired; scan a new pairing QR code".into())
        };
        self.enrollment.clear();
        self.status.record_outcome(result.is_ok());
        *active = None;
        result.map(|()| true)
    }
}

type Notifier = dyn Fn(NotifierContext) + Send + Sync;

/// Bridges the HTTP pairing-activation handler (which must respond quickly)
/// to whatever platform-specific "Approve this device?" UI is available, if
/// any. Granting `allow_input_commands` / `allow_power_commands` happens
/// only inside the notifier's callback once a human approves -- never
/// synchronously inside the HTTP handler.
pub struct PairingCoordinator {
    notifier: Option<Arc<Notifier>>,
    status: Arc<PairingStatusHandle>,
    enrollment: Arc<EnrollmentSlot>,
    session: Arc<PairingSessionSlot>,
    last_prompt_at: Mutex<Option<Instant>>,
    lifecycle: Arc<Mutex<bool>>,
    active_prompt: Arc<Mutex<Option<(uuid::Uuid, Instant)>>>,
}

impl PairingCoordinator {
    pub fn new(notifier: Option<Arc<Notifier>>) -> Self {
        Self {
            notifier,
            status: Arc::new(PairingStatusHandle::default()),
            enrollment: Arc::new(EnrollmentSlot::default()),
            session: Arc::new(PairingSessionSlot::default()),
            last_prompt_at: Mutex::new(None),
            lifecycle: Arc::new(Mutex::new(false)),
            active_prompt: Arc::new(Mutex::new(None)),
        }
    }

    /// Mints a fresh one-time pairing-session token for a newly displayed QR
    /// code, replacing whatever token (if any) was minted for a previous QR
    /// display.
    pub fn mint_pairing_session_token(&self) -> String {
        let _cancelled = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        self.session.mint()
    }

    pub fn invalidate(&self) {
        let mut cancelled = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        *cancelled = true;
        *self.session.lock() = None;
        self.enrollment.clear();
        self.status.record_outcome(false);
        *self.active_prompt.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Cancel invitations/prompts without retiring the running coordinator.
    /// Used before pause/revoke/permission changes so an old dialog cannot
    /// add authority after the local user's security decision.
    pub fn cancel_pending(&self) {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        *self.session.lock() = None;
        self.enrollment.clear();
        self.status.record_outcome(false);
        *self.active_prompt.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Non-consuming check for the current pairing-session token. See
    /// [`PairingSessionSlot::matches`].
    pub fn pairing_session_matches(&self, candidate: &str) -> bool {
        self.session.matches(candidate)
    }

    /// Non-consuming check used while authenticating a state-changing
    /// pairing request. Consumption is deferred until `request_with_session`
    /// has established that a new desktop prompt can actually be shown.
    pub fn pairing_session_claimable(&self, candidate: &str) -> bool {
        self.session.claimable(candidate)
    }

    pub fn session_device_id(&self, candidate: &str) -> Option<String> {
        self.session.enrolled_device(candidate)
    }

    /// Consumes the current pairing-session token for a state-changing
    /// pairing action. See [`PairingSessionSlot::try_consume`].
    #[cfg(test)]
    pub fn consume_pairing_session_token(&self, candidate: &str) -> bool {
        self.session.try_consume(candidate)
    }

    /// No desktop UI is available to confirm pairing (for example, a platform
    /// build without a tray yet).
    #[cfg(any(not(target_os = "windows"), test))]
    pub fn unavailable() -> Self {
        Self::new(None)
    }

    pub fn approval_state(&self) -> ApprovalState {
        self.expire_pending();
        self.status.get()
    }

    fn expire_pending(&self) {
        let mut active = self.active_prompt.lock().unwrap_or_else(|p| p.into_inner());
        if active
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
        {
            self.enrollment.clear();
            self.status.record_outcome(false);
            *active = None;
        }
    }

    /// Whether the given device id is the enrollment currently awaiting the
    /// desktop's answer.
    pub fn enrollment_pending_for(&self, device_id: &str) -> bool {
        self.enrollment.holds(device_id)
    }

    /// Shows the desktop approval prompt. With `pending: Some(...)` this is
    /// a per-device enrollment; the pending record is queued only when a
    /// prompt is actually shown, so an on-screen prompt can never grant a
    /// device the person was not asked about.
    pub fn request(
        &self,
        info: PairingRequestInfo,
        config: SharedConfig,
        registry: SharedDeviceRegistry,
        pending: Option<PendingEnrollment>,
    ) -> PairingOutcome {
        self.request_with_session(info, config, registry, pending, None)
    }

    /// Like `request`, but atomically burns a one-time QR credential only
    /// after availability and prompt-cooldown checks pass. A Busy or
    /// Unavailable response therefore remains retryable with the same QR.
    pub fn request_with_session(
        &self,
        info: PairingRequestInfo,
        _config: SharedConfig,
        registry: SharedDeviceRegistry,
        pending: Option<PendingEnrollment>,
        pairing_session_token: Option<&str>,
    ) -> PairingOutcome {
        let Some(notifier) = self.notifier.clone() else {
            return PairingOutcome::Unavailable;
        };
        let cancelled = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        if *cancelled {
            return PairingOutcome::Unavailable;
        }
        if registry.lock().map(|r| r.paused()).unwrap_or(true) {
            return PairingOutcome::Unavailable;
        }

        self.expire_pending();

        // A desktop dialog can outlive the notification-spam cooldown. Never
        // replace its shared enrollment slot while it is unanswered: the old
        // dialog's Yes callback would otherwise register a newer device that
        // the user was never shown.
        if self.status.get() == ApprovalState::Pending {
            return if pending.is_some() {
                PairingOutcome::Busy
            } else {
                PairingOutcome::AwaitingApproval
            };
        }

        let mut last_prompt_at = self
            .last_prompt_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let now = Instant::now();
        let should_prompt = last_prompt_at
            .map(|previous| now.duration_since(previous) >= PROMPT_COOLDOWN)
            .unwrap_or(true);

        if !should_prompt {
            return if pending.is_some() {
                PairingOutcome::Busy
            } else {
                PairingOutcome::AwaitingApproval
            };
        }

        if pairing_session_token
            .map(|token| {
                !self
                    .session
                    .try_consume_for(token, pending.as_ref().map(|p| p.device_id.as_str()))
            })
            .unwrap_or(false)
        {
            return PairingOutcome::InvalidSession;
        }

        *last_prompt_at = Some(now);
        drop(last_prompt_at);

        let is_enrollment = pending.is_some();
        if let Some(pending) = pending {
            self.enrollment.set(pending);
        }

        self.status.set(ApprovalState::Pending);
        let prompt_id = uuid::Uuid::new_v4();
        *self.active_prompt.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((prompt_id, now + ENROLLMENT_TTL));
        info!(peer = %info.peer, "WakeMATE showing a pairing confirmation prompt on the desktop");
        let context = NotifierContext {
            info,
            status: self.status.clone(),
            enrollment: self.enrollment.clone(),
            registry,
            lifecycle: self.lifecycle.clone(),
            active_prompt: self.active_prompt.clone(),
            prompt_id,
            expires_at: now + ENROLLMENT_TTL,
            is_enrollment,
        };
        drop(cancelled);
        notifier(context);

        PairingOutcome::AwaitingApproval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, devices::DeviceRegistry};

    #[test]
    fn pause_rejects_open_approval_and_cancel_does_not_retire_coordinator() {
        let (coordinator, context, token) = pending_context();
        context.registry.lock().unwrap().set_paused(true).unwrap();
        assert!(context
            .complete_permissions(Some(Permissions::preset(4)))
            .is_err());
        assert_eq!(coordinator.approval_state(), ApprovalState::Denied);
        context.registry.lock().unwrap().set_paused(false).unwrap();
        assert_eq!(context.registry.lock().unwrap().authenticate(&token), None);
        let invitation = coordinator.mint_pairing_session_token();
        coordinator.cancel_pending();
        assert!(!coordinator.pairing_session_matches(&invitation));
        *coordinator.last_prompt_at.lock().unwrap() = None;
        let fresh = coordinator.mint_pairing_session_token();
        assert_eq!(
            coordinator.request_with_session(
                request_info(),
                shared_config(),
                shared_registry(),
                Some(PendingEnrollment::new("New Phone".into())),
                Some(&fresh)
            ),
            PairingOutcome::AwaitingApproval
        );
        assert!(!context
            .complete_permissions(Some(Permissions::preset(4)))
            .unwrap());
        assert_eq!(coordinator.approval_state(), ApprovalState::Pending);
    }

    fn shared_config() -> SharedConfig {
        Arc::new(Mutex::new(AppConfig::default()))
    }

    fn shared_registry() -> SharedDeviceRegistry {
        Arc::new(Mutex::new(DeviceRegistry::default()))
    }

    fn request_info() -> PairingRequestInfo {
        PairingRequestInfo {
            peer: "192.168.1.20".parse().unwrap(),
        }
    }

    fn pending_context() -> (PairingCoordinator, NotifierContext, String) {
        let captured = Arc::new(Mutex::new(None));
        let sink = captured.clone();
        let coordinator = PairingCoordinator::new(Some(Arc::new(move |context| {
            *sink.lock().unwrap() = Some(context);
        })));
        let pending = PendingEnrollment::new("Phone".into());
        let token = pending.token.clone();
        coordinator.request(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(pending),
        );
        let context = captured.lock().unwrap().take().unwrap();
        (coordinator, context, token)
    }

    #[test]
    fn invalidation_cancels_qr_pending_callbacks_and_future_requests() {
        let (coordinator, context, token) = pending_context();
        let qr = coordinator.mint_pairing_session_token();
        coordinator.invalidate();
        assert!(!coordinator.pairing_session_matches(&qr));
        assert!(!context
            .complete_permissions(Some(Permissions::preset(4)))
            .unwrap());
        assert_eq!(context.registry.lock().unwrap().authenticate(&token), None);
        assert_eq!(
            coordinator.request(request_info(), shared_config(), shared_registry(), None),
            PairingOutcome::Unavailable
        );
    }

    #[test]
    fn expired_dialog_never_grants_capabilities_or_registers_device() {
        let (coordinator, mut context, token) = pending_context();
        context.expires_at = Instant::now();
        assert!(!context
            .complete_permissions(Some(Permissions::preset(4)))
            .unwrap());
        assert_eq!(coordinator.approval_state(), ApprovalState::Denied);
        assert_eq!(context.registry.lock().unwrap().authenticate(&token), None);
    }

    #[test]
    fn approval_grants_only_selected_permissions_in_one_record() {
        let (coordinator, context, token) = pending_context();
        assert!(context
            .complete_permissions(Some(Permissions::default()))
            .unwrap());
        assert_eq!(coordinator.approval_state(), ApprovalState::Approved);
        let registry = context.registry.lock().unwrap();
        let id = registry.authenticate(&token).unwrap();
        assert_eq!(registry.permissions(&id), Some(Permissions::default()));
        assert!(!registry.allows(&id, crate::devices::Capability::Input));
    }

    #[test]
    fn completion_saves_before_publishing_and_denial_does_not_write() {
        let (coordinator, context, token) = pending_context();
        assert_eq!(context.registry.lock().unwrap().authenticate(&token), None);
        assert!(context
            .complete_permissions(Some(Permissions::preset(4)))
            .unwrap());
        assert_eq!(coordinator.approval_state(), ApprovalState::Approved);
        assert!(context
            .registry
            .lock()
            .unwrap()
            .authenticate(&token)
            .is_some());
        let (_, denied, token) = pending_context();
        assert!(!denied.complete_permissions(None).unwrap());
        assert_eq!(denied.registry.lock().unwrap().authenticate(&token), None);
    }

    #[test]
    fn registry_failure_rolls_back_capabilities_and_reports_denial() {
        let (coordinator, context, token) = pending_context();
        let dir = std::env::temp_dir().join(format!(
            "wakemate-approval-failure-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        *context.registry.lock().unwrap() =
            DeviceRegistry::load_from_path(blocker.join("devices.json"));
        assert!(context
            .complete_permissions(Some(Permissions::preset(4)))
            .is_err());
        assert_eq!(coordinator.approval_state(), ApprovalState::Denied);
        assert_eq!(context.registry.lock().unwrap().authenticate(&token), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn timeout_releases_reservation_and_old_callback_cannot_approve_new_phone() {
        let (coordinator, old, _) = pending_context();
        *coordinator.active_prompt.lock().unwrap() = Some((old.prompt_id, Instant::now()));
        *coordinator.last_prompt_at.lock().unwrap() = None;
        assert_eq!(coordinator.approval_state(), ApprovalState::Denied);
        assert_eq!(
            coordinator.request(
                request_info(),
                shared_config(),
                shared_registry(),
                Some(PendingEnrollment::new("New Phone".into()))
            ),
            PairingOutcome::AwaitingApproval
        );
        assert!(!old
            .complete_permissions(Some(Permissions::preset(4)))
            .unwrap());
        assert_eq!(coordinator.approval_state(), ApprovalState::Pending);
    }

    #[test]
    fn unavailable_coordinator_reports_idle_and_refuses_requests() {
        let coordinator = PairingCoordinator::unavailable();
        assert_eq!(coordinator.approval_state(), ApprovalState::Idle);

        let outcome = coordinator.request(request_info(), shared_config(), shared_registry(), None);
        assert_eq!(outcome, PairingOutcome::Unavailable);
    }

    #[test]
    fn request_marks_pending_and_notifier_outcome_is_reported() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(|context: NotifierContext| {
            // The real notifier spawns a thread and records the human's
            // answer later; recording synchronously here exercises the
            // same path.
            assert_eq!(context.status.get(), ApprovalState::Pending);
            context.status.record_outcome(true);
        })));

        let outcome = coordinator.request(request_info(), shared_config(), shared_registry(), None);

        assert_eq!(outcome, PairingOutcome::AwaitingApproval);
        assert_eq!(coordinator.approval_state(), ApprovalState::Approved);
    }

    #[test]
    fn approved_enrollment_lands_in_the_registry() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(|context: NotifierContext| {
            // Simulate the desktop user clicking "Yes".
            let pending = context.enrollment.take_valid().expect("enrollment queued");
            context
                .registry
                .lock()
                .unwrap()
                .register_with_id(&pending.device_id, &pending.device_name, &pending.token)
                .unwrap();
            context.status.record_outcome(true);
        })));

        let registry = shared_registry();
        let pending = PendingEnrollment::new("Test Phone".to_string());
        let device_id = pending.device_id.clone();
        let device_token = pending.token.clone();

        let outcome = coordinator.request(
            request_info(),
            shared_config(),
            registry.clone(),
            Some(pending),
        );

        assert_eq!(outcome, PairingOutcome::AwaitingApproval);
        let registry = registry.lock().unwrap();
        assert!(registry.contains(&device_id));
        assert_eq!(registry.authenticate(&device_token), Some(device_id));
    }

    #[test]
    fn denied_enrollment_is_discarded() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(|context: NotifierContext| {
            context.enrollment.clear();
            context.status.record_outcome(false);
        })));

        let registry = shared_registry();
        let pending = PendingEnrollment::new("Test Phone".to_string());
        let device_id = pending.device_id.clone();

        coordinator.request(
            request_info(),
            shared_config(),
            registry.clone(),
            Some(pending),
        );

        assert_eq!(coordinator.approval_state(), ApprovalState::Denied);
        assert!(!coordinator.enrollment_pending_for(&device_id));
        assert!(registry.lock().unwrap().list().is_empty());
    }

    #[test]
    fn enrollment_during_prompt_cooldown_reports_busy() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(|_context: NotifierContext| {
            // Leave the prompt "unanswered" so the cooldown applies.
        })));

        let first = coordinator.request(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(PendingEnrollment::new("Phone A".to_string())),
        );
        assert_eq!(first, PairingOutcome::AwaitingApproval);

        let second = coordinator.request(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(PendingEnrollment::new("Phone B".to_string())),
        );
        assert_eq!(second, PairingOutcome::Busy);

        // A legacy activation retry inside the cooldown is still just
        // "waiting", not an error.
        let legacy = coordinator.request(request_info(), shared_config(), shared_registry(), None);
        assert_eq!(legacy, PairingOutcome::AwaitingApproval);
    }

    #[test]
    fn unavailable_request_does_not_consume_pairing_session() {
        let coordinator = PairingCoordinator::unavailable();
        let token = coordinator.mint_pairing_session_token();

        let outcome = coordinator.request_with_session(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(PendingEnrollment::new("Test Phone".to_string())),
            Some(&token),
        );

        assert_eq!(outcome, PairingOutcome::Unavailable);
        assert!(coordinator.pairing_session_claimable(&token));
    }

    #[test]
    fn busy_request_keeps_session_claimable_until_prompt_finishes() {
        let coordinator = PairingCoordinator::new(Some(Arc::new(|_context: NotifierContext| {
            // Keep the prompt pending.
        })));

        let first_pending = PendingEnrollment::new("Phone A".to_string());
        let first_device_id = first_pending.device_id.clone();
        let first = coordinator.request(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(first_pending),
        );
        assert_eq!(first, PairingOutcome::AwaitingApproval);

        let token = coordinator.mint_pairing_session_token();
        let busy_pending = PendingEnrollment::new("Phone B".to_string());
        let busy_device_id = busy_pending.device_id.clone();
        let busy = coordinator.request_with_session(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(busy_pending),
            Some(&token),
        );
        assert_eq!(busy, PairingOutcome::Busy);
        assert!(coordinator.pairing_session_claimable(&token));
        assert!(coordinator.enrollment_pending_for(&first_device_id));
        assert!(!coordinator.enrollment_pending_for(&busy_device_id));

        *coordinator
            .last_prompt_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;

        let still_busy = coordinator.request_with_session(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(PendingEnrollment::new("Phone B".to_string())),
            Some(&token),
        );
        assert_eq!(still_busy, PairingOutcome::Busy);
        assert!(coordinator.pairing_session_claimable(&token));
        assert!(coordinator.enrollment_pending_for(&first_device_id));

        // Once the first prompt has a definite answer, the same QR session
        // can retry and is consumed exactly when its own prompt is shown.
        coordinator.status.record_outcome(false);
        coordinator.enrollment.clear();

        let retry = coordinator.request_with_session(
            request_info(),
            shared_config(),
            shared_registry(),
            Some(PendingEnrollment::new("Phone B".to_string())),
            Some(&token),
        );
        assert_eq!(retry, PairingOutcome::AwaitingApproval);
        assert!(!coordinator.pairing_session_claimable(&token));
    }

    #[test]
    fn expired_enrollment_cannot_be_taken() {
        let slot = EnrollmentSlot::default();
        let mut pending = PendingEnrollment::new("Old Phone".to_string());
        pending.expires_at = Instant::now();
        let device_id = pending.device_id.clone();
        slot.set(pending);

        assert!(!slot.holds(&device_id));
        assert!(slot.take_valid().is_none());
    }

    #[test]
    fn committing_enrollment_never_exposes_a_denied_gap() {
        let slot = EnrollmentSlot::default();
        let registry = shared_registry();
        let pending = PendingEnrollment::new("Test Phone".to_string());
        let device_id = pending.device_id.clone();
        slot.set(pending);

        assert!(slot.holds(&device_id));
        assert!(!registry.lock().unwrap().contains(&device_id));

        let committed = slot
            .commit_valid(&registry)
            .unwrap()
            .expect("valid enrollment");
        assert_eq!(committed.device_id, device_id);
        assert!(registry.lock().unwrap().contains(&device_id));
        assert!(!slot.holds(&device_id));
    }

    #[test]
    fn denial_is_reported_until_the_next_prompt() {
        let handle = PairingStatusHandle::default();
        handle.set(ApprovalState::Pending);
        handle.record_outcome(false);
        assert_eq!(handle.get(), ApprovalState::Denied);
        assert_eq!(handle.get().as_str(), "denied");
    }

    #[test]
    fn pairing_session_token_matches_after_minting() {
        let slot = PairingSessionSlot::default();
        let token = slot.mint();
        assert!(slot.matches(&token));
        assert!(!slot.matches("some-other-token"));
    }

    #[test]
    fn pairing_session_token_can_only_be_consumed_once() {
        let slot = PairingSessionSlot::default();
        let token = slot.mint();
        assert!(slot.try_consume(&token));
        assert!(!slot.try_consume(&token));
    }

    #[test]
    fn pairing_session_token_still_matches_read_only_after_being_consumed() {
        // /v1/pairing/status polls repeatedly with the same token while
        // waiting for desktop approval; only the state-changing enroll /
        // activate actions burn it.
        let slot = PairingSessionSlot::default();
        let token = slot.mint();
        assert!(slot.try_consume(&token));
        assert!(slot.matches(&token));
    }

    #[test]
    fn minting_a_new_pairing_session_token_invalidates_the_previous_one() {
        let slot = PairingSessionSlot::default();
        let first = slot.mint();
        let second = slot.mint();

        assert!(!slot.matches(&first));
        assert!(slot.matches(&second));
    }

    #[test]
    fn expired_pairing_session_token_is_rejected() {
        let slot = PairingSessionSlot::default();
        let token = slot.mint();
        if let Some(session) = slot.lock().as_mut() {
            session.expires_at = Instant::now();
        }

        assert!(!slot.matches(&token));
        assert!(!slot.try_consume(&token));
    }

    #[test]
    fn coordinator_mints_and_consumes_pairing_session_tokens() {
        let coordinator = PairingCoordinator::unavailable();
        let token = coordinator.mint_pairing_session_token();

        assert!(coordinator.pairing_session_matches(&token));
        assert!(coordinator.consume_pairing_session_token(&token));
        assert!(!coordinator.consume_pairing_session_token(&token));
        // Still readable after being consumed once, for status polling.
        assert!(coordinator.pairing_session_matches(&token));
    }
}
