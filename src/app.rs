use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Query, Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

use crate::request_limits::CommandBudget;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::{
    config::{AppConfig, SharedConfig},
    devices::{Capability, Permissions, SharedDeviceRegistry},
    error::AppError,
    input_executor::{InputCommand, InputError, InputExecutor},
    pairing::{
        ApprovalState, PairingCoordinator, PairingOutcome, PairingRequestInfo, PendingEnrollment,
    },
    secure_attention::{self, SasOutcome},
    security::RateLimiter,
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
    pub input: InputExecutor,
    requests: Arc<Semaphore>,
    power: Arc<Semaphore>,
    command_budget: Arc<CommandBudget>,
    /// Defense-in-depth flag for a non-interactive server context. Windows no
    /// longer starts such a server, but any future/headless host still lacks
    /// a desktop session for approval or input, so both stay refused.
    pub headless: bool,
}

impl AppState {
    #[cfg(test)]
    pub fn new(
        config: SharedConfig,
        pairing: Arc<PairingCoordinator>,
        registry: SharedDeviceRegistry,
        headless: bool,
    ) -> Result<Self, String> {
        Ok(Self::with_input(
            config,
            pairing,
            registry,
            headless,
            InputExecutor::new().map_err(|e| e.to_string())?,
        ))
    }

    pub fn with_input(
        config: SharedConfig,
        pairing: Arc<PairingCoordinator>,
        registry: SharedDeviceRegistry,
        headless: bool,
        input: InputExecutor,
    ) -> Self {
        Self {
            config,
            pairing,
            registry,
            headless,
            input,
            rate_limiter: Arc::new(RateLimiter::new()),
            requests: Arc::new(Semaphore::new(64)),
            power: Arc::new(Semaphore::new(1)),
            command_budget: Arc::new(CommandBudget::default()),
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            native_client_boundary,
        ))
        .with_state(state)
}

/// Transitional HTTP is credential-free health only; never enrollment/control.
pub fn public_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .layer(middleware::from_fn(
            |request: Request, next: Next| async move {
                if request.headers().contains_key(AUTH_HEADER)
                    || request.headers().contains_key(header::AUTHORIZATION)
                {
                    AppError::forbidden("Credentials require pinned HTTPS").into_response()
                } else {
                    next.run(request).await
                }
            },
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            native_client_boundary,
        ))
        .with_state(state)
}

