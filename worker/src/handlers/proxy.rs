use axum::response::IntoResponse;
use http::{HeaderMap, StatusCode};
use linkup::{Session, get_additional_headers, get_target_service};
use worker::{Fetch, RequestInit, wasm_bindgen::JsValue, worker_sys::web_sys};

use crate::{http_error::HttpError, worker_state::WorkerState, ws::handle_ws_resp};

pub async fn handle_all(
    state: WorkerState,
    req: worker::Request,
) -> Result<web_sys::Response, HttpError> {
    let mut request_headers: HeaderMap = req.headers().into();
    let headers: linkup::HeaderMap = (&request_headers).into();
    let url = req.inner().url();
    let (session_name, config) = match state.session_allocator.get_request_session(&url, &headers).await {
        Ok(session) => session,
        Err(_) => {
            return Err(HttpError::new(
                "Linkup was unable to determine the session origin of the request.\nMake sure your request includes a valid session ID in the referer or tracestate headers. - Worker".to_string(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ))
        }
    };

    let target_service = match get_target_service(&url, &headers, &config, &session_name) {
        Some(result) => result,
        None => {
            return Err(HttpError::new(
                "The request belonged to a session, but there was no target for the request.\nCheck your routing rules in the linkup config for a match. - Worker".to_string(),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    let extra_headers = get_additional_headers(&url, &headers, &session_name, &target_service);
    let is_websocket = request_headers
        .get("upgrade")
        .map(|v| v == "websocket")
        .unwrap_or(false);

    // Rewrite request for the target service
    let extra_http_headers: HeaderMap = extra_headers.into();
    request_headers.extend(extra_http_headers);
    request_headers.remove(http::header::HOST);
    linkup::normalize_cookie_header(&mut request_headers);

    let mut upstream_init = RequestInit::new();
    upstream_init
        .with_method(req.method())
        .with_headers((&request_headers).into())
        .with_body(req.inner().body().map(JsValue::from));

    let upstream_request = match worker::Request::new_with_init(&target_service.url, &upstream_init)
    {
        Ok(req) => req,
        Err(e) => {
            return Err(HttpError::new(
                format!("Failed to parse request: {}", e),
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    let cacheable_req = is_cacheable_request(&upstream_request, &config);
    let cache_key = get_cache_key(&upstream_request, &session_name).unwrap_or_default();

    if cacheable_req && let Some(upstream_response) = get_cached_req(cache_key.clone()).await {
        return Ok(upstream_response.into());
    }

    let mut upstream_response = match Fetch::Request(upstream_request).send().await {
        Ok(resp) => resp,
        Err(e) => {
            return Err(HttpError::new(
                format!("Failed to fetch from target service: {}", e),
                StatusCode::BAD_GATEWAY,
            ));
        }
    };

    if is_websocket {
        return crate::into_raw_response(handle_ws_resp(upstream_response).await.into_response())
            .map_err(|e| {
                HttpError::new(
                    format!("Failed to create websocket response: {}", e),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            });
    }

    if cacheable_req {
        let cache_clone = match upstream_response.cloned() {
            Ok(resp) => resp,
            Err(e) => {
                return Err(HttpError::new(
                    format!("Failed to clone response: {}", e),
                    StatusCode::BAD_GATEWAY,
                ));
            }
        };

        if let Err(e) = set_cached_req(cache_key, cache_clone).await {
            return Err(HttpError::new(
                format!("Failed to cache response: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    }

    let mut response_headers: HeaderMap = upstream_response.headers().into();
    response_headers.extend(linkup::allow_all_cors());

    Ok(upstream_response
        .with_headers((&response_headers).into())
        .into())
}

fn is_cacheable_request(req: &worker::Request, config: &Session) -> bool {
    if req.method() != worker::Method::Get {
        return false;
    }
    if let Some(routes) = &config.cache_routes {
        let path = req.path();
        if routes.iter().any(|route| route.is_match(&path)) {
            return true;
        }
    }
    false
}

fn get_cache_key(req: &worker::Request, session_name: &str) -> Option<String> {
    let mut cache_url = req.url().ok()?;
    let curr_domain = cache_url.domain().unwrap_or("example.com");
    if cache_url
        .set_host(Some(&format!("{}.{}", session_name, curr_domain)))
        .is_err()
    {
        return None;
    }
    Some(cache_url.to_string())
}

async fn get_cached_req(cache_key: String) -> Option<worker::Response> {
    match worker::Cache::default().get(cache_key, false).await {
        Ok(Some(resp)) => Some(resp),
        _ => None,
    }
}

async fn set_cached_req(cache_key: String, resp: worker::Response) -> worker::Result<()> {
    // Avoid caching error statuses or partial content
    if resp.status_code() > 499 || resp.status_code() == 206 {
        return Ok(());
    }
    worker::Cache::default().put(cache_key, resp).await?;
    Ok(())
}
