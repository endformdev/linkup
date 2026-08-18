#![allow(dead_code)]

use std::{path::PathBuf, process::Command};

use linkup::{
    Domain, LocalService, MachineId, ServiceTarget, SessionDefinition, SessionService,
    SessionState, State, TunneledSessionRequest, config::ServiceConfig,
};
use linkup_clients::WorkerClient;
use linkup_local_server::{ServerState, StateStore, dns::DnsCatalog, router};
use reqwest::Url;
use tokio::net::TcpListener;

#[derive(Debug)]
pub enum ServerKind {
    Local,
    Worker,
}

pub async fn setup_server(kind: ServerKind) -> (String, Option<StateStore>) {
    match kind {
        ServerKind::Local => {
            let worker_url = Url::parse("http://localhost").unwrap();
            let state_store =
                StateStore::in_memory(State::new(worker_url.clone(), "token123".to_string()))
                    .unwrap();
            let state = ServerState {
                https_client: linkup_clients::https_client(),
                dns_catalog: DnsCatalog::new(),
                https_certs_dir: PathBuf::default(),
                state_store: state_store.clone(),
                worker_client: WorkerClient::new(&worker_url, "token123"),
            };

            let app = router(state);

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            (format!("http://{}", addr), Some(state_store))
        }
        ServerKind::Worker => {
            if !check_worker_running() {
                panic!("Worker not running! Run npx wrangler@latest dev in the worker dir");
            }
            ("http://localhost:8787".to_string(), None)
        }
    }
}

pub async fn post(url: String, body: String) -> reqwest::Response {
    let client = reqwest::Client::new();
    client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer token123")
        // TODO(augustoccesar)[2025-02-24]: Proper test version header
        .header("x-linkup-version", "99.99.99")
        .body(body)
        .send()
        .await
        .expect("Failed to send request")
}

pub fn create_session_request(name: String, fe_location: Option<String>) -> String {
    let location = match fe_location {
        Some(location) => location,
        None => "http://example.com".to_string(),
    };
    let req = TunneledSessionRequest {
        machine_id: MachineId::generate(),
        session_name: Some(name),
        session_token: "token".to_string(),
        definition: SessionDefinition {
            domains: vec![Domain {
                domain: "example.com".to_string(),
                default_service: "frontend".to_string(),
                routes: None,
            }],
            services: vec![SessionService {
                name: "frontend".to_string(),
                location: Url::parse(&location).unwrap(),
                rewrites: None,
            }],
            cache_routes: None,
        },
    };
    serde_json::to_string(&req).unwrap()
}

pub async fn seed_session(state_store: &StateStore, name: &str, fe_url: &str) {
    let session = SessionState {
        token: "token".to_string(),
        config_path: "/tmp/linkup.yml".to_string(),
        domains: vec![Domain {
            domain: "example.com".to_string(),
            default_service: "frontend".to_string(),
            routes: None,
        }],
        services: vec![LocalService {
            current: ServiceTarget::Remote,
            config: ServiceConfig {
                name: "frontend".to_string(),
                remote: Url::parse(fe_url).unwrap(),
                local: Url::parse(fe_url).unwrap(),
                directory: None,
                rewrites: None,
                health: None,
            },
        }],
        cache_routes: None,
    };

    state_store
        .upsert_session(
            name.to_string(),
            session,
            Url::parse("https://tunnel.example.com").unwrap(),
        )
        .unwrap();
}

pub fn check_worker_running() -> bool {
    let output = Command::new("bash")
        .arg("-c")
        .arg("lsof -i tcp:8787")
        .output()
        .expect("Failed to execute command");

    output.status.success()
}
