use axum::{
    Router,
    extract::{Request, State},
    middleware::{Next, from_fn_with_state},
    response::IntoResponse,
    routing::{any, get, post},
};
use http::{HeaderMap, StatusCode};
use linkup::{Version, VersionChannel};
use worker::console_warn;

use crate::{handlers, worker_state::WorkerState};

pub fn router(state: WorkerState) -> Router {
    Router::new()
        .route("/linkup/check", get(async || "OK"))
        .route(
            "/linkup/sessions/preview",
            post(handlers::sessions::upsert_preview),
        )
        .route(
            "/linkup/sessions/tunneled",
            post(handlers::sessions::upsert_tunneled),
        )
        .route_layer(from_fn_with_state(state.clone(), authenticate))
        // ----------------------------------------------------------------------------------------
        // --- Fallback
        // ----------------------------------------------------------------------------------------
        .fallback(any(handlers::proxy::handle_all))
        .with_state(state)
}

async fn authenticate(
    State(state): State<WorkerState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    if request.uri().path().starts_with("/linkup") {
        match headers.get("x-linkup-version") {
            Some(value) => match Version::try_from(value.to_str().unwrap()) {
                Ok(client_version) => {
                    if client_version < state.min_supported_client_version
                        && client_version.channel() != VersionChannel::Beta
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            "Your Linkup CLI is outdated, please upgrade to the latest version.",
                        )
                            .into_response();
                    }
                }
                Err(_) => {
                    return (StatusCode::UNAUTHORIZED, "Invalid x-linkup-version header.")
                        .into_response();
                }
            },
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    "No x-linkup-version header, please upgrade your Linkup CLI.",
                )
                    .into_response();
            }
        }

        let authorization = headers.get(http::header::AUTHORIZATION);
        match authorization {
            Some(token) => match token.to_str() {
                Ok(token) => {
                    let parsed_token = token.replace("Bearer ", "");
                    if parsed_token != state.cloudflare.worker_token {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                }
                Err(err) => {
                    console_warn!(
                        "Token on Authorization header contains unsupported characters: '{:?}', {}",
                        token,
                        err.to_string()
                    );

                    return StatusCode::UNAUTHORIZED.into_response();
                }
            },
            None => {
                return (StatusCode::UNAUTHORIZED, "Missing authorization header.").into_response();
            }
        }
    }

    next.run(request).await
}
