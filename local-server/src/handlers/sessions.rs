use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use http::StatusCode;
use linkup::{
    DeleteSessionRequest, LocalTunneledSessionRequest, PreviewSessionRequest,
    SessionDetailResponse, SessionsListResponse, TunneledSessionRequest,
};
use linkup_clients::WorkerClientError;
use rand::distr::{Alphanumeric, SampleString};

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
    let state = match server_state.state_store.state() {
        Ok(state) => state,
        Err(error) => {
            return ApiError::new(
                format!("Failed to read local state: {error}"),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    };

    let session_name = match resolve_tunneled_session_name(&state, request.session_name.as_deref())
    {
        Ok(session_name) => session_name,
        Err(message) => {
            return ApiError::new(message, StatusCode::BAD_REQUEST).into_response();
        }
    };
    let mut worker_request = TunneledSessionRequest::from(&request);
    worker_request.session_name = session_name;

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
    let state = match server_state.state_store.state() {
        Ok(state) => state,
        Err(error) => {
            return ApiError::new(
                format!("Failed to read local state: {error}"),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    };

    if state.main_session.as_deref() == Some(&session_name) {
        return ApiError::new(
            "The main session cannot be deleted".to_string(),
            StatusCode::BAD_REQUEST,
        )
        .into_response();
    }

    let Some(local_session) = state.sessions.get(&session_name) else {
        return ApiError::new(
            format!("Session '{}' not found", session_name),
            StatusCode::NOT_FOUND,
        )
        .into_response();
    };

    let worker_request = DeleteSessionRequest {
        session_token: local_session.token.clone(),
    };
    match server_state
        .worker_client
        .delete_session(&session_name, &worker_request)
        .await
    {
        Ok(()) | Err(WorkerClientError::Response(StatusCode::NOT_FOUND, _)) => {}
        Err(WorkerClientError::Response(status_code, message)) => {
            return ApiError::new(message, status_code).into_response();
        }
        Err(error) => {
            return ApiError::new(
                format!("Failed to delete session from Worker: {error}"),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response();
        }
    }

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
                format!("Failed to delete session: {}", error),
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

fn resolve_tunneled_session_name(
    state: &linkup::State,
    requested_name: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(main_session_name) = &state.main_session else {
        return Ok(requested_name.map(str::to_string));
    };

    if let Some(requested_name) = requested_name
        && state.sessions.contains_key(requested_name)
    {
        return Ok(Some(requested_name.to_string()));
    }

    let requested_name = match requested_name {
        Some(requested_name) => {
            validate_requested_name(requested_name)?;
            requested_name.to_string()
        }
        None => loop {
            let generated = Alphanumeric
                .sample_string(&mut rand::rng(), 6)
                .to_lowercase();
            let session_name = format!("{main_session_name}-{generated}");
            if !state.sessions.contains_key(&session_name) {
                break generated;
            }
        },
    };

    let session_name = format!("{main_session_name}-{requested_name}");
    if session_name.len() > 63 {
        return Err(format!(
            "Session name '{session_name}' is longer than the DNS limit of 63 characters"
        ));
    }

    Ok(Some(session_name))
}

fn validate_requested_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('-')
        || name.ends_with('-')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(
            "Session names may contain only lowercase letters, numbers, and hyphens".to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    fn state_with_main_session() -> linkup::State {
        let mut state = linkup::State::new(
            Url::parse("https://worker.example.com").unwrap(),
            "token".to_string(),
        );
        state.main_session = Some("happy-cow".to_string());
        state
    }

    #[test]
    fn scopes_requested_names_to_the_main_session() {
        let name = resolve_tunneled_session_name(&state_with_main_session(), Some("agent"));

        assert_eq!(name.unwrap().as_deref(), Some("happy-cow-agent"));
    }

    #[test]
    fn generated_names_are_scoped_to_the_main_session() {
        let name = resolve_tunneled_session_name(&state_with_main_session(), None)
            .unwrap()
            .unwrap();

        assert!(name.starts_with("happy-cow-"));
        assert_eq!(name.len(), "happy-cow-".len() + 6);
    }

    #[test]
    fn keeps_the_full_name_when_restoring_an_existing_session() {
        let mut state = state_with_main_session();
        state.sessions.insert(
            "happy-cow-agent".to_string(),
            linkup::SessionState {
                token: "token".to_string(),
                config_path: "/worktree/linkup.yml".to_string(),
                services: vec![],
                domains: vec![],
                cache_routes: None,
            },
        );

        let name = resolve_tunneled_session_name(&state, Some("happy-cow-agent"));

        assert_eq!(name.unwrap().as_deref(), Some("happy-cow-agent"));
    }

    #[test]
    fn rejects_invalid_requested_names() {
        let error =
            resolve_tunneled_session_name(&state_with_main_session(), Some("Agent")).unwrap_err();

        assert_eq!(
            error,
            "Session names may contain only lowercase letters, numbers, and hyphens"
        );
    }
}
