use std::{
    env,
    fs::File,
    os::unix::process::CommandExt,
    process::{self, Stdio},
    time::Duration,
};

use anyhow::Context;
use reqwest::StatusCode;
use sysinfo::Pid;
use tokio::time::sleep;
use url::Url;

use linkup::{
    MachineId, SessionDefinition, SessionKind, TunnelData, TunneledSessionRequest,
    TunneledSessionResponse,
};
use linkup_clients::{LocalServerClient, LocalServerClientError};

use super::{PidError, ServiceId};
use crate::{Result, linkup_certs_dir_path, linkup_file_path, state::State};

const ID: ServiceId = ServiceId("linkup-local-server");
const NAME: &str = "Linkup local server";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed while handing file: {0}")]
    FileHandling(#[from] std::io::Error),
    #[error("Failed to stop pid: {0}")]
    StoppingPid(#[from] PidError),
    #[error("Failed to reach the local server")]
    ServerUnreachable,
}

pub fn url() -> Url {
    Url::parse("http://localhost:80").expect("linkup url invalid")
}

pub async fn start() -> Result<()> {
    if super::find_pid(ID).is_some() {
        log::info!("Already running.");

        return Ok(());
    }

    log::info!("Starting...");
    spawn_process()?;

    let mut reachable = is_reachable().await;
    let mut attempts: u8 = 0;
    loop {
        match (reachable, attempts) {
            (true, _) => break,
            (false, 0..10) => {
                sleep(Duration::from_millis(1000)).await;
                attempts += 1;

                log::info!("Waiting for server... retry #{attempts}");

                reachable = is_reachable().await;
            }
            (false, 10..) => {
                log::error!("Failed to reach server");

                return Err(Error::ServerUnreachable.into());
            }
        }
    }
    log::info!("Ready!");

    Ok(())
}

pub fn stop() {
    super::stop(ID);
}

pub fn find_pid() -> Option<Pid> {
    super::find_pid(ID)
}

pub async fn is_reachable() -> bool {
    matches!(
        LocalServerClient::new(&url()).health_check().await,
        Ok(true)
    )
}

pub async fn update_state(state: &mut State, machine_id: MachineId) -> Result<TunnelData> {
    log::info!("Uploading state to server...");
    let tunneled_session = upload_tunneled_state(state, machine_id).await?;

    log::info!("Updating local state file...");
    state.linkup.session_name = tunneled_session.session_name;
    state.linkup.kind = SessionKind::Tunneled;
    state
        .save()
        .expect("failed to update local state file with session name");

    Ok(tunneled_session.tunnel_data)
}

async fn upload_tunneled_state(
    state: &State,
    machine_id: MachineId,
) -> Result<TunneledSessionResponse> {
    let local_server_client = LocalServerClient::new(&url());
    let definition: SessionDefinition = state.into();
    let session_name =
        (!state.linkup.session_name.is_empty()).then(|| state.linkup.session_name.clone());
    let request = TunneledSessionRequest {
        machine_id,
        session_name,
        session_token: state.linkup.session_token.clone(),
        definition,
    };

    let session_response = local_server_client.tunneled_session(&request).await;

    let session_response = match session_response {
        Ok(session_response) => session_response,
        Err(LocalServerClientError::Response(StatusCode::CONFLICT, _)) => {
            log::debug!(
                "Requested name from state file already exists, attempting to create with a new name"
            );

            let unnamed_request = TunneledSessionRequest {
                machine_id,
                session_name: None,
                session_token: request.session_token,
                definition: request.definition,
            };

            local_server_client
                .tunneled_session(&unnamed_request)
                .await?
        }
        Err(error) => return Err(error.into()),
    };

    Ok(session_response)
}

fn spawn_process() -> Result<()> {
    log::debug!("Starting {}", NAME);

    let stdout_file = File::create(linkup_file_path("localserver-stdout"))?;
    let stderr_file = File::create(linkup_file_path("localserver-stderr"))?;

    let mut command =
        process::Command::new(env::current_exe().context("Failed to get the current executable")?);
    command.env(
        "RUST_LOG",
        "info,hickory_server=warn,hyper_util=warn,h2=warn,tower_http=info",
    );
    command.env("LINKUP_SERVICE_ID", ID.to_string());
    command.args([
        "server",
        "--certs-dir",
        linkup_certs_dir_path().to_str().unwrap(),
    ]);

    command
        .process_group(0)
        .stdout(stdout_file)
        .stderr(stderr_file)
        .stdin(Stdio::null())
        .spawn()?;

    Ok(())
}
