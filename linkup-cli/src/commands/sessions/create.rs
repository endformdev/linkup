use std::{collections::HashSet, path::Path};

use anyhow::{Context, bail};
use linkup::{MachineId, ServiceTarget, SessionKind};

use crate::{
    Result,
    commands::start::set_session_env,
    config::load_config_with_override,
    services::local_server,
    session::{SessionRow, print_sessions_table},
    state::{self, State, session_from_config},
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, value_name = "NAME", help = "Request a specific session name")]
    name: Option<String>,

    #[arg(
        long,
        value_name = "SERVICE",
        help = "Initially route a service locally",
        conflicts_with = "all_local"
    )]
    local: Vec<String>,

    #[arg(long, help = "Initially route every service locally")]
    all_local: bool,
}

pub async fn run(args: &Args, config_arg: Option<&Path>, machine_id: MachineId) -> Result<()> {
    if !local_server::is_reachable().await {
        bail!("Linkup is not running. Run 'linkup start' before creating another session.");
    }

    let state = state::load()?;
    let (config, config_path) = load_config_with_override(config_arg)?;

    if config.linkup.worker_url != state.worker_url {
        bail!(
            "The session config uses worker '{}', but Linkup is connected to '{}'",
            config.linkup.worker_url,
            state.worker_url
        );
    }

    ensure_domains_are_managed(&state, &config.domains)?;

    let mut session = session_from_config(config, &config_path);
    set_initial_routes(&mut session, &args.local, args.all_local)?;

    let response =
        local_server::upsert_tunneled_session(machine_id, args.name.clone(), session.clone())
            .await
            .context("Failed to create tunneled session")?;

    set_session_env(&session)?;

    let row = SessionRow::from_session(&response.session_name, &session, SessionKind::Tunneled);
    let state = state::load()?;

    println!();
    print_sessions_table(&[row], state.main_session.as_deref());

    Ok(())
}

fn set_initial_routes(
    session: &mut linkup::SessionState,
    local_services: &[String],
    all_local: bool,
) -> Result<()> {
    if all_local {
        for service in &mut session.services {
            service.current = ServiceTarget::Local;
        }

        return Ok(());
    }

    for service_name in local_services {
        let service = session
            .services
            .iter_mut()
            .find(|service| service.config.name == *service_name)
            .with_context(|| format!("Service '{service_name}' does not exist"))?;

        service.current = ServiceTarget::Local;
    }

    Ok(())
}

fn ensure_domains_are_managed(state: &State, domains: &[linkup::Domain]) -> Result<()> {
    let managed = state.domain_strings();
    let requested = domains
        .iter()
        .map(|domain| domain.domain.clone())
        .collect::<HashSet<_>>();
    let unmanaged = requested.difference(&managed).cloned().collect::<Vec<_>>();

    if !unmanaged.is_empty() {
        bail!(
            "The running Linkup instance does not manage these domains: {}",
            unmanaged.join(", ")
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use linkup::{LocalService, SessionState, config::ServiceConfig};
    use url::Url;

    use super::*;

    fn session() -> SessionState {
        SessionState {
            token: "token".to_string(),
            config_path: "/tmp/linkup.yml".to_string(),
            services: ["frontend", "api"]
                .into_iter()
                .map(|name| LocalService {
                    current: ServiceTarget::Remote,
                    config: ServiceConfig {
                        name: name.to_string(),
                        remote: Url::parse(&format!("https://{name}.example.com")).unwrap(),
                        local: Url::parse("http://localhost:3000").unwrap(),
                        directory: None,
                        rewrites: None,
                        health: None,
                    },
                })
                .collect(),
            domains: vec![],
            cache_routes: None,
        }
    }

    #[test]
    fn selected_services_start_local() {
        let mut session = session();

        set_initial_routes(&mut session, &["api".to_string()], false).unwrap();

        assert_eq!(session.services[0].current, ServiceTarget::Remote);
        assert_eq!(session.services[1].current, ServiceTarget::Local);
    }

    #[test]
    fn all_services_can_start_local() {
        let mut session = session();

        set_initial_routes(&mut session, &[], true).unwrap();

        assert!(
            session
                .services
                .iter()
                .all(|service| service.current == ServiceTarget::Local)
        );
    }
}
