use std::{
    collections::{BTreeMap, HashSet},
    fmt::{self, Display, Formatter},
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Domain, SessionDefinition, SessionService, config::ServiceConfig};

pub const LOCAL_STATE_VERSION: u8 = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalState {
    pub version: u8,
    pub worker_url: Url,
    pub worker_token: String,
    pub tunnel: Option<Url>,
    pub default_session: Option<String>,
    pub sessions: BTreeMap<String, SessionState>,
}

impl LocalState {
    pub fn new(worker_url: Url, worker_token: String, tunnel: Option<Url>) -> Self {
        Self {
            version: LOCAL_STATE_VERSION,
            worker_url,
            worker_token,
            tunnel,
            default_session: None,
            sessions: BTreeMap::new(),
        }
    }

    pub fn default_session(&self) -> Option<(&str, &SessionState)> {
        let name = self.default_session.as_deref()?;
        self.sessions.get(name).map(|session| (name, session))
    }

    pub fn domain_strings(&self) -> HashSet<String> {
        self.sessions
            .values()
            .flat_map(|session| session.domains.iter())
            .map(|domain| domain.domain.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionState {
    pub token: String,
    pub config_path: String,
    pub services: Vec<LocalService>,
    pub domains: Vec<Domain>,
    #[serde(
        default,
        serialize_with = "crate::serde_ext::serialize_opt_vec_regex",
        deserialize_with = "crate::serde_ext::deserialize_opt_vec_regex"
    )]
    pub cache_routes: Option<Vec<Regex>>,
}

impl From<&SessionState> for SessionDefinition {
    fn from(session: &SessionState) -> Self {
        let services = session
            .services
            .iter()
            .map(|service| SessionService {
                name: service.config.name.clone(),
                location: service.current_url(),
                rewrites: service.config.rewrites.clone(),
            })
            .collect();

        Self {
            services,
            domains: session.domains.clone(),
            cache_routes: session.cache_routes.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalService {
    pub current: ServiceTarget,

    #[serde(flatten)]
    pub config: ServiceConfig,
}

impl LocalService {
    pub fn current_url(&self) -> Url {
        match self.current {
            ServiceTarget::Local => self.config.local.clone(),
            ServiceTarget::Remote => self.config.remote.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ServiceTarget {
    Local,
    Remote,
}

impl Display for ServiceTarget {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(formatter, "local"),
            Self::Remote => write!(formatter, "remote"),
        }
    }
}
