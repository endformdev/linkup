use linkup::{ConfigError, HeaderMap, Session, session_names_from_request};
use worker::kv::KvStore;

use crate::name_gen::{random_animal, random_six_char};

#[derive(Clone)]
pub struct WorkerSessionRegistry {
    kv: KvStore,
}

impl WorkerSessionRegistry {
    pub fn new(kv: KvStore) -> Self {
        Self { kv }
    }

    pub async fn session_for_request(
        &self,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<(String, Session), SessionRegistryError> {
        for name in session_names_from_request(url, headers) {
            if let Some(session) = self.find(&name).await? {
                return Ok((name, session));
            }
        }

        Err(SessionRegistryError::NoSessionForRequest(url.to_string()))
    }

    pub async fn upsert<'name>(
        &self,
        name: &'name str,
        session: &Session,
    ) -> Result<&'name str, SessionRegistryError> {
        if name.is_empty() {
            return Err(SessionRegistryError::EmptySessionName);
        }

        let existing_session = self.find(name).await?;
        validate_name_reuse(existing_session.as_ref(), session)?;

        let serialized_session = serde_json::to_string(session)
            .map_err(|error| SessionRegistryError::InvalidSession(error.to_string()))?;
        self.write(name, &serialized_session).await?;

        Ok(name)
    }

    pub async fn find(&self, name: &str) -> Result<Option<Session>, SessionRegistryError> {
        let Some(value) = self.read(name).await? else {
            return Ok(None);
        };
        let session: serde_json::Value = serde_json::from_str(&value)
            .map_err(|error| SessionRegistryError::InvalidSession(error.to_string()))?;
        Session::try_from(session)
            .map(Some)
            .map_err(|error: ConfigError| SessionRegistryError::InvalidSession(error.to_string()))
    }

    pub async fn find_available_tunneled_session_name(
        &self,
    ) -> Result<String, SessionRegistryError> {
        let mut animal_attempts = 0;

        loop {
            let candidate = if animal_attempts < 20 {
                animal_attempts += 1;
                random_animal()
            } else {
                random_six_char()
            };

            if self.read(&candidate).await?.is_none() {
                return Ok(candidate);
            }
        }
    }

    async fn read(&self, name: &str) -> Result<Option<String>, SessionRegistryError> {
        self.kv
            .get(name)
            .text()
            .await
            .map_err(|error| SessionRegistryError::Read(error.to_string()))
    }

    async fn write(&self, name: &str, session: &str) -> Result<(), SessionRegistryError> {
        let mut put = self
            .kv
            .put(name, session)
            .map_err(|error| SessionRegistryError::Write(error.to_string()))?;

        // Default to expiring sessions after 7 days of inactivity.
        put = put.expiration_ttl(60 * 60 * 24 * 7);
        put.execute()
            .await
            .map_err(|error| SessionRegistryError::Write(error.to_string()))
    }
}

pub(crate) fn derive_preview_session_name(session: &Session) -> String {
    session.sha()[..6].to_string()
}

fn validate_name_reuse(
    existing: Option<&Session>,
    incoming: &Session,
) -> Result<(), SessionRegistryError> {
    if let Some(existing) = existing
        && existing.session_token != incoming.session_token
    {
        return Err(SessionRegistryError::SessionNameConflict);
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SessionRegistryError {
    #[error("Could not read session: {0}")]
    Read(String),
    #[error("Could not write session: {0}")]
    Write(String),
    #[error("Invalid session data: {0}")]
    InvalidSession(String),
    #[error("Session name is empty")]
    EmptySessionName,
    #[error("Session with name already exists")]
    SessionNameConflict,
    #[error("No session found for request {0}")]
    NoSessionForRequest(String),
}

#[cfg(test)]
mod tests {
    use linkup::{PREVIEW_SESSION_TOKEN, SessionDefinition, SessionKind};

    use super::*;

    fn preview_session() -> Session {
        let definition = serde_json::json!({
            "services": [
                {
                    "name": "frontend",
                    "location": "https://frontend.example.com"
                },
                {
                    "name": "backend",
                    "location": "https://backend.example.com"
                }
            ],
            "domains": [
                {
                    "domain": "example.com",
                    "default_service": "frontend",
                    "routes": [
                        {
                            "path": "^/api(?:/|$)",
                            "service": "backend"
                        }
                    ]
                }
            ],
            "cache_routes": null
        });

        Session::new(
            SessionKind::Preview,
            PREVIEW_SESSION_TOKEN.to_string(),
            serde_json::from_value::<SessionDefinition>(definition).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn identical_preview_sessions_get_the_same_name() {
        let first = preview_session();
        let mut second = first.clone();
        second.services.reverse();

        let first_name = derive_preview_session_name(&first);
        let second_name = derive_preview_session_name(&second);

        assert_eq!(first_name.len(), 6);
        assert_eq!(first_name, second_name);
    }

    #[test]
    fn rejects_reusing_a_name_with_another_token() {
        let first = preview_session();
        let mut second = first.clone();
        second.session_token = "another-token".to_string();

        let error = validate_name_reuse(Some(&first), &second).unwrap_err();

        assert!(matches!(error, SessionRegistryError::SessionNameConflict));
    }
}
