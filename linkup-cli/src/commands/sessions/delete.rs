use anyhow::{Context, bail};

use crate::{Result, commands::stop::remove_service_env, services::local_server, state};

#[derive(clap::Args)]
pub struct Args {
    #[arg(help = "Name of the session to delete")]
    name: String,
}

pub async fn run(args: &Args) -> Result<()> {
    if !local_server::is_reachable().await {
        bail!("Linkup is not running. Run 'linkup start' before deleting a session.");
    }

    let state = state::load()?;
    if state.main_session.as_deref() == Some(&args.name) {
        bail!("The main session cannot be deleted");
    }

    let session = state
        .sessions
        .get(&args.name)
        .with_context(|| format!("Session '{}' does not exist", args.name))?
        .clone();

    local_server::delete_session(&args.name)
        .await
        .with_context(|| format!("Failed to delete session '{}'", args.name))?;

    for service in &session.services {
        if let Some(directory) = &service.config.directory
            && let Err(error) = remove_service_env(directory.clone(), session.config_path.clone())
        {
            log::warn!(
                "Could not remove environment files for service '{}': {error}",
                service.config.name
            );
        }
    }

    println!("Deleted session '{}'", args.name);

    Ok(())
}
