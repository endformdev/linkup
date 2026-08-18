use std::fs::{self};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::env_files::clear_env_file;
use crate::state::State;
use crate::{Result, services};

#[derive(clap::Args)]
pub struct Args {}

pub fn stop(_args: &Args, clear_env: bool) -> Result<()> {
    match (State::load(), clear_env) {
        (Ok(state), true) => {
            // Reset env vars back to what they were before
            for session in state.sessions.values() {
                for service in &session.services {
                    if let Some(directory) = &service.config.directory
                        && let Err(error) =
                            remove_service_env(directory.clone(), session.config_path.clone())
                    {
                        println!(
                            "Could not remove env for service {}: {}",
                            service.config.name, error
                        );
                    }
                }
            }
        }
        (Ok(_), false) => (),
        (Err(err), _) => {
            log::warn!("Failed to fetch local state: {}", err);
        }
    }

    services::local_server::stop();
    services::cloudflared::stop();

    println!("Stopped linkup");

    Ok(())
}

fn remove_service_env(directory: String, config_path: String) -> Result<()> {
    let config_dir = Path::new(&config_path)
        .parent()
        .with_context(|| format!("config_path '{directory}' does not have a parent directory"))?;

    let service_path = PathBuf::from(config_dir).join(&directory);

    let env_files: Vec<_> = fs::read_dir(&service_path)
        .with_context(|| format!("Failed to read service directory {:?}", &service_path))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".env"))
        .collect();

    for env_file in env_files {
        let env_path = env_file.path();

        clear_env_file(&directory, &env_path)?;
    }

    Ok(())
}
