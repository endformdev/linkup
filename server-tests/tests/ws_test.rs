use std::str::FromStr;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use helpers::ServerKind;
use http::{HeaderName, HeaderValue, StatusCode};
use tokio::net::TcpListener;

use crate::helpers::{seed_session, setup_server};

mod helpers;

#[tokio::test]
async fn can_request_underlying_websocket_server() {
    let (url, allocator) = setup_server(ServerKind::Local).await;
    let ws_url = setup_websocket_server().await;

    seed_session(allocator.as_ref().unwrap(), "ws-session", &ws_url).await;

    let uri = http::Uri::from_str(url.as_str()).unwrap();
    let req = http::Request::builder()
        .uri(format!("ws://{}/ws", uri.authority().unwrap()))
        .header("referer", "example.com")
        .header("traceparent", "xzyabc")
        .header("tracestate", "linkup-session=ws-session")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-version", "13")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("host", uri.authority().unwrap().to_string())
        .body(())
        .unwrap();

    let (mut ws_stream, ws_resp) = tokio_tungstenite::connect_async(req)
        .await
        .expect("Failed to connect to WebSocket server");

    assert_eq!(ws_resp.status(), 101);
    assert_eq!(
        ws_resp.headers().get("my-special-header"),
        Some(&HeaderValue::from_str("special-value").unwrap())
    );

    let msg = "Hello, WebSocket!";
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
        .await
        .expect("Failed to send message");

    match ws_stream.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
            assert_eq!(text, msg);
        }
        anything_else => {
            println!("{:?}", anything_else);
            panic!("Failed to receive echoed message")
        }
    }

    let payload = vec![1, 2, 3, 4];
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            payload.clone().into(),
        ))
        .await
        .expect("Failed to send binary message");

    match ws_stream.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes))) => {
            assert_eq!(bytes.as_ref(), payload.as_slice());
        }
        anything_else => {
            println!("{:?}", anything_else);
            panic!("Failed to receive echoed binary message")
        }
    }

    let ping = vec![5, 6, 7, 8];
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Ping(
            ping.clone().into(),
        ))
        .await
        .expect("Failed to send ping");

    match ws_stream.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(bytes))) => {
            assert_eq!(bytes.as_ref(), ping.as_slice());
        }
        anything_else => {
            println!("{:?}", anything_else);
            panic!("Failed to receive pong")
        }
    }

    ws_stream
        .close(None)
        .await
        .expect("Failed to close WebSocket");

    match ws_stream.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
            println!("Received close frame from server: {:?}", frame);
        }
        None => {
            println!("Connection closed without explicit close frame from server");
        }
        other => {
            panic!(
                "Expected a close frame or stream termination, but got: {:?}",
                other
            );
        }
    }
}

#[tokio::test]
async fn forwards_failed_websocket_upgrade_response() {
    let (url, allocator) = setup_server(ServerKind::Local).await;
    let ws_url = setup_rejecting_server().await;

    seed_session(allocator.as_ref().unwrap(), "ws-rejected-session", &ws_url).await;

    let uri = http::Uri::from_str(url.as_str()).unwrap();
    let req = http::Request::builder()
        .uri(format!("ws://{}/ws", uri.authority().unwrap()))
        .header("referer", "example.com")
        .header("tracestate", "linkup-session=ws-rejected-session")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-version", "13")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("host", uri.authority().unwrap().to_string())
        .body(())
        .unwrap();

    match tokio_tungstenite::connect_async(req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        other => panic!("Expected upstream HTTP rejection, got: {other:?}"),
    }
}

async fn websocket_echo(ws: WebSocketUpgrade) -> impl IntoResponse {
    let mut response = ws.on_upgrade(handle_websocket);
    response.headers_mut().append(
        HeaderName::from_str("my-special-header").unwrap(),
        HeaderValue::from_str("special-value").unwrap(),
    );

    response
}

async fn handle_websocket(mut socket: WebSocket) {
    println!("WebSocket connected");
    while let Some(result) = socket.recv().await {
        match result {
            Ok(msg) => {
                println!("Received message: {:?}", msg);
                match msg {
                    Message::Text(text) => {
                        if let Err(e) = socket.send(Message::Text(text)).await {
                            println!("Failed to send message: {:?}", e);
                            break;
                        }
                    }
                    Message::Binary(bytes) => {
                        if let Err(e) = socket.send(Message::Binary(bytes)).await {
                            println!("Failed to send message: {:?}", e);
                            break;
                        }
                    }
                    Message::Ping(bytes) => {
                        if let Err(e) = socket.send(Message::Pong(bytes)).await {
                            println!("Failed to send message: {:?}", e);
                            break;
                        }
                    }
                    Message::Close(_) => {
                        println!("Received close on server, closing socket");
                        if let Err(e) = socket.close().await {
                            println!("Failed to close: {:?}", e);
                        }
                        break;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                println!("WebSocket error: {:?}", e);
                break;
            }
        }
    }
    println!("WebSocket disconnected");
}

async fn setup_websocket_server() -> String {
    let app = Router::new().route("/ws", axum::routing::get(websocket_echo));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{}", addr)
}

async fn setup_rejecting_server() -> String {
    let app = Router::new().route(
        "/ws",
        axum::routing::get(|| async { StatusCode::BAD_REQUEST }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{}", addr)
}
