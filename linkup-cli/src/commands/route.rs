use anyhow::{Context, anyhow};
use colored::Colorize;
use linkup::{MachineId, ServiceTarget, SessionState};
use url::Url;

use crate::{Result, services, state::State};

#[derive(clap::ValueEnum, Clone)]
pub enum TargetArg {
    Local,
    Remote,
}

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, value_name = "NAME", help = "Session to update")]
    session: Option<String>,

    target: TargetArg,

    #[arg(required_unless_present = "all")]
    service_names: Vec<String>,

    #[arg(
        short,
        long,
        help = "Route all services. Cannot be used with SERVICE_NAMES.",
        conflicts_with = "service_names"
    )]
    all: bool,
}

pub async fn route(args: &Args, machine_id: MachineId) -> Result<()> {
    if !services::local_server::is_reachable().await {
        println!(
            "{}",
            "Seems like your local Linkup server is not running. Please run 'linkup start' first."
                .yellow()
        );

        return Ok(());
    }

    let service_target = match args.target {
        TargetArg::Local => ServiceTarget::Local,
        TargetArg::Remote => ServiceTarget::Remote,
    };

    let state = State::load()?;
    let session_name = args
        .session
        .clone()
        .or_else(|| state.default_session.clone())
        .context("No default session is configured; specify one with --session")?;
    let mut session = state
        .sessions
        .get(&session_name)
        .cloned()
        .with_context(|| format!("Session '{session_name}' does not exist"))?;

    let target_map =
        set_service_targets(&mut session, &args.service_names, args.all, service_target)?;

    services::local_server::upsert_tunneled_session(
        machine_id,
        Some(session_name.clone()),
        session,
    )
    .await?;

    let name_width = target_map
        .iter()
        .map(|(service_name, _)| service_name.len())
        .max()
        .unwrap_or(0);

    println!("\nSession: {}", session_name.bold());
    for (service_name, url) in &target_map {
        println!(
            "  {:<width$}  ->  {}",
            service_name,
            url,
            width = name_width,
        );
    }

    Ok(())
}

fn set_service_targets(
    session: &mut SessionState,
    service_names: &[String],
    all: bool,
    target: ServiceTarget,
) -> Result<Vec<(String, Url)>> {
    let mut new_targets = Vec::new();

    if all {
        for service in session.services.iter_mut() {
            service.current = target.clone();

            new_targets.push((service.config.name.clone(), service.current_url()));
        }
    } else {
        for service_name in service_names {
            let service = session
                .services
                .iter_mut()
                .find(|s| s.config.name.as_str() == service_name)
                .ok_or_else(|| anyhow!("Service '{}' does not exist", service_name))?;

            service.current = target.clone();

            new_targets.push((service.config.name.clone(), service.current_url()));
        }
    }

    new_targets.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(new_targets)
}
