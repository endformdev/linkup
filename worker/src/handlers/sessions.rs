use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use http::StatusCode;
use linkup::{
    DeleteSessionRequest, PREVIEW_SESSION_TOKEN, PreviewSessionRequest, Session, SessionKind,
    SessionResponse, TunneledSessionRequest, TunneledSessionResponse,
};

use crate::{
    http_error::HttpError,
    session_registry::{SessionRegistryError, derive_preview_session_name},
    tunnel,
    worker_state::WorkerState,
};

#[worker::send]
pub async fn upsert_preview(
    State(state): State<WorkerState>,
    Json(request): Json<PreviewSessionRequest>,
) -> impl IntoResponse {
    let session = match Session::new(
        SessionKind::Preview,
        PREVIEW_SESSION_TOKEN.to_string(),
        request.definition,
    ) {
        Ok(conf) => conf,
        Err(e) => {
            return HttpError::new(
                format!("Failed to parse server config: {} - Worker", e),
                StatusCode::BAD_REQUEST,
            )
            .into_response();
        }
    };

    for service in &session.services {
        if let Some(host) = service.location.host()
            && &host.to_string() == "localhost"
        {
            return HttpError::new(
                "Preview session cannot contain services pointing to localhost".to_string(),
                StatusCode::BAD_REQUEST,
            )
            .into_response();
        }
    }

    let session_name = match request.session_name {
        Some(session_name) => session_name,
        None => derive_preview_session_name(&session),
    };

    if let Err(error) = state.sessions.upsert(&session_name, &session).await {
        match error {
            SessionRegistryError::SessionNameConflict => {
                return HttpError::new("Conflict".to_string(), StatusCode::CONFLICT)
                    .into_response();
            }
            _ => {
                return HttpError::new(
                    format!("Failed to store server config: {}", error),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .into_response();
            }
        }
    }

    (StatusCode::OK, Json(SessionResponse { session_name })).into_response()
}

#[worker::send]
pub async fn upsert_tunneled(
    State(state): State<WorkerState>,
    Json(request): Json<TunneledSessionRequest>,
) -> impl IntoResponse {
    let mut session = match Session::new(
        SessionKind::Tunneled,
        request.session_token,
        request.definition,
    ) {
        Ok(conf) => conf,
        Err(e) => {
            return HttpError::new(
                format!("Failed to parse server config: {} - Worker", e),
                StatusCode::BAD_REQUEST,
            )
            .into_response();
        }
    };

    let session_name = match request.session_name {
        Some(session_name) => session_name,
        None => match state.sessions.find_available_tunneled_session_name().await {
            Ok(generated_session_name) => generated_session_name,
            Err(error) => {
                return HttpError::new(
                    format!("Failed generate new session name: {}", error),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .into_response();
            }
        },
    };

    let tunnel_data = match tunnel::upsert_tunnel(&state, &request.machine_id).await {
        Ok(data) => data,
        Err(e) => {
            return HttpError::new(
                format!("Failed to upsert tunnel: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    };

    for service in session.services.iter_mut() {
        if let Some(host) = service.location.host()
            && &host.to_string() == "localhost"
        {
            service.location = tunnel_data
                .url
                .parse()
                .expect("tunnel url should be valid URL");
        }
    }

    if let Err(error) = state.sessions.upsert(&session_name, &session).await {
        match error {
            SessionRegistryError::SessionNameConflict => {
                return HttpError::new("Conflict".to_string(), StatusCode::CONFLICT)
                    .into_response();
            }
            _ => {
                return HttpError::new(
                    format!("Failed to store server config: {}", error),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .into_response();
            }
        }
    }

    let response = TunneledSessionResponse {
        session_name,
        tunnel_data,
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[worker::send]
pub async fn delete(
    State(state): State<WorkerState>,
    Path(session_name): Path<String>,
    Json(request): Json<DeleteSessionRequest>,
) -> impl IntoResponse {
    match state
        .sessions
        .delete(&session_name, &request.session_token)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(SessionRegistryError::SessionNotFound) => {
            HttpError::new("Session not found".to_string(), StatusCode::NOT_FOUND).into_response()
        }
        Err(SessionRegistryError::SessionTokenMismatch) => HttpError::new(
            "Session token does not match".to_string(),
            StatusCode::CONFLICT,
        )
        .into_response(),
        Err(error) => HttpError::new(
            format!("Failed to delete session: {error}"),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response(),
    }
}
