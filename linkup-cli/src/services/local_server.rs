use std::{
    env,
    fs::File,
    os::unix::process::CommandExt,
    process::{self, Stdio},
    time::Duration,
};

use anyhow::Context;
use sysinfo::Pid;
use tokio::time::sleep;
use url::Url;

use linkup::{LocalTunneledSessionRequest, MachineId, SessionState, TunneledSessionResponse};
use linkup_clients::LocalServerClient;

use super::{PidError, ServiceId};
use crate::{Result, linkup_certs_dir_path, linkup_file_path};

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

pub async fn upsert_tunneled_session(
    machine_id: MachineId,
    session_name: Option<String>,
    session: SessionState,
) -> Result<TunneledSessionResponse> {
    log::info!("Uploading session to server...");
    let local_server_client = LocalServerClient::new(&url());
    let request = LocalTunneledSessionRequest {
        machine_id,
        session_name,
        session,
    };

    Ok(local_server_client.tunneled_session(&request).await?)
}

pub async fn delete_session(session_name: &str) -> Result<()> {
    log::info!("Deleting session...");
    let local_server_client = LocalServerClient::new(&url());

    Ok(local_server_client.delete_session(session_name).await?)
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
