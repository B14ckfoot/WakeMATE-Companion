use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};

use tracing::{info, warn};

use crate::{
    config::{AppConfig, SharedConfig},
    devices::SharedDeviceRegistry,
    error::AppError,
    input::InputController,
    pairing::{
        ApprovalState, PairingCoordinator, PairingOutcome, PairingRequestInfo, PendingEnrollment,
    },
    secure_attention::{self, SasOutcome},
    security::{tokens_match, RateLimiter},
    system,
    types::{
        ApiResponse, CommandRequest, EnrollRequest, EnrollResponse, HealthResponse, InfoResponse,
        PairingActivationResponse, PairingStatusQuery, PairingStatusResponse,
        SecurityCommandAction, SecurityCommandResult, SecurityCommandStatus, SecurityFallback,
        SystemAction, WakeRequest, AUTH_HEADER, PROTOCOL_VERSION,
    },
};

const MAX_DEVICE_NAME_CHARS: usize = 48;

#[derive(Clone)]
pub struct AppState {
    pub config: SharedConfig,
    pub rate_limiter: Arc<RateLimiter>,
    pub pairing: Arc<PairingCoordinator>,
    pub registry: SharedDeviceRegistry,
    /// Set for the boot-time, pre-logon service instance. No interactive
    /// desktop session exists to approve a pairing prompt or receive input
    /// commands in that context, so both are refused regardless of config.
    pub headless: bool,
}

