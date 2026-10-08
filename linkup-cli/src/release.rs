mod github {
    use std::{env, fs, path::PathBuf, time::Duration};

    use flate2::read::GzDecoder;
    use linkup::VersionError;
    use reqwest::header::HeaderValue;
    use serde::{Deserialize, Serialize, de::DeserializeOwned};
    use tar::Archive;
    use url::Url;

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("ReqwestError: {0}")]
        Reqwest(#[from] reqwest::Error),
        #[error("IoError: {0}")]
        Io(#[from] std::io::Error),
        #[error("File missing from downloaded compressed archive")]
        MissingBinary,
        #[error("Release have an invalid tag")]
        InvalidVersionTag(#[from] VersionError),
        #[error("Hit a rate limit while checking for updates")]
        RateLimit(u64),
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Release {
        #[serde(rename = "name")]
        pub version: String,
        pub assets: Vec<Asset>,
    }

    impl Release {
        /// Examples of Linkup asset files:
        /// - linkup-x86_64-apple-darwin.tar.gz
        /// - linkup-aarch64-apple-darwin.tar.gz
        /// - linkup-x86_64-unknown-linux-gnu.tar.gz
        /// - linkup-aarch64-unknown-linux-gnu.tar.gz
        pub fn linkup_asset(&self, os: &str, arch: &str) -> Option<Asset> {
            let lookup_os = match os {
                "macos" => "apple-darwin",
                "linux" => "unknown-linux",
                _ => return None,
            };

            let asset = self
                .assets
                .iter()
                .find(|asset| {
                    asset.name.contains(lookup_os)
                        && asset.name.contains(arch)
                        && asset.name.ends_with(".tar.gz")
                })
                .cloned();

            if asset.is_none() {
                log::debug!(
                    "Linkup release for OS '{}' and ARCH '{}' not found on version {}",
                    lookup_os,
                    arch,
                    self.version
                );
            }

            asset
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Asset {
        name: String,
        #[serde(rename = "browser_download_url")]
        download_url: String,
    }

    impl Asset {
        async fn inner_download(&self) -> Result<PathBuf, Error> {
            let response = reqwest::get(&self.download_url).await?;

            let file_path = env::temp_dir().join(&self.name);
            let mut file = fs::File::create(&file_path)?;

            let mut content = std::io::Cursor::new(response.bytes().await?);
            std::io::copy(&mut content, &mut file)?;

            Ok(file_path)
        }

        pub async fn download(&self) -> Result<PathBuf, Error> {
            let filename = "linkup";
            let file_path = self.inner_download().await?;
            let file = fs::File::open(&file_path)?;

            let decoder = GzDecoder::new(file);
            let mut archive = Archive::new(decoder);

            let new_exe_path = archive.entries()?.filter_map(|e| e.ok()).find_map(
                |mut entry| -> Option<PathBuf> {
                    let entry_path = entry.path().unwrap();

                    if entry_path.to_str().unwrap().contains(filename) {
                        let path = env::temp_dir().join(filename);

                        entry.unpack(&path).unwrap();

                        Some(path)
                    } else {
                        None
                    }
                },
            );

            match new_exe_path {
                Some(new_exe_path) => Ok(new_exe_path),
                None => Err(Error::MissingBinary),
            }
        }
    }

    pub(super) async fn fetch_releases() -> Result<Vec<Release>, Error> {
        let url: Url = "https://api.github.com/repos/endformdev/linkup/releases?per_page=100"
            .parse()
            .expect("GitHub URL to be correct");

        Ok(fetch(url).await?.unwrap_or_default())
    }

    async fn fetch<T>(url: Url) -> Result<Option<T>, Error>
    where
        T: DeserializeOwned,
    {
        let mut req = reqwest::Request::new(reqwest::Method::GET, url);
        let headers = req.headers_mut();
        headers.append("User-Agent", HeaderValue::from_static("linkup-cli"));
        headers.append(
            "Accept",
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.append(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let response = client.execute(req).await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#checking-the-status-of-your-rate-limit
            let retry_at = response
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|value| value.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or_else(super::next_morning_utc_seconds);

            return Err(Error::RateLimit(retry_at));
        }

        Ok(Some(response.json::<T>().await?))
    }
}

use std::{cmp::Ordering, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{linkup_file_path, state::State};
use github::Asset;
use linkup::{Version, VersionChannel};
use linkup_clients::WorkerClient;

const CACHE_FILE_NAME: &str = "releases_cache.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct Release {
    pub channel: VersionChannel,
    pub version: Version,
    pub binary: Asset,
}

impl Release {
    fn from_github_release(gh_release: &github::Release, os: &str, arch: &str) -> Option<Release> {
        let version = Version::try_from(gh_release.version.as_str());
        let asset = gh_release.linkup_asset(os, arch);

        match (version, asset) {
            (Ok(version), Some(asset)) => Some(Release {
                channel: version.channel(),
                version,
                binary: asset,
            }),
            _ => None,
        }
    }
}

pub enum Update {
    Available(Release),
    /// There is a newer release, but it's not supported by the deployed worker.
    RequiresWorkerUpdate {
        release: Release,
        worker_version: Version,
    },
}

#[derive(Serialize, Deserialize)]
pub struct CachedReleases {
    fetched_at: u64,
    next_fetch_at: u64,
    releases: Vec<Release>,
    #[serde(default)]
    worker_version: Option<Version>,
}

impl CachedReleases {
    fn empty_with_retry(retry_at: u64) -> Self {
        Self {
            fetched_at: now(),
            next_fetch_at: retry_at,
            releases: Vec::default(),
            worker_version: None,
        }
    }

    fn cache_path() -> PathBuf {
        linkup_file_path(CACHE_FILE_NAME)
    }

    /// Always return the cache only if is "fresh". If the cache is expired, this will delete the
    /// cache file and return None.
    fn load() -> Option<Self> {
        let path = linkup_file_path(CACHE_FILE_NAME);
        if !path.exists() {
            return None;
        }

        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) => {
                log::debug!("failed to open cached latest release file: {}", error);

                return None;
            }
        };

        let cache: Self = match serde_json::from_reader(file) {
            Ok(cache) => cache,
            Err(error) => {
                log::debug!("failed to parse cached latest release: {}", error);

                if std::fs::remove_file(&path).is_err() {
                    log::debug!("failed to delete latest release cache file");
                }

                return None;
            }
        };

        if now() > cache.next_fetch_at {
            Self::clear();

            return None;
        }

        Some(cache)
    }

    fn save(&self) {
        match std::fs::File::create(linkup_file_path(CACHE_FILE_NAME)) {
            Ok(new_file) => {
                if let Err(error) = serde_json::to_writer_pretty(new_file, self) {
                    log::debug!("failed to write the release data into cache: {}", error);
                }
            }
            Err(error) => {
                log::debug!("Failed to create release cache file: {}", error);
            }
        }
    }

    pub fn clear() {
        let path = Self::cache_path();
        if !path.exists() {
            return;
        }

        if let Err(error) = std::fs::remove_file(&path) {
            log::debug!("failed to delete cached latest release file: {}", error);
        }
    }
}

async fn fetch_releases(os: &str, arch: &str) -> Result<Vec<Release>, github::Error> {
    let releases = github::fetch_releases()
        .await?
        .iter()
        .filter_map(|gh_release| Release::from_github_release(gh_release, os, arch))
        .collect();

    Ok(releases)
}

async fn fetch_worker_version() -> Option<Version> {
    let state = State::load().ok()?;
    let worker = WorkerClient::new(&state.linkup.worker_url, &state.linkup.worker_token);

    match worker.version().await {
        Ok(version) => Some(version),
        Err(error) => {
            log::debug!("Failed to fetch the worker version: {}", error);

            None
        }
    }
}

async fn load_or_fetch_releases() -> CachedReleases {
    if let Some(cached_releases) = CachedReleases::load() {
        return cached_releases;
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let cache = match fetch_releases(os, arch).await {
        Ok(releases) => CachedReleases {
            fetched_at: now(),
            next_fetch_at: next_morning_utc_seconds(),
            releases,
            worker_version: fetch_worker_version().await,
        },
        Err(github::Error::RateLimit(retry_at)) => CachedReleases::empty_with_retry(retry_at),
        Err(_) => CachedReleases::empty_with_retry(next_morning_utc_seconds()),
    };

    cache.save();

    cache
}

/// Look for a newer release on the channel. Stable releases are only offered if they have the
/// same major version as the deployed worker, unless `ignore_worker` is set. If the worker version
/// is unknown, the newest release is offered.
pub async fn check_for_update(
    current_version: &Version,
    channel: Option<VersionChannel>,
    ignore_worker: bool,
) -> Option<Update> {
    let channel = channel.unwrap_or_else(|| current_version.channel());
    log::debug!("Looking for available update on '{channel}' channel.");

    let cache = load_or_fetch_releases().await;
    let worker_version = cache.worker_version.as_ref().filter(|_| !ignore_worker);

    select_update(&cache.releases, current_version, &channel, worker_version)
}

fn select_update(
    releases: &[Release],
    current_version: &Version,
    channel: &VersionChannel,
    worker_version: Option<&Version>,
) -> Option<Update> {
    let newer_releases: Vec<&Release> = releases
        .iter()
        .filter(|release| &release.channel == channel)
        .filter(|release| {
            channel != &current_version.channel() || &release.version > current_version
        })
        .collect();

    let newest = |releases: Vec<&Release>| {
        releases
            .into_iter()
            .max_by(|a, b| a.version.partial_cmp(&b.version).unwrap_or(Ordering::Equal))
            .cloned()
    };

    // Beta releases are not versioned against the worker, so they are always offered.
    let worker_version = match worker_version {
        Some(worker_version) if channel == &VersionChannel::Stable => worker_version,
        _ => return newest(newer_releases).map(Update::Available),
    };

    let supported_releases = newer_releases
        .iter()
        .copied()
        .filter(|release| release.version.major == worker_version.major)
        .collect();

    match newest(supported_releases) {
        Some(release) => Some(Update::Available(release)),
        None => newest(newer_releases).map(|release| Update::RequiresWorkerUpdate {
            release,
            worker_version: worker_version.clone(),
        }),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_secs()
}

fn next_morning_utc_seconds() -> u64 {
    let seconds_in_day = 60 * 60 * 24;
    let now_in_seconds = now();

    let seconds_since_midnight = now_in_seconds % seconds_in_day;

    now_in_seconds + (seconds_in_day - seconds_since_midnight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_newest_release_when_worker_version_is_unknown() {
        let update = select(&["4.1.1", "4.2.0", "5.0.0"], "4.1.1", None);

        assert_eq!(available_version(update), Some("5.0.0".to_string()));
    }

    #[test]
    fn offers_newest_release_with_same_major_as_worker() {
        let update = select(
            &["4.1.1", "4.2.0", "4.3.0", "5.0.0"],
            "4.1.1",
            Some("4.2.0"),
        );

        assert_eq!(available_version(update), Some("4.3.0".to_string()));
    }

    #[test]
    fn requires_worker_update_when_only_newer_majors_exist() {
        let update = select(&["4.2.0", "5.0.0", "5.1.0"], "4.2.0", Some("4.2.0"));

        match update {
            Some(Update::RequiresWorkerUpdate {
                release,
                worker_version,
            }) => {
                assert_eq!(release.version.to_string(), "5.1.0");
                assert_eq!(worker_version.to_string(), "4.2.0");
            }
            _ => panic!("expected an update that requires a worker update"),
        }
    }

    #[test]
    fn offers_newer_major_once_worker_is_updated() {
        let update = select(&["4.2.0", "5.0.0"], "4.2.0", Some("5.0.0"));

        assert_eq!(available_version(update), Some("5.0.0".to_string()));
    }

    #[test]
    fn offers_nothing_when_already_on_newest_release() {
        let update = select(&["4.1.1", "4.2.0"], "4.2.0", Some("4.2.0"));

        assert!(update.is_none());
    }

    #[test]
    fn ignores_worker_version_for_beta_releases() {
        let update = select(
            &["0.0.0-next-202610010000-abc", "0.0.0-next-202610020000-def"],
            "0.0.0-next-202610010000-abc",
            Some("4.2.0"),
        );

        assert_eq!(
            available_version(update),
            Some("0.0.0-next-202610020000-def".to_string())
        );
    }

    fn release(version: &str) -> Release {
        let version = Version::try_from(version).unwrap();

        Release {
            channel: version.channel(),
            binary: serde_json::from_value(serde_json::json!({
                "name": format!("linkup-{version}-aarch64-apple-darwin.tar.gz"),
                "browser_download_url": "https://example.com",
            }))
            .unwrap(),
            version,
        }
    }

    fn select(releases: &[&str], current: &str, worker: Option<&str>) -> Option<Update> {
        let releases: Vec<Release> = releases.iter().map(|version| release(version)).collect();
        let current = Version::try_from(current).unwrap();
        let worker = worker.map(|version| Version::try_from(version).unwrap());

        select_update(&releases, &current, &current.channel(), worker.as_ref())
    }

    fn available_version(update: Option<Update>) -> Option<String> {
        match update {
            Some(Update::Available(release)) => Some(release.version.to_string()),
            _ => None,
        }
    }
}
