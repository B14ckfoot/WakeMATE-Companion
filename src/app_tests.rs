use super::*;
use crate::devices::DeviceRegistry;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use std::sync::Mutex;
use tower::ServiceExt;

fn state() -> AppState {
    let state = AppState::new(
        Arc::new(Mutex::new(AppConfig::default())),
        Arc::new(PairingCoordinator::new(Some(Arc::new(|_| {})))),
        Arc::new(Mutex::new(DeviceRegistry::default())),
        false,
    )
    .unwrap();
    state
        .registry
        .lock()
        .unwrap()
        .register_with_id("baseline", "Phone", "approved-token")
        .unwrap();
    state
}

async fn request(
    state: AppState,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
    origin: Option<&str>,
) -> Response {
    let mut builder = Request::builder()
        .uri(path)
        .method(if body.is_some() { "POST" } else { "GET" })
        .extension(ConnectInfo(
            "192.168.1.50:50000".parse::<SocketAddr>().unwrap(),
        ));
    if let Some(token) = token {
        builder = builder.header(AUTH_HEADER, token);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    router(state)
        .oneshot(
            builder
                .body(Body::from(body.unwrap_or("").to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn protected_endpoints_reject_missing_and_wrong_credentials() {
    for (path, body) in [
        ("/v1/info", None),
        ("/v1/pairing/check", None),
        ("/v1/pairing/status", None),
        ("/v1/pairing/enroll", Some("{}")),
        ("/v1/pairing/activate", Some("{}")),
        ("/v1/wake", Some(r#"{"mac":"00:11:22:33:44:55"}"#)),
        (
            "/v1/command",
            Some(r#"{"type":"system","action":"shutdown"}"#),
        ),
    ] {
        for token in [None, Some("wrong")] {
            assert_eq!(
                request(state(), path, token, body, None).await.status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
    }
}

#[tokio::test]
async fn qr_token_cannot_control_or_wake_but_can_poll_after_enrolling() {
    let state = state();
    let token = state.pairing.mint_pairing_session_token();
    for (path, body) in [
        ("/v1/info", None),
        (
            "/v1/command",
            Some(r#"{"type":"system","action":"shutdown"}"#),
        ),
        ("/v1/wake", Some(r#"{"mac":"00:11:22:33:44:55"}"#)),
    ] {
        assert_eq!(
            request(state.clone(), path, Some(&token), body, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let enrolled = request(
        state.clone(),
        "/v1/pairing/enroll",
        Some(&token),
        Some(r#"{"device_name":"Phone"}"#),
        None,
    )
    .await;
    assert_eq!(enrolled.status(), StatusCode::OK);
    assert_eq!(enrolled.headers()[header::CACHE_CONTROL], "no-store");
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(enrolled.into_body(), 8192).await.unwrap()).unwrap();
    let device_token = body["data"]["device_token"].as_str().unwrap();
    assert_eq!(
        request(
            state.clone(),
            "/v1/pairing/check",
            Some(device_token),
            None,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let status = request(
        state.clone(),
        &format!(
            "/v1/pairing/status?device_id={}",
            body["data"]["device_id"].as_str().unwrap()
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(status.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(body["data"]["approval"], "pending");
    assert_eq!(
        request(state, "/v1/pairing/enroll", Some(&token), Some("{}"), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn approved_device_cannot_enroll_and_revocation_takes_effect_on_next_request() {
    let state = state();
    state
        .registry
        .lock()
        .unwrap()
        .register_with_id("phone", "Phone", "device-token")
        .unwrap();
    assert_eq!(
        request(
            state.clone(),
            "/v1/pairing/check",
            Some("device-token"),
            None,
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            state.clone(),
            "/v1/pairing/enroll",
            Some("device-token"),
            Some("{}"),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    state.registry.lock().unwrap().revoke("phone").unwrap();
    assert_eq!(
        request(state, "/v1/pairing/check", Some("device-token"), None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn disabled_capabilities_reject_input_and_power_before_any_os_action() {
    for command in [
        r#"{"type":"key_press","key":"enter"}"#,
        r#"{"type":"media","action":"play_pause"}"#,
        r#"{"type":"system","action":"shutdown"}"#,
        r#"{"type":"security_screen","fallback":"lock"}"#,
    ] {
        assert_eq!(
            request(
                state(),
                "/v1/command",
                Some("approved-token"),
                Some(command),
                None
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn headless_mode_refuses_pairing_and_input_even_with_flags_enabled() {
    let mut state = state();
    state.headless = true;
    let invitation = state.pairing.mint_pairing_session_token();

    for (path, body) in [
        ("/v1/pairing/enroll", "{}"),
        ("/v1/pairing/activate", "{}"),
        ("/v1/command", r#"{"type":"key_press","key":"enter"}"#),
    ] {
        assert_eq!(
            request(
                state.clone(),
                path,
                Some(if path == "/v1/pairing/enroll" {
                    invitation.as_str()
                } else {
                    "approved-token"
                }),
                Some(body),
                None
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn browser_requests_are_blocked_and_responses_are_not_cacheable() {
    for origin in ["https://example.com", "null", "http://localhost"] {
        let response = request(
            state(),
            "/v1/pairing/enroll",
            Some("approved-token"),
            Some("{}"),
            Some(origin),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let response = request(state(), "/v1/health", None, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
}

#[tokio::test]
async fn oversized_bodies_and_unknown_commands_are_rejected() {
    assert_eq!(
        request(
            state(),
            "/v1/command",
            Some("approved-token"),
            Some(&"x".repeat(8193)),
            None
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        request(
            state(),
            "/v1/command",
            Some("approved-token"),
            Some(r#"{"type":"exec","command":"calc.exe"}"#),
            None
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn failures_across_routes_share_the_same_lockout() {
    let state = state();
    for path in ["/v1/info", "/v1/pairing/check"]
        .into_iter()
        .cycle()
        .take(8)
    {
        assert_eq!(
            request(state.clone(), path, Some("wrong"), None, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            state,
            "/v1/pairing/status",
            Some("approved-token"),
            None,
            None
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn control_characters_cannot_truncate_the_native_prompt() {
    let mut state = state();
    let shown_name = Arc::new(Mutex::new(None));
    let capture = shown_name.clone();
    state.pairing = Arc::new(PairingCoordinator::new(Some(Arc::new(move |context| {
        *capture.lock().unwrap() = context.enrollment.pending_device_name();
    }))));
    let token = state.pairing.mint_pairing_session_token();
    let response = request(
        state.clone(),
        "/v1/pairing/enroll",
        Some(&token),
        Some(r#"{"device_name":"Trusted PC\u0000hidden text"}"#),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    // The unsafe name is replaced with a peer-based label before it reaches the UI.
    assert_eq!(
        shown_name.lock().unwrap().as_deref(),
        Some("Phone at 192.168.1.50")
    );
}

#[tokio::test]
async fn permanent_shared_credentials_never_authenticate_and_permissions_are_per_phone() {
    let state = state();
    for (path, body) in [
        ("/v1/info", None),
        ("/v1/pairing/enroll", Some("{}")),
        ("/v1/command", Some(r#"{"type":"key_press","key":"enter"}"#)),
    ] {
        assert_eq!(
            request(state.clone(), path, Some("legacy-token"), body, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    state
        .registry
        .lock()
        .unwrap()
        .register_with_permissions(
            "desktop",
            "Desktop phone",
            "desktop-token",
            crate::devices::Permissions::preset(3),
        )
        .unwrap();
    assert_eq!(
        request(
            state.clone(),
            "/v1/command",
            Some("desktop-token"),
            Some(r#"{"type":"key_press","key":"enter"}"#),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            state.clone(),
            "/v1/command",
            Some("approved-token"),
            Some(r#"{"type":"key_press","key":"enter"}"#),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            state.clone(),
            "/v1/command",
            Some("desktop-token"),
            Some(r#"{"type":"system","action":"shutdown"}"#),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    state.registry.lock().unwrap().set_paused(true).unwrap();
    assert_eq!(
        request(
            state,
            "/v1/command",
            Some("desktop-token"),
            Some(r#"{"type":"key_press","key":"enter"}"#),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn http_compatibility_exposes_health_only_and_rejects_credentials() {
    let state = state();
    for (path, token, expected) in [
        ("/v1/health", None, StatusCode::OK),
        ("/v1/health", Some("approved-token"), StatusCode::FORBIDDEN),
        ("/v1/info", None, StatusCode::NOT_FOUND),
        ("/v1/pairing/enroll", None, StatusCode::NOT_FOUND),
    ] {
        let mut builder = Request::builder().uri(path);
        if let Some(token) = token {
            builder = builder.header(AUTH_HEADER, token);
        }
        let response = public_router(state.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn status_queries_are_bound_to_the_authenticated_phone_or_invitation() {
    let state = state();
    state
        .registry
        .lock()
        .unwrap()
        .register_with_id("other", "Other phone", "other-token")
        .unwrap();
    assert_eq!(
        request(
            state.clone(),
            "/v1/pairing/status?device_id=other",
            Some("approved-token"),
            None,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let invitation = state.pairing.mint_pairing_session_token();
    assert_eq!(
        request(
            state.clone(),
            "/v1/pairing/status?device_id=baseline",
            Some(&invitation),
            None,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    state.registry.lock().unwrap().set_paused(true).unwrap();
    assert_eq!(
        request(
            state,
            "/v1/pairing/enroll",
            Some(&invitation),
            Some("{}"),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn global_request_capacity_recovers_and_busy_replies_are_not_cached() {
    let state = state();
    let permits = state.requests.clone().try_acquire_many_owned(64).unwrap();
    let response = request(state.clone(), "/v1/health", None, None, None).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    drop(permits);
    assert_eq!(
        request(state, "/v1/health", None, None, None)
            .await
            .status(),
        StatusCode::OK
    );
}
