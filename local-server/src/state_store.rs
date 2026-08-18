use std::{fs, path::PathBuf, sync::Arc};

use linkup::{LOCAL_STATE_VERSION, LocalState, SessionState};
use tokio::sync::RwLock;
use url::Url;

#[derive(Clone)]
pub struct StateStore {
    path: Arc<PathBuf>,
    state: Arc<RwLock<LocalState>>,
}

impl StateStore {
    pub fn load(path: PathBuf) -> Result<Self, Error> {
        let content = fs::read_to_string(&path)?;
        let state: LocalState = serde_yaml::from_str(&content)?;

        if state.version != LOCAL_STATE_VERSION {
            return Err(Error::UnsupportedVersion(state.version));
        }

        Ok(Self {
            path: Arc::new(path),
            state: Arc::new(RwLock::new(state)),
        })
    }

    pub async fn upsert_session(
        &self,
        name: String,
        session: SessionState,
        tunnel_url: Url,
    ) -> Result<(), Error> {
        let mut state = self.state.write().await;

        if state.default_session.is_none() {
            state.default_session = Some(name.clone());
        }

        state.sessions.insert(name, session);
        state.tunnel_url = Some(tunnel_url);

        let yaml = serde_yaml::to_string(&*state)?;
        fs::write(self.path.as_ref(), yaml)?;

        Ok(())
    }

    pub async fn delete_session(&self, name: &str) -> Result<(), Error> {
        let mut state = self.state.write().await;
        state.sessions.remove(name);

        if state.default_session.as_deref() == Some(name) {
            state.default_session = state.sessions.keys().next().cloned();
        }

        let yaml = serde_yaml::to_string(&*state)?;
        fs::write(self.path.as_ref(), yaml)?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to access local state: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse local state: {0}")]
    Serde(#[from] serde_yaml::Error),
    #[error("Unsupported local state version {0}")]
    UnsupportedVersion(u8),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use linkup::{LocalState, MachineId};

    use super::*;

    #[tokio::test]
    async fn persists_sessions_and_keeps_the_first_as_default() {
        let path = std::env::temp_dir().join(format!("linkup-state-{}", MachineId::generate()));
        let initial = LocalState {
            version: LOCAL_STATE_VERSION,
            worker_url: Url::parse("https://worker.example.com").unwrap(),
            worker_token: "token".to_string(),
            tunnel_url: None,
            default_session: None,
            sessions: BTreeMap::new(),
        };
        fs::write(&path, serde_yaml::to_string(&initial).unwrap()).unwrap();
        let store = StateStore::load(path.clone()).unwrap();
        let session = SessionState {
            token: "session-token".to_string(),
            config_path: "/worktree/linkup.yml".to_string(),
            services: vec![],
            domains: vec![],
            cache_routes: None,
        };
        let tunnel_url = Url::parse("https://tunnel.example.com").unwrap();

        store
            .upsert_session("main".to_string(), session.clone(), tunnel_url.clone())
            .await
            .unwrap();
        store
            .upsert_session("agent".to_string(), session, tunnel_url.clone())
            .await
            .unwrap();

        let persisted: LocalState =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted.default_session.as_deref(), Some("main"));
        assert_eq!(persisted.sessions.len(), 2);
        assert_eq!(persisted.tunnel_url, Some(tunnel_url));

        store.delete_session("main").await.unwrap();
        let persisted: LocalState =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted.default_session.as_deref(), Some("agent"));
        assert!(!persisted.sessions.contains_key("main"));

        fs::remove_file(path).unwrap();
    }
}
