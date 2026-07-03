use axum::{
    body::Body,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use http::{HeaderMap, HeaderName, StatusCode, Uri};
use hyper::body::Incoming;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use linkup::{TargetService, get_additional_headers, get_target_service};
use tokio::io::copy_bidirectional;

use crate::{AxumHttpsClient, ServerState, handlers::ApiError};

pub async fn handle_all(State(server_state): State<ServerState>, req: Request) -> Response {
    let headers: linkup::HeaderMap = req.headers().into();
    let url = if req.uri().scheme().is_some() {
        req.uri().to_string()
    } else {
        format!(
            "http://{}{}",
            req.headers()
                .get(http::header::HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("localhost"),
            req.uri()
        )
    };

    let (session_name, config) = match server_state.session_allocator.get_request_session(&url, &headers).await {
        Ok(session) => session,
        Err(_) => {
            return ApiError::new(
                "Linkup was unable to determine the session origin of the request. Ensure that your request includes a valid session identifier in the referer or tracestate headers. - Local Server".to_string(),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
                .into_response()
        }
    };

    let target_service = match get_target_service(&url, &headers, &config, &session_name) {
        Some(result) => result,
        None => {
            return ApiError::new(
                "The request belonged to a session, but there was no target for the request. Check that the routing rules in your linkup config have a match for this request. - Local Server".to_string(),
                StatusCode::NOT_FOUND,
            )
                .into_response()
        }
    };

    let extra_headers = get_additional_headers(&url, &headers, &session_name, &target_service);

    handle_http_req(
        req,
        target_service,
        extra_headers,
        server_state.https_client,
        server_state.upgrade_client,
    )
    .await
}

const DISALLOWED_HEADERS: [HeaderName; 2] = [
    HeaderName::from_static("content-encoding"),
    HeaderName::from_static("content-length"),
];

async fn handle_http_req(
    mut req: Request,
    target_service: TargetService,
    extra_headers: linkup::HeaderMap,
    client: AxumHttpsClient,
    upgrade_client: AxumHttpsClient,
) -> Response {
    let is_upgrade_request = req.headers().contains_key(http::header::UPGRADE)
        && header_contains_token(req.headers(), http::header::CONNECTION, "upgrade");

    let downstream_upgrade = if is_upgrade_request && req.extensions().get::<OnUpgrade>().is_some()
    {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };

    *req.uri_mut() = Uri::try_from(&target_service.url).unwrap();
    let extra_http_headers: HeaderMap = extra_headers.into();
    req.headers_mut().extend(extra_http_headers);
    // Request uri and host headers should not conflict
    req.headers_mut().remove(http::header::HOST);
    linkup::normalize_cookie_header(req.headers_mut());

    if downstream_upgrade.is_some() || target_service.url.starts_with("http://") {
        *req.version_mut() = http::Version::HTTP_11;
    }

    // Send the modified request to the target service.
    let upstream_client = if downstream_upgrade.is_some() {
        upgrade_client
    } else {
        client
    };

    let mut resp = match upstream_client.request(req).await {
        Ok(resp) => resp,
        Err(e) => {
            return ApiError::new(
                format!(
                    "Failed to proxy request - are all your servers started? {}",
                    e
                ),
                StatusCode::BAD_GATEWAY,
            )
            .into_response();
        }
    };

    if let Some(downstream_upgrade) = downstream_upgrade
        && resp.status() == StatusCode::SWITCHING_PROTOCOLS
    {
        return handle_upgrade_response(resp, downstream_upgrade);
    }

    resp.headers_mut().extend(linkup::allow_all_cors());

    resp.into_response()
}

fn handle_upgrade_response(
    mut upstream_resp: http::Response<Incoming>,
    downstream_upgrade: OnUpgrade,
) -> Response {
    let upstream_upgrade = hyper::upgrade::on(&mut upstream_resp);
    spawn_upgrade_tunnel(downstream_upgrade, upstream_upgrade);

    let (parts, _) = upstream_resp.into_parts();
    let mut downstream_resp = Response::from_parts(parts, Body::empty());

    for header in &DISALLOWED_HEADERS {
        downstream_resp.headers_mut().remove(header);
    }
    downstream_resp
        .headers_mut()
        .extend(linkup::allow_all_cors());

    downstream_resp
}

fn spawn_upgrade_tunnel(downstream_upgrade: OnUpgrade, upstream_upgrade: OnUpgrade) {
    tokio::spawn(async move {
        let downstream = match downstream_upgrade.await {
            Ok(upgraded) => upgraded,
            Err(error) => {
                eprintln!("Failed to upgrade downstream connection: {error}");
                return;
            }
        };

        let upstream = match upstream_upgrade.await {
            Ok(upgraded) => upgraded,
            Err(error) => {
                eprintln!("Failed to upgrade upstream connection: {error}");
                return;
            }
        };

        let mut downstream = TokioIo::new(downstream);
        let mut upstream = TokioIo::new(upstream);

        if let Err(error) = copy_bidirectional(&mut downstream, &mut upstream).await {
            eprintln!("Error proxying upgraded connection: {error}");
        }
    });
}

fn header_contains_token(headers: &HeaderMap, header: HeaderName, token: &str) -> bool {
    headers.get_all(header).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
    })
}
