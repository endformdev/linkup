use std::{
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use linkup::{
    ConfigError, HeaderMap, STATE_VERSION, Session, SessionState, State, session_names_from_request,
};
use url::Url;

#[derive(Clone)]
pub struct StateStore {
    path: Option<Arc<PathBuf>>,
    state: Arc<RwLock<State>>,
}

impl StateStore {
    pub fn load(path: PathBuf) -> Result<Self, StateStoreError> {
        let content = fs::read_to_string(&path)?;
        let state: State = serde_yaml::from_str(&content)?;
        Self::validate_version(&state)?;

        Ok(Self {
            path: Some(Arc::new(path)),
            state: Arc::new(RwLock::new(state)),
        })
    }

    pub fn create(path: PathBuf, state: State) -> Result<Self, StateStoreError> {
        Self::validate_version(&state)?;
        let store = Self::in_memory(state)?;
        let store = Self {
            path: Some(Arc::new(path)),
            ..store
        };
        store.persist(&store.state()?)?;

        Ok(store)
    }

    pub fn in_memory(state: State) -> Result<Self, StateStoreError> {
        Self::validate_version(&state)?;

        Ok(Self {
            path: None,
            state: Arc::new(RwLock::new(state)),
        })
    }

    pub fn state(&self) -> Result<State, StateStoreError> {
        Ok(self
            .state
            .read()
            .map_err(|_| StateStoreError::LockPoisoned)?
            .clone())
    }

    pub fn upsert_session(
        &self,
        name: String,
        session: SessionState,
        tunnel_url: Url,
    ) -> Result<Session, StateStoreError> {
        let proxy_session = Session::try_from(&session)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| StateStoreError::LockPoisoned)?;
        let mut next = state.clone();

        if next.default_session.is_none() {
            next.default_session = Some(name.clone());
        }

        next.sessions.insert(name, session);
        next.tunnel_url = Some(tunnel_url);
        self.persist(&next)?;
        *state = next;

        Ok(proxy_session)
    }

    pub fn delete_session(&self, name: &str) -> Result<Option<Session>, StateStoreError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| StateStoreError::LockPoisoned)?;
        let Some(session) = state.sessions.get(name) else {
            return Ok(None);
        };
        let proxy_session = Session::try_from(session)?;
        let mut next = state.clone();
        next.sessions.remove(name);

        if next.default_session.as_deref() == Some(name) {
            next.default_session = next.sessions.keys().next().cloned();
        }

        self.persist(&next)?;
        *state = next;

        Ok(Some(proxy_session))
    }

    pub fn find_session(&self, name: &str) -> Result<Option<Session>, StateStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| StateStoreError::LockPoisoned)?;
        state
            .sessions
            .get(name)
            .map(Session::try_from)
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_sessions(&self) -> Result<Vec<(String, Session)>, StateStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| StateStoreError::LockPoisoned)?;

        state
            .sessions
            .iter()
            .map(|(name, session)| Ok((name.clone(), Session::try_from(session)?)))
            .collect()
    }

    pub fn get_request_session(
        &self,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<(String, Session), StateStoreError> {
        for name in session_names_from_request(url, headers) {
            if let Some(session) = self.find_session(&name)? {
                return Ok((name, session));
            }
        }

        Err(StateStoreError::NoSessionForRequest(url.to_string()))
    }

    fn validate_version(state: &State) -> Result<(), StateStoreError> {
        if state.version != STATE_VERSION {
            return Err(StateStoreError::UnsupportedVersion(state.version));
        }

        Ok(())
    }

    fn persist(&self, state: &State) -> Result<(), StateStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let yaml = serde_yaml::to_string(state)?;
        fs::write(path.as_ref(), yaml)?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("Failed to access local state: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse local state: {0}")]
    Serde(#[from] serde_yaml::Error),
    #[error("Unsupported local state version {0}")]
    UnsupportedVersion(u8),
    #[error("Local state lock was poisoned")]
    LockPoisoned,
    #[error("Invalid session in local state: {0}")]
    Config(#[from] ConfigError),
    #[error("No session found for request {0}")]
    NoSessionForRequest(String),
}

#[cfg(test)]
mod tests {
    use linkup::{Domain, LocalService, MachineId, ServiceTarget, config::ServiceConfig};

    use super::*;

    fn empty_state() -> State {
        State::new(
            Url::parse("https://worker.example.com").unwrap(),
            "token".to_string(),
        )
    }

    #[test]
    fn persists_sessions_and_keeps_the_first_as_default() {
        let path = std::env::temp_dir().join(format!("linkup-state-{}", MachineId::generate()));
        let store = StateStore::create(path.clone(), empty_state()).unwrap();
        let session = SessionState {
            token: "session-token".to_string(),
            config_path: "/worktree/linkup.yml".to_string(),
            services: vec![LocalService {
                current: ServiceTarget::Remote,
                config: ServiceConfig {
                    name: "frontend".to_string(),
                    remote: Url::parse("https://frontend.example.com").unwrap(),
                    local: Url::parse("http://localhost:3000").unwrap(),
                    directory: None,
                    rewrites: None,
                    health: None,
                },
            }],
            domains: vec![Domain {
                domain: "example.com".to_string(),
                default_service: "frontend".to_string(),
                routes: None,
            }],
            cache_routes: None,
        };
        let tunnel_url = Url::parse("https://tunnel.example.com").unwrap();

        store
            .upsert_session("main".to_string(), session.clone(), tunnel_url.clone())
            .unwrap();
        store
            .upsert_session("agent".to_string(), session, tunnel_url.clone())
            .unwrap();

        let persisted = StateStore::load(path.clone()).unwrap().state().unwrap();
        assert_eq!(persisted.default_session.as_deref(), Some("main"));
        assert_eq!(persisted.sessions.len(), 2);
        assert_eq!(persisted.tunnel_url, Some(tunnel_url));

        store.delete_session("main").unwrap();
        let persisted = StateStore::load(path.clone()).unwrap().state().unwrap();
        assert_eq!(persisted.default_session.as_deref(), Some("agent"));
        assert!(!persisted.sessions.contains_key("main"));

        fs::remove_file(path).unwrap();
    }
}