impl AppState {
    pub fn new(
        config: SharedConfig,
        pairing: Arc<PairingCoordinator>,
        registry: SharedDeviceRegistry,
        headless: bool,
    ) -> Self {
        Self {
            config,
            rate_limiter: Arc::new(RateLimiter::new()),
            pairing,
            registry,
            headless,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/v1/health", get(health))
        .route("/v1/info", get(info))
        .route("/v1/pairing/check", get(pairing_check))
        .route("/v1/pairing/status", get(pairing_status))
        .route("/v1/pairing/enroll", post(pairing_enroll))
        .route("/v1/pairing/activate", post(pairing_activate))
        .route("/v1/wake", post(wake))
        .route("/v1/command", post(command))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(state)
}

async fn root() -> Json<ApiResponse<()>> {
    Json(ApiResponse::message("WakeMATE companion is running"))
}

async fn health(State(state): State<AppState>) -> Json<ApiResponse<HealthResponse>> {
    let config = config_snapshot(&state.config).unwrap_or_else(|_| AppConfig::default());

    Json(ApiResponse::ok(
        "companion online",
        HealthResponse {
            status: "online",
            device_name: config.device_name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
        },
    ))
}

async fn info(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<InfoResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    if config.require_auth_for_info {
        authenticate(&state, peer, &headers, &config.api_token)?;
    }

    let bind_address = config.effective_bind_address();
    let network = system::primary_network_info();

    Ok(Json(ApiResponse::ok(
        "companion details",
        InfoResponse {
            device_name: config.device_name.clone(),
            local_ip: network
                .as_ref()
                .map(|info| info.local_ip.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            interface_name: network.as_ref().map(|info| info.interface_name.clone()),
            mac_address: network.as_ref().and_then(|info| info.mac_address.clone()),
            subnet_mask: network.as_ref().and_then(|info| info.subnet_mask.clone()),
            broadcast_address: network
                .as_ref()
                .and_then(|info| info.broadcast_address.clone()),
            ping_address: network.as_ref().and_then(|info| info.ping_address.clone()),
            wol_port: network.as_ref().map(|info| info.wol_port).unwrap_or(9),
            bind_address,
            discovery_port: config.discovery_port,
            allow_remote_connections: config.allow_remote_connections,
            allow_discovery: config.discovery_enabled(),
            allow_input_commands: config.allow_input_commands,
            allow_power_commands: config.allow_power_commands,
        },
    )))
}

async fn pairing_check(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;
    Ok(Json(ApiResponse::message("pairing token accepted")))
}

async fn pairing_status(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<PairingStatusQuery>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingStatusResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate_pairing_status(&state, peer, &headers, &config.api_token)?;

    let device_id = query
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let approval = if let Some(device_id) = device_id {
        // Per-device enrollment state: approved once the registry holds it,
        // pending while it sits in the enrollment slot, otherwise denied
        // (an explicit "No", an expired prompt, or a later revocation all
        // mean the same thing to the phone: not authorized, enroll again).
        let registered = registry_snapshot(&state)?.contains(device_id);
        if registered {
            ApprovalState::Approved
        } else if state.pairing.enrollment_pending_for(device_id) {
            ApprovalState::Pending
        } else {
            ApprovalState::Denied
        }
    } else {
        // Legacy process-wide state. If capabilities are already granted (an
        // earlier approval, possibly in a previous run), report "approved"
        // even though no prompt happened this session -- the phone cares
        // about the effective outcome.
        match state.pairing.approval_state() {
            ApprovalState::Idle if config.allow_input_commands || config.allow_power_commands => {
                ApprovalState::Approved
            }
            state => state,
        }
    };

    Ok(Json(ApiResponse::ok(
        "pairing status",
        PairingStatusResponse {
            approval: approval.as_str(),
            allow_input_commands: config.allow_input_commands,
            allow_power_commands: config.allow_power_commands,
        },
    )))
}

async fn pairing_enroll(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<EnrollRequest>,
) -> Result<Json<ApiResponse<EnrollResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    // Enrollment specifically requires the shared QR token -- a per-device
    // token must not be able to mint further device tokens.
    authenticate_shared_only(&state, peer, &headers, &config.api_token)?;

    if state.headless {
        return Err(AppError::forbidden(
            "no one is signed in on this computer right now, so pairing cannot be confirmed; \
             open the WakeMATE tray app on the desktop and try again",
        ));
    }

    let device_name = request
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(MAX_DEVICE_NAME_CHARS).collect::<String>())
        .unwrap_or_else(|| format!("Phone at {}", peer.ip()));

    let pending = PendingEnrollment::new(device_name);
    let device_id = pending.device_id.clone();
    let device_token = pending.token.clone();

    match state.pairing.request(
        PairingRequestInfo { peer: peer.ip() },
        state.config.clone(),
        state.registry.clone(),
        Some(pending),
    ) {
        PairingOutcome::AwaitingApproval => Ok(Json(ApiResponse::ok(
            "waiting for approval on the desktop",
            EnrollResponse {
                device_id,
                device_token,
                status: "pending_approval",
            },
        ))),
        PairingOutcome::Busy => Err(AppError::too_many_requests(
            "another pairing request is already waiting for approval on this computer; \
             try again in a few seconds",
        )),
        PairingOutcome::Unavailable => Err(AppError::forbidden(
            "the WakeMATE tray app is not running, so pairing cannot be confirmed on this computer",
        )),
    }
}

async fn pairing_activate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingActivationResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate_pairing_activate(&state, peer, &headers, &config.api_token)?;

    if state.headless {
        return Err(AppError::forbidden(
            "no one is signed in on this computer right now, so pairing cannot be confirmed; \
             open the WakeMATE tray app on the desktop and try again",
        ));
    }

    match state.pairing.request(
        PairingRequestInfo { peer: peer.ip() },
        state.config.clone(),
        state.registry.clone(),
        None,
    ) {
        PairingOutcome::AwaitingApproval | PairingOutcome::Busy => Ok(Json(ApiResponse::ok(
            "waiting for approval on the desktop",
            PairingActivationResponse {
                allow_input_commands: config.allow_input_commands,
                allow_power_commands: config.allow_power_commands,
                status: "pending_approval",
            },
        ))),
        PairingOutcome::Unavailable => Err(AppError::forbidden(
            "the WakeMATE tray app is not running, so pairing cannot be confirmed on this computer",
        )),
    }
}

