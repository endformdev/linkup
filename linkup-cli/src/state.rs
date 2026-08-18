use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use linkup::{LocalService, ServiceTarget, SessionState, config::Config};
use linkup_local_server::StateStore;
use rand::distr::{Alphanumeric, SampleString};

use crate::{LINKUP_STATE_FILE, Result, config::load_config_with_override, linkup_file_path};

pub use linkup::State;

pub fn load() -> Result<State> {
    load_from_path(&state_file_path())
}

pub fn load_from_path(path: &Path) -> Result<State> {
    StateStore::load(path.to_path_buf())
        .with_context(|| format!("Failed to load state file from {path:?}"))?
        .state()
        .context("Failed to read local state")
}

pub fn from_config(config_path: Option<&Path>) -> Result<(State, SessionState)> {
    let (config, config_path) = load_config_with_override(config_path)?;
    let state = State::new(
        config.linkup.worker_url.clone(),
        config.linkup.worker_token.clone(),
    );
    let session = session_from_config(config, &config_path);

    Ok((state, session))
}

pub fn save(state: &State) -> Result<()> {
    save_to_path(state, &state_file_path())
}

pub fn save_to_path(state: &State, path: &Path) -> Result<()> {
    StateStore::create(path.to_path_buf(), state.clone())
        .with_context(|| format!("Failed to write state file to {path:?}"))?;

    Ok(())
}

pub fn session_from_config(config: Config, config_path: &Path) -> SessionState {
    let token = Alphanumeric.sample_string(&mut rand::rng(), 16);
    let services = config
        .services
        .into_iter()
        .map(|config| LocalService {
            config,
            current: ServiceTarget::Remote,
        })
        .collect();

    SessionState {
        token,
        config_path: config_path.to_string_lossy().to_string(),
        services,
        domains: config.domains,
        cache_routes: config.linkup.cache_routes,
    }
}

pub fn managed_domains(state: Option<&State>, config_path: Option<&Path>) -> Vec<String> {
    let mut domains = HashSet::new();

    if let Ok((config, _)) = load_config_with_override(config_path) {
        domains.extend(config.domains.into_iter().map(|domain| domain.domain));
    }

    if let Some(state) = state {
        domains.extend(state.domain_strings());
    }

    domains.into_iter().collect()
}

pub fn top_level_domains(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .filter(|&domain| {
            !domains
                .iter()
                .any(|other_domain| other_domain != domain && domain.ends_with(other_domain))
        })
        .cloned()
        .collect()
}

pub fn state_file_path() -> PathBuf {
    linkup_file_path(LINKUP_STATE_FILE)
}

/// Remove leftover isolated session state files (`state-*`) from previous versions.
pub fn cleanup_legacy_state_files() {
    let dir = crate::linkup_dir_path();
    let prefix = format!("{}-", LINKUP_STATE_FILE);

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(&prefix)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    const CONFIG: &str = r#"
linkup:
  worker_url: https://remote-linkup.example.com
  worker_token: test_token_123
services:
  - name: frontend
    remote: http://remote-service1.example.com
    local: http://localhost:8000
  - name: backend
    remote: http://remote-service2.example.com
    local: http://localhost:8001
    directory: ../backend
domains:
  - domain: example.com
    default_service: frontend
"#;

    #[test]
    fn config_creates_remote_session() {
        let config = serde_yaml::from_str(CONFIG).unwrap();
        let path = PathBuf::from_str("./path/to/config.yaml").unwrap();
        let session = session_from_config(config, &path);

        assert_eq!(session.config_path, "./path/to/config.yaml");
        assert_eq!(session.services.len(), 2);
        assert_eq!(session.services[0].current, ServiceTarget::Remote);
        assert_eq!(
            session.services[1].config.directory.as_deref(),
            Some("../backend")
        );
        assert_eq!(session.domains[0].domain, "example.com");
        assert!(!session.token.is_empty());
    }

    #[test]
    fn state_round_trips_multiple_sessions() {
        let config: Config = serde_yaml::from_str(CONFIG).unwrap();
        let first = session_from_config(config.clone(), Path::new("/first/linkup.yml"));
        let second = session_from_config(config, Path::new("/second/linkup.yml"));
        let mut state = State::new(
            url::Url::parse("https://remote-linkup.example.com").unwrap(),
            "token".to_string(),
        );
        state.main_session = Some("main".to_string());
        state.sessions.insert("main".to_string(), first);
        state.sessions.insert("agent".to_string(), second);

        let yaml = serde_yaml::to_string(&state).unwrap();
        let decoded: State = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(decoded.main_session.as_deref(), Some("main"));
        assert_eq!(decoded.sessions.len(), 2);
    }
}