async fn native_client_boundary(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    // WakeMATE has native phone clients, no browser API consumer. A browser
    // Origin (including "null") is never needed for this local control API.
    let mut response = if request.headers().contains_key(header::ORIGIN) {
        AppError::forbidden("browser origins are not allowed").into_response()
    } else if let Ok(_permit) = state.requests.clone().try_acquire_owned() {
        match tokio::time::timeout(std::time::Duration::from_secs(10), next.run(request)).await {
            Ok(response) => response,
            Err(_) => AppError::request_timeout("Request timed out; retry a short operation")
                .into_response(),
        }
    } else {
        AppError::too_many_requests("Companion is busy; retry shortly").into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
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
    let principal = authenticate(&state, peer, &headers)?;
    require_capability(&state, &principal, Capability::Status)?;
    let permissions = registry_snapshot(&state)?
        .permissions(&principal.id)
        .unwrap_or_default();
    let network = crate::network::for_peer(peer.ip());
    Ok(Json(ApiResponse::ok(
        "companion details",
        InfoResponse {
            device_name: config.device_name.clone(),
            local_ip: network
                .as_ref()
                .map(|n| n.local_ip.clone())
                .unwrap_or_else(|| "127.0.0.1".into()),
            interface_name: network.as_ref().map(|n| n.interface_name.clone()),
            mac_address: network.as_ref().and_then(|n| n.mac_address.clone()),
            subnet_mask: network.as_ref().and_then(|n| n.subnet_mask.clone()),
            broadcast_address: network.as_ref().and_then(|n| n.broadcast_address.clone()),
            ping_address: network.as_ref().and_then(|n| n.ping_address.clone()),
            wol_port: network.as_ref().map(|n| n.wol_port).unwrap_or(9),
            bind_address: config.effective_tls_bind_address(),
            discovery_port: config.discovery_port,
            allow_remote_connections: config.allow_remote_connections,
            allow_discovery: config.discovery_enabled(),
            allow_input_commands: permissions.input,
            allow_power_commands: permissions.power,
        },
    )))
}

async fn pairing_check(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<()>>, AppError> {
    authenticate(&state, peer, &headers)?;
    Ok(Json(ApiResponse::message("approved device accepted")))
}

async fn pairing_status(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<PairingStatusQuery>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingStatusResponse>>, AppError> {
    let credential = authenticate_pairing_status(&state, peer, &headers)?;
    // Expire the reservation before taking registry locks; callbacks commit in
    // lifecycle -> registry -> enrollment order.
    state.pairing.approval_state();
    let own_id = match &credential {
        AuthenticatedCredential::Device(principal) => Some(principal.id.clone()),
        AuthenticatedCredential::PairingSession(token) => state.pairing.session_device_id(token),
    };
    if query
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .is_some_and(|id| own_id.as_deref() != Some(id))
    {
        return Err(AppError::forbidden(
            "Pairing status is scoped to this phone or QR enrollment",
        ));
    }
    let id = own_id.as_deref();
    let (approval, permissions) = if let Some(id) = id {
        let registry = registry_snapshot(&state)?;
        if let Some(permissions) = registry.permissions(id).filter(|_| !registry.paused()) {
            (ApprovalState::Approved, permissions)
        } else if state.pairing.enrollment_pending_for(id) {
            (ApprovalState::Pending, Permissions::default())
        } else {
            (ApprovalState::Denied, Permissions::default())
        }
    } else {
        (ApprovalState::Idle, Permissions::default())
    };
    Ok(Json(ApiResponse::ok(
        "pairing status",
        PairingStatusResponse {
            approval: approval.as_str(),
            allow_input_commands: approval == ApprovalState::Approved && permissions.input,
            allow_power_commands: approval == ApprovalState::Approved && permissions.power,
        },
    )))
}

async fn pairing_enroll(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<EnrollRequest>,
) -> Result<Json<ApiResponse<EnrollResponse>>, AppError> {
    // Only an unconsumed, short-lived visual invitation can enroll a phone.
    let credential = authenticate_invitation(&state, peer, &headers)?;
    if registry_snapshot(&state)?.paused() {
        return Err(AppError::forbidden(
            "Remote access is paused. Resume or recover it from the PC settings before pairing.",
        ));
    }

    if state.headless {
        return Err(AppError::forbidden(
            "no one is signed in on this computer right now, so pairing cannot be confirmed; \
             open the WakeMATE tray app on the desktop and try again",
        ));
    }

    let device_name = request
        .device_name
        .as_deref()
        .filter(|name| {
            !name.chars().any(|c| {
                c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        })
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(MAX_DEVICE_NAME_CHARS).collect::<String>())
        .unwrap_or_else(|| format!("Phone at {}", peer.ip()));

    let pending = PendingEnrollment::new(device_name);
    let device_id = pending.device_id.clone();
    let device_token = pending.token.clone();

    let request_info = PairingRequestInfo { peer: peer.ip() };
    let outcome = match credential.pairing_session_token() {
        Some(token) => state.pairing.request_with_session(
            request_info,
            state.config.clone(),
            state.registry.clone(),
            Some(pending),
            Some(token),
        ),
        None => state.pairing.request(
            request_info,
            state.config.clone(),
            state.registry.clone(),
            Some(pending),
        ),
    };

    match outcome {
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
        PairingOutcome::InvalidSession => Err(AppError::unauthorized(
            "this pairing QR code has expired or was already used; show a new pairing QR code and scan it again",
        )),
    }
}

async fn pairing_activate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingActivationResponse>>, AppError> {
    authenticate_pairing_status(&state, peer, &headers)?;
    Err(AppError::forbidden(
        "Legacy activation is retired. Update the phone app and scan a new pairing QR code.",
    ))
}

async fn wake(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeRequest>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let principal = authenticate(&state, peer, &headers)?;
    require_capability(&state, &principal, Capability::Wake)?;
    if !state.command_budget.admit(&principal.id, Capability::Wake) {
        return Err(AppError::too_many_requests(
            "Wake requests are limited to four per second per phone",
        ));
    }
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
    let principal = authenticate(&state, peer, &headers)?;
    let capability = match &request {
        CommandRequest::Wake { .. } => Capability::Wake,
        CommandRequest::System { .. } | CommandRequest::SecurityScreen { .. } => Capability::Power,
        CommandRequest::Media { .. } => Capability::Media,
        _ => Capability::Input,
    };
    require_capability(&state, &principal, capability)?;
    if !state.command_budget.admit(&principal.id, capability) {
        return Err(AppError::too_many_requests(
            "Command rate exceeded; slow down and retry",
        ));
    }
    if state.headless
        && matches!(
            capability,
            Capability::Input | Capability::Media | Capability::Power
        )
    {
        return Err(AppError::forbidden(
            "Desktop commands require an interactive user session",
        ));
    }
    match request {
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
            return Ok(Json(ApiResponse::ok_empty("wake packet sent")));
        }
        CommandRequest::SecurityScreen { fallback } => {
            let permit =
                state.power.clone().try_acquire_owned().map_err(|_| {
                    AppError::too_many_requests("A system action is already running")
                })?;
            let state_for_worker = state.clone();
            let id = principal.id.clone();
            return tokio::task::spawn_blocking(move || {
                let _permit = permit;
                require_capability(&state_for_worker, &Principal { id }, Capability::Power)?;
                Ok(Json(security_screen(&state_for_worker, fallback)))
            })
            .await
            .map_err(|_| AppError::internal("System worker stopped"))?;
        }
        CommandRequest::System { action } => {
            let permit =
                state.power.clone().try_acquire_owned().map_err(|_| {
                    AppError::too_many_requests("A system action is already running")
                })?;
            let registry = state.registry.clone();
            let id = principal.id.clone();
            let message = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if !registry
                    .lock()
                    .map(|r| r.allows(&id, Capability::Power))
                    .unwrap_or(false)
                {
                    return Err(AppError::forbidden(
                        "Permission was revoked or control paused",
                    ));
                }
                system::perform_system_action(action).map_err(AppError::unsupported)
            })
            .await
            .map_err(|_| AppError::internal("System worker stopped"))??;
            return Ok(Json(ApiResponse::ok_empty(message)));
        }
        request => {
            let input = match request {
                CommandRequest::MouseMove { delta_x, delta_y } => {
                    InputCommand::MouseMove { delta_x, delta_y }
                }
                CommandRequest::MouseClick { button, double } => InputCommand::MouseClick {
                    button: button.unwrap_or(crate::types::MouseButtonArg::Left),
                    double: double.unwrap_or(false),
                },
                CommandRequest::MouseButton { button, action } => InputCommand::MouseButton {
                    button: button.unwrap_or(crate::types::MouseButtonArg::Left),
                    action,
                },
                CommandRequest::MouseScroll { direction, amount } => InputCommand::MouseScroll {
                    direction,
                    amount: amount.unwrap_or(3),
                },
                CommandRequest::KeyPress { key } => InputCommand::KeyPress { key },
                CommandRequest::TextInput { text } => InputCommand::TextInput { text },
                CommandRequest::Media { action } => InputCommand::Media { action },
                _ => return Err(AppError::bad_request("Unsupported input command")),
            };
            let registry = state.registry.clone();
            let id = principal.id.clone();
            let authorize = Arc::new(move || {
                registry
                    .lock()
                    .map(|r| r.allows(&id, capability))
                    .unwrap_or(false)
            });
            state
                .input
                .execute(&principal.id, input, authorize)
                .await
                .map_err(input_error)?;
        }
    }
    Ok(Json(ApiResponse::ok_empty("input accepted by Windows")))
}

fn input_error(error: InputError) -> AppError {
    match error {
        InputError::Invalid(message) => AppError::bad_request(message),
        InputError::Busy => AppError::too_many_requests(
            "Remote input is busy; release local keys/buttons or retry shortly",
        ),
        InputError::Cancelled | InputError::Denied => {
            AppError::forbidden("Input cancelled: permission changed or control paused")
        }
        InputError::Unsupported(message) => AppError::unsupported(message),
        InputError::Execution(message) => AppError::internal(message),
        InputError::Unavailable => AppError::internal("Input worker unavailable"),
    }
}

fn require_capability(
    state: &AppState,
    principal: &Principal,
    capability: Capability,
) -> Result<(), AppError> {
    if registry_snapshot(state)?.allows(&principal.id, capability) {
        Ok(())
    } else {
        Err(AppError::forbidden("This phone does not have permission, or remote control is paused. Change permissions on the PC."))
    }
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
        // A non-interactive server has no desktop session, so there
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
        #[cfg(not(target_os = "windows"))]
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

#[derive(Clone)]
struct Principal {
    id: String,
}

enum AuthenticatedCredential {
    PairingSession(String),
    Device(Principal),
}
impl AuthenticatedCredential {
    fn pairing_session_token(&self) -> Option<&str> {
        match self {
            Self::PairingSession(token) => Some(token),
            Self::Device(_) => None,
        }
    }
}

fn authenticate(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<Principal, AppError> {
    match authenticate_inner(state, peer, headers, true, false, false)? {
        AuthenticatedCredential::Device(principal) => Ok(principal),
        _ => Err(AppError::unauthorized(
            "Approved device credential required",
        )),
    }
}
fn authenticate_invitation(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<AuthenticatedCredential, AppError> {
    authenticate_inner(state, peer, headers, false, true, true)
}
fn authenticate_pairing_status(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<AuthenticatedCredential, AppError> {
    authenticate_inner(state, peer, headers, true, true, false)
}
fn authenticate_inner(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    devices: bool,
    invitations: bool,
    claimable: bool,
) -> Result<AuthenticatedCredential, AppError> {
    if let Some(remaining) = state.rate_limiter.locked_out(peer.ip()) {
        return Err(AppError::too_many_requests(format!(
            "Too many failed attempts; retry in {} seconds",
            remaining.as_secs().max(1)
        )));
    }
    let provided = headers.get(AUTH_HEADER).and_then(|v| v.to_str().ok());
    let credential = provided.and_then(|token| {
        if invitations
            && if claimable {
                state.pairing.pairing_session_claimable(token)
            } else {
                state.pairing.pairing_session_matches(token)
            }
        {
            return Some(AuthenticatedCredential::PairingSession(token.into()));
        }
        if devices {
            return state
                .registry
                .lock()
                .ok()
                .and_then(|r| r.authenticate(token))
                .map(|id| AuthenticatedCredential::Device(Principal { id }));
        }
        None
    });
    match credential {
        Some(credential) => {
            state.rate_limiter.record_success(peer.ip());
            Ok(credential)
        }
        None => {
            state.rate_limiter.record_failure(peer.ip());
            Err(AppError::unauthorized(
                "Approved device or current QR invitation required",
            ))
        }
    }
}
fn registry_snapshot(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::devices::DeviceRegistry>, AppError> {
    state
        .registry
        .lock()
        .map_err(|_| AppError::internal("Device registry unavailable"))
}

fn config_snapshot(config: &SharedConfig) -> Result<AppConfig, AppError> {
    config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| AppError::internal("failed to access application config"))
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