async fn wake(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeRequest>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;

    system::send_wol(
        &request.mac,
        request.broadcast.as_deref().unwrap_or("255.255.255.255"),
        request.port.unwrap_or(9),
    )
    .map_err(AppError::bad_request)?;

    Ok(Json(ApiResponse::message("wake packet sent")))
}

async fn command(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CommandRequest>,
) -> Result<Json<ApiResponse<SecurityCommandResult>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;

    // Handled ahead of the shared match because it is the one command that
    // answers with a structured payload instead of a bare message.
    if let CommandRequest::SecurityScreen { fallback } = request {
        ensure_power_enabled(&state, &config)?;
        return Ok(Json(security_screen(&state, fallback)));
    }

    let input = InputController;
    let message = match request {
        CommandRequest::Wake {
            mac,
            broadcast,
            port,
        } => {
            system::send_wol(
                &mac,
                broadcast.as_deref().unwrap_or("255.255.255.255"),
                port.unwrap_or(9),
            )
            .map_err(AppError::bad_request)?;
            "wake packet sent".to_string()
        }
        CommandRequest::MouseMove { delta_x, delta_y } => {
            ensure_input_enabled(&state, &config)?;
            input
                .mouse_move_relative(delta_x, delta_y)
                .map_err(AppError::internal)?;
            format!("mouse moved by {delta_x}, {delta_y}")
        }
        CommandRequest::MouseClick { button, double } => {
            ensure_input_enabled(&state, &config)?;
            let button = button.unwrap_or(crate::types::MouseButtonArg::Left);
            let double = double.unwrap_or(false);
            input
                .mouse_click(button, double)
                .map_err(AppError::internal)?;
            if double {
                "mouse double click sent".to_string()
            } else {
                "mouse click sent".to_string()
            }
        }
        CommandRequest::MouseButton { button, action } => {
            ensure_input_enabled(&state, &config)?;
            let button = button.unwrap_or(crate::types::MouseButtonArg::Left);
            input
                .mouse_button(button, action)
                .map_err(AppError::internal)?;
            "mouse button state sent".to_string()
        }
        CommandRequest::MouseScroll { direction, amount } => {
            ensure_input_enabled(&state, &config)?;
            let amount = amount.unwrap_or(3);
            input
                .mouse_scroll(direction, amount)
                .map_err(AppError::internal)?;
            "mouse scroll sent".to_string()
        }
        CommandRequest::KeyPress { key } => {
            ensure_input_enabled(&state, &config)?;
            input.key_press(&key).map_err(AppError::bad_request)?;
            format!("key press sent: {key}")
        }
        CommandRequest::TextInput { text } => {
            ensure_input_enabled(&state, &config)?;
            input.text_input(&text).map_err(AppError::internal)?;
            "text input sent".to_string()
        }
        CommandRequest::Media { action } => {
            ensure_input_enabled(&state, &config)?;
            input.media_action(action).map_err(AppError::bad_request)?;
            "media command sent".to_string()
        }
        CommandRequest::System { action } => {
            ensure_power_enabled(&state, &config)?;
            system::perform_system_action(action)
                .map_err(AppError::unsupported)?
                .to_string()
        }
        // Returned above, before this match runs. Reported as an error rather
        // than a panic so a future refactor that breaks that invariant fails
        // one request instead of the connection.
        CommandRequest::SecurityScreen { .. } => {
            return Err(AppError::internal(
                "security screen command was not dispatched",
            ));
        }
    };

    Ok(Json(ApiResponse::ok_empty(message)))
}

