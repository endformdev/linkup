use std::time::Duration;

use linkup::{SessionResponse, TunneledSessionResponse, UpsertSessionRequest, Version};
use reqwest::{StatusCode, header};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("{0}")]
    UrlParse(#[from] url::ParseError),
    #[error("{0}")]
    Serde(#[from] serde_json::Error),
    #[error("request failed with status {0}: {1}")]
    Response(StatusCode, String),
    #[error("{0}")]
    InvalidVersion(#[from] linkup::VersionError),
    #[error(
        "Your Linkup worker ({worker_version}) doesn't support {feature}, which needs version {required_version} or newer. Ask your admin to run `linkup infra deploy`."
    )]
    UnsupportedByWorker {
        feature: String,
        required_version: Version,
        worker_version: Version,
    },
}

#[derive(Clone)]
pub struct WorkerClient {
    url: Url,
    inner: reqwest::Client,
}

impl WorkerClient {
    pub fn new(url: &Url, worker_token: &str) -> Self {
        let mut headers = header::HeaderMap::new();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {}", worker_token))
            .expect("token to contain only valid bytes");

        auth_value.set_sensitive(true);

        headers.insert(header::AUTHORIZATION, auth_value);
        headers.insert(
            "x-linkup-version",
            header::HeaderValue::from_static(CURRENT_VERSION),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("reqwest client to be valid and created");

        Self {
            url: url.clone(),
            inner: client,
        }
    }

    /// Version of the deployed worker. Workers deployed before the version header was added don't
    /// send it, so they are reported as the last version without it.
    pub async fn version(&self) -> Result<Version, Error> {
        let response = self
            .inner
            .get(self.url.join("/linkup/check")?)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        let version = match response.headers().get("x-linkup-worker-version") {
            Some(value) => Version::try_from(value.to_str().unwrap_or_default())?,
            None => Version::try_from(LAST_VERSION_WITHOUT_HEADER)?,
        };

        Ok(version)
    }

    /// Fails if the worker is older than `required_version`. New CLI features that rely on new
    /// worker functionality should call this first, so older workers get a clear error instead of
    /// an unexpected response.
    pub async fn require_version(
        &self,
        required_version: &str,
        feature: &str,
    ) -> Result<(), Error> {
        let required_version = Version::try_from(required_version)?;
        let worker_version = self.version().await?;

        if worker_version < required_version {
            return Err(Error::UnsupportedByWorker {
                feature: feature.to_string(),
                required_version,
                worker_version,
            });
        }

        Ok(())
    }

    pub async fn tunneled_session(
        &self,
        params: &UpsertSessionRequest,
    ) -> Result<TunneledSessionResponse, Error> {
        self.post("/linkup/v2/sessions/tunneled", params).await
    }

    pub async fn preview_session(
        &self,
        params: &UpsertSessionRequest,
    ) -> Result<SessionResponse, Error> {
        self.post("/linkup/v2/sessions/preview", params).await
    }

    // TODO(@augustoccesar)[2026-04-21]: This is the same on local_server. Can probably be combined
    async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        params: &T,
    ) -> Result<R, Error> {
        let params = serde_json::to_string(params)?;
        let endpoint = self.url.join(path)?;
        let response = self
            .inner
            .post(endpoint)
            .header("Content-Type", "application/json")
            .body(params)
            .send()
            .await?;

        if response.status().is_success() {
            let content = response.json().await?;

            Ok(content)
        } else {
            Err(Error::Response(
                response.status(),
                response.text().await.unwrap_or_else(|_| "".to_string()),
            ))
        }
    }
}

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const LAST_VERSION_WITHOUT_HEADER: &str = "4.1.1";
