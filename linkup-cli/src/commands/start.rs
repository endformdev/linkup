use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use linkup::{MachineId, SessionState, TunnelData};

use crate::{
    Result,
    env_files::write_to_env_file,
    services::{self, local_server},
    session::{SessionRow, print_sessions_table},
    state::{self, State},
};

#[derive(clap::Args)]
pub struct Args {}

pub async fn start(_args: &Args, config_arg: Option<&Path>, machine_id: MachineId) -> Result<()> {
    if state::load().is_ok() && local_server::is_reachable().await {
        println!("Linkup is already running. Run 'linkup stop' first to restart.",);

        return Ok(());
    }

    let (_state, sessions) = load_state_and_sessions(config_arg)?;

    state::cleanup_legacy_state_files();

    services::local_server::start().await?;

    let mut tunnel_data: Option<TunnelData> = None;
    for (session_name, session) in sessions {
        let response =
            services::local_server::upsert_tunneled_session(machine_id, session_name, session)
                .await?;
        tunnel_data.get_or_insert(response.tunnel_data);
    }

    let state = state::load()?;
    restore_linkup_env(&state);
    log::info!("Finished setting up!");

    let tunnel_data = tunnel_data.context("No tunnel data returned while restoring sessions")?;
    services::cloudflared::start(&tunnel_data).await?;

    let rows = state
        .sessions
        .iter()
        .map(|(name, session)| {
            SessionRow::from_session(name, session, linkup::SessionKind::Tunneled)
        })
        .collect::<Vec<_>>();

    println!();
    print_sessions_table(&rows, state.main_session.as_deref());

    Ok(())
}

fn restore_linkup_env(state: &State) {
    for (name, session) in &state.sessions {
        if let Err(error) = set_session_env(session) {
            log::warn!("Could not restore environment files for session '{name}': {error}");
        }
    }
}

pub(crate) fn set_session_env(session: &SessionState) -> Result<()> {
    for service in &session.services {
        if let Some(directory) = &service.config.directory {
            set_service_env(directory.clone(), session.config_path.clone())?
        }
    }

    Ok(())
}

#[allow(clippy::type_complexity)]
fn load_state_and_sessions(
    config_arg: Option<&Path>,
) -> Result<(State, Vec<(Option<String>, SessionState)>)> {
    if let Ok(state) = state::load()
        && !state.sessions.is_empty()
    {
        let sessions = state
            .sessions
            .iter()
            .map(|(name, session)| (Some(name.clone()), session.clone()))
            .collect();

        return Ok((state, sessions));
    }

    let (state, session) = state::from_config(config_arg)?;
    state::save(&state)?;

    Ok((state, vec![(None, session)]))
}

fn set_service_env(directory: String, config_path: String) -> Result<()> {
    let config_dir = Path::new(&config_path)
        .parent()
        .with_context(|| format!("config_path '{directory}' does not have a parent directory"))?;

    let service_path = PathBuf::from(config_dir).join(&directory);

    let dev_env_files: Vec<_> = fs::read_dir(&service_path)
        .with_context(|| format!("Failed to read service directory {:?}", service_path))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_string_lossy().ends_with(".linkup")
                && entry.file_name().to_string_lossy().starts_with(".env.")
        })
        .collect();

    if dev_env_files.is_empty() {
        return Err(anyhow!("No dev env files found on {:?}", directory));
    }

    for dev_env_file in dev_env_files {
        let dev_env_path = dev_env_file.path();
        let env_path =
            PathBuf::from(dev_env_path.parent().unwrap()).join(dev_env_path.file_stem().unwrap());

        write_to_env_file(&directory, &dev_env_path, &env_path)?;
    }

    Ok(())
}