/// Runs the Ctrl+Alt+Delete request and reports exactly what Windows did.
///
/// Answers `200 OK` in every case: the payload's `status` is the real result,
/// and folding "Windows refused this" into an HTTP error would lose the
/// distinction between a refusal, an unsupported OS, and a failed attempt.
/// The caller is already authenticated and power-gated by this point.
fn security_screen(
    state: &AppState,
    fallback: SecurityFallback,
) -> ApiResponse<SecurityCommandResult> {
    info!(
        fallback = ?fallback,
        "security screen command authorized"
    );

    if state.headless {
        // The pre-logon service instance has no interactive session, so there
        // is no desktop for a security screen to appear on.
        let result = SecurityCommandResult {
            status: SecurityCommandStatus::Unsupported,
            action: SecurityCommandAction::None,
            fallback_used: false,
            detail: "no one is signed in on this computer right now, so there is no desktop to show the security screen on".to_string(),
        };
        warn!(status = ?result.status, "security screen command refused");
        return ApiResponse::failed("security screen unavailable", result);
    }

    let outcome = secure_attention::request_secure_attention_sequence();

    let result = match outcome {
        SasOutcome::Requested => SecurityCommandResult {
            status: SecurityCommandStatus::Success,
            action: SecurityCommandAction::SecureAttentionSequence,
            fallback_used: false,
            detail: "Windows accepted the Secure Attention Sequence".to_string(),
        },
        SasOutcome::Unsupported { detail } => {
            apply_fallback(fallback, SecurityCommandStatus::Unsupported, detail)
        }
        SasOutcome::PermissionRequired { detail } => {
            apply_fallback(fallback, SecurityCommandStatus::PermissionRequired, detail)
        }
        SasOutcome::Failed { detail } => {
            apply_fallback(fallback, SecurityCommandStatus::ExecutionFailed, detail)
        }
    };

    if result.status == SecurityCommandStatus::Success {
        info!(
            action = ?result.action,
            fallback_used = result.fallback_used,
            "security screen command completed"
        );
        let message = if result.fallback_used {
            "security screen unavailable; fallback performed"
        } else {
            "security screen opened"
        };
        ApiResponse::ok(message, result)
    } else {
        warn!(
            status = ?result.status,
            detail = %result.detail,
            "security screen command could not be completed"
        );
        ApiResponse::failed("security screen unavailable", result)
    }
}

/// Turns a refused Secure Attention Sequence into whatever substitute the
/// caller explicitly opted into. With no fallback the desktop is left
/// untouched and the original refusal is reported unchanged, so a phone can
/// never trigger a surprise lock it did not ask for.
fn apply_fallback(
    fallback: SecurityFallback,
    status: SecurityCommandStatus,
    detail: String,
) -> SecurityCommandResult {
    match fallback {
        SecurityFallback::None => SecurityCommandResult {
            status,
            action: SecurityCommandAction::None,
            fallback_used: false,
            detail,
        },
        SecurityFallback::Lock => match system::perform_system_action(SystemAction::Lock) {
            Ok(_) => SecurityCommandResult {
                status: SecurityCommandStatus::Success,
                action: SecurityCommandAction::Lock,
                fallback_used: true,
                detail,
            },
            Err(error) => SecurityCommandResult {
                status: SecurityCommandStatus::ExecutionFailed,
                action: SecurityCommandAction::None,
                fallback_used: false,
                detail: format!("{detail} Locking the computer instead also failed: {error}"),
            },
        },
    }
}

/// How `authenticate_inner` treats the one-time pairing-session token (the
/// credential embedded in the pairing QR code / Universal Link) in addition
/// to the permanent shared `api_token`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionTokenMode {
    /// Not accepted at all (`/v1/wake`, `/v1/command`, `/v1/info`): the
    /// pairing-session token is scoped to the pairing endpoints only.
    None,
    /// Accepted without being consumed (`/v1/pairing/status`), since a phone
    /// polls this repeatedly with the same token while waiting for desktop
    /// approval.
    ReadOnly,
    /// Accepted and burned on success (`/v1/pairing/enroll`,
    /// `/v1/pairing/activate`), so a leaked QR code or link cannot be
    /// replayed to start unlimited pairing attempts.
    Consume,
}

