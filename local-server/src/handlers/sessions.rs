use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use http::StatusCode;
use linkup::{
    LocalTunneledSessionRequest, PreviewSessionRequest, SessionDetailResponse,
    SessionsListResponse, TunneledSessionRequest,
};
use linkup_clients::WorkerClientError;

use crate::{ServerState, handlers::ApiError};

pub async fn list_sessions(State(server_state): State<ServerState>) -> impl IntoResponse {
    match server_state.state_store.list_sessions() {
        Ok(sessions) => Json(SessionsListResponse {
            sessions: HashMap::from_iter(sessions),
        })
        .into_response(),
        Err(error) => ApiError::new(
            format!("Failed to list sessions: {}", error),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response(),
    }
}

pub async fn get_session(
    State(server_state): State<ServerState>,
    Path(session_name): Path<String>,
) -> impl IntoResponse {
    match server_state.state_store.find_session(&session_name) {
        Ok(Some(session)) => Json(SessionDetailResponse {
            session_kind: session.kind,
            session_name,
            services: session.services,
            domains: session.domains,
        })
        .into_response(),
        Ok(None) => ApiError::new(
            format!("Session '{}' not found", session_name),
            StatusCode::NOT_FOUND,
        )
        .into_response(),
        Err(error) => ApiError::new(
            format!("Failed to get session: {}", error),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response(),
    }
}

pub async fn upsert_preview(
    State(server_state): State<ServerState>,
    Json(request): Json<PreviewSessionRequest>,
) -> impl IntoResponse {
    match server_state.worker_client.preview_session(&request).await {
        Ok(session_response) => Json(session_response).into_response(),
        Err(error) => match error {
            WorkerClientError::Response(status_code, message) => {
                ApiError::new(message, status_code).into_response()
            }
            _ => ApiError::new(
                format!("Failed to request to Worker: {}", error),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response(),
        },
    }
}

pub async fn upsert_tunneled(
    State(server_state): State<ServerState>,
    Json(request): Json<LocalTunneledSessionRequest>,
) -> impl IntoResponse {
    let worker_request = TunneledSessionRequest::from(&request);
    let tunneled_session = match server_state
        .worker_client
        .tunneled_session(&worker_request)
        .await
    {
        Ok(tunneled_session) => tunneled_session,
        Err(error) => match error {
            WorkerClientError::Response(StatusCode::CONFLICT, _) => {
                return ApiError::new("Conflict".to_string(), StatusCode::CONFLICT).into_response();
            }
            _ => {
                return ApiError::new(
                    format!("Failed to request to Worker: {}", error),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .into_response();
            }
        },
    };

    let tunnel_url = match tunneled_session.tunnel_data.url.parse() {
        Ok(url) => url,
        Err(error) => {
            return ApiError::new(
                format!("Worker returned an invalid tunnel URL: {error}"),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    };

    let session = match server_state.state_store.upsert_session(
        tunneled_session.session_name.clone(),
        request.session,
        tunnel_url,
    ) {
        Ok(session) => session,
        Err(error) => {
            return ApiError::new(
                format!("Failed to store local session: {error}"),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    };

    for domain in &session.domains {
        let full_domain = format!(
            "{session_name}.{domain}",
            session_name = tunneled_session.session_name,
            domain = domain.domain,
        );

        server_state.dns_catalog.register_record(&full_domain).await;
    }

    (StatusCode::OK, Json(tunneled_session)).into_response()
}

pub async fn delete_session(
    State(server_state): State<ServerState>,
    Path(session_name): Path<String>,
) -> impl IntoResponse {
    let session = match server_state.state_store.delete_session(&session_name) {
        Ok(None) => {
            return ApiError::new(
                format!("Session '{}' not found", session_name),
                StatusCode::NOT_FOUND,
            )
            .into_response();
        }
        Err(error) => {
            return ApiError::new(
                format!("Failed to find session: {}", error),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
        Ok(Some(session)) => session,
    };

    for domain in &session.domains {
        let full_domain = format!("{session_name}.{domain}", domain = domain.domain);

        server_state
            .dns_catalog
            .deregister_record(&full_domain)
            .await;
    }

    StatusCode::NO_CONTENT.into_response()
}
