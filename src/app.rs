use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};

use crate::{
    config::{AppConfig, SharedConfig},
    error::AppError,
    input::InputController,
    pairing::{ApprovalState, PairingCoordinator, PairingOutcome, PairingRequestInfo},
    security::{tokens_match, RateLimiter},
    system,
    types::{
        ApiResponse, CommandRequest, HealthResponse, InfoResponse, PairingActivationResponse,
        PairingStatusResponse, WakeRequest, AUTH_HEADER, PROTOCOL_VERSION,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub config: SharedConfig,
    pub rate_limiter: Arc<RateLimiter>,
    pub pairing: Arc<PairingCoordinator>,
    /// Set for the boot-time, pre-logon service instance. No interactive
    /// desktop session exists to approve a pairing prompt or receive input
    /// commands in that context, so both are refused regardless of config.
    pub headless: bool,
}

impl AppState {
    pub fn new(config: SharedConfig, pairing: Arc<PairingCoordinator>, headless: bool) -> Self {
        Self {
            config,
            rate_limiter: Arc::new(RateLimiter::new()),
            pairing,
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
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingStatusResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;

    // If capabilities are already granted (an earlier approval, possibly in a
    // previous run), report "approved" even though no prompt happened this
    // session -- the phone cares about the effective outcome.
    let approval = match state.pairing.approval_state() {
        ApprovalState::Idle if config.allow_input_commands || config.allow_power_commands => {
            ApprovalState::Approved
        }
        state => state,
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

async fn pairing_activate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PairingActivationResponse>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;

    if state.headless {
        return Err(AppError::forbidden(
            "no one is signed in on this computer right now, so pairing cannot be confirmed; \
             open the WakeMATE tray app on the desktop and try again",
        ));
    }

    match state
        .pairing
        .request(PairingRequestInfo { peer: peer.ip() }, state.config.clone())
    {
        PairingOutcome::AwaitingApproval => Ok(Json(ApiResponse::ok(
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
) -> Result<Json<ApiResponse<()>>, AppError> {
    let config = config_snapshot(&state.config)?;
    authenticate(&state, peer, &headers, &config.api_token)?;

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
    };

    Ok(Json(ApiResponse::message(message)))
}

/// Validates the pairing token with a constant-time comparison and applies
/// the per-IP lockout so a stolen or guessed token cannot be brute-forced by
/// hammering any authenticated endpoint.
fn authenticate(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    expected_token: &str,
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
        .map(|provided| tokens_match(provided, expected_token))
        .unwrap_or(false);

    if !accepted {
        state.rate_limiter.record_failure(peer.ip());
        return Err(AppError::unauthorized("unauthorized"));
    }

    state.rate_limiter.record_success(peer.ip());
    Ok(())
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