/// Validates the pairing credential -- the shared QR token or any approved
/// per-device token -- with constant-time comparisons, and applies the
/// per-IP lockout so a stolen or guessed token cannot be brute-forced by
/// hammering any authenticated endpoint.
fn authenticate(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), AppError> {
    authenticate_inner(
        state,
        peer,
        headers,
        expected_token,
        true,
        SessionTokenMode::None,
    )
}

/// Like `authenticate`, but only the shared QR token or a valid one-time
/// pairing-session token is accepted -- never a per-device token, so an
/// already-approved phone can never mint further device tokens for other
/// phones. The session token is consumed on success.
fn authenticate_shared_only(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), AppError> {
    authenticate_inner(
        state,
        peer,
        headers,
        expected_token,
        false,
        SessionTokenMode::Consume,
    )
}

/// Used by `/v1/pairing/status`: accepts the shared token, an approved
/// device token, or the current pairing-session token, without consuming
/// the session token -- a phone polls this endpoint repeatedly with the same
/// token while waiting for the desktop prompt to be answered.
fn authenticate_pairing_status(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), AppError> {
    authenticate_inner(
        state,
        peer,
        headers,
        expected_token,
        true,
        SessionTokenMode::ReadOnly,
    )
}

/// Used by `/v1/pairing/activate` (the legacy, non-per-device pairing
/// flow): accepts the shared token, an approved device token, or the
/// current pairing-session token, consuming the session token on success.
fn authenticate_pairing_activate(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), AppError> {
    authenticate_inner(
        state,
        peer,
        headers,
        expected_token,
        true,
        SessionTokenMode::Consume,
    )
}

fn authenticate_inner(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
    accept_device_tokens: bool,
    session_mode: SessionTokenMode,
) -> Result<(), AppError> {
    if let Some(remaining) = state.rate_limiter.locked_out(peer.ip()) {
        return Err(AppError::too_many_requests(format!(
            "too many failed attempts; try again in {} seconds",
            remaining.as_secs().max(1)
        )));
    }

    let provided = headers
        .get(AUTH_HEADER)
        .and_then(|value| value.to_str().ok());

    let accepted = provided
        .map(|provided| {
            if tokens_match(provided, expected_token) {
                return true;
            }
            match session_mode {
                SessionTokenMode::None => {}
                SessionTokenMode::ReadOnly => {
                    if state.pairing.pairing_session_matches(provided) {
                        return true;
                    }
                }
                SessionTokenMode::Consume => {
                    if state.pairing.consume_pairing_session_token(provided) {
                        return true;
                    }
                }
            }
            if !accept_device_tokens {
                return false;
            }
            state
                .registry
                .lock()
                .map(|registry| registry.authenticate(provided).is_some())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !accepted {
        state.rate_limiter.record_failure(peer.ip());
        return Err(AppError::unauthorized("unauthorized"));
    }

    state.rate_limiter.record_success(peer.ip());
    Ok(())
}

fn registry_snapshot(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::devices::DeviceRegistry>, AppError> {
    state
        .registry
        .lock()
        .map_err(|_| AppError::internal("failed to access the paired-device registry"))
}

fn ensure_input_enabled(state: &AppState, config: &AppConfig) -> Result<(), AppError> {
    if state.headless {
        return Err(AppError::forbidden(
            "input commands are not available from the pre-logon service",
        ));
    }

    if config.allow_input_commands {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "input commands are disabled in the config",
        ))
    }
}

fn ensure_power_enabled(_state: &AppState, config: &AppConfig) -> Result<(), AppError> {
    if config.allow_power_commands {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "power commands are disabled in the config",
        ))
    }
}

fn config_snapshot(config: &SharedConfig) -> Result<AppConfig, AppError> {
    config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| AppError::internal("failed to access application config"))
}
