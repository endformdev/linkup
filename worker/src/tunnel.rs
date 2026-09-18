use std::fmt::Display;

use linkup::TunnelData;

use crate::worker_state::WorkerState;

#[derive(Debug)]
pub enum CreateTunnelError {
    CreateCloudflareTunnel(String),
    CreateDNS(String),
    FetchZone(String),
}

#[derive(Debug)]
pub enum DeleteTunnelError {
    DeleteCloudflareTunnel(String),
    GetDNSRecord(String),
    DeleteDNSRecord(String),
}

impl Display for CreateTunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateTunnelError::CreateCloudflareTunnel(text) => {
                write!(f, "Failed to crate tunnel: {}", text)
            }
            CreateTunnelError::CreateDNS(text) => write!(f, "Failed to crate DNS record: {}", text),
            CreateTunnelError::FetchZone(text) => {
                write!(f, "Failed to fetch Zone details: {}", text)
            }
        }
    }
}

impl Display for DeleteTunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteTunnelError::DeleteCloudflareTunnel(text) => {
                write!(f, "Failed to delete tunnel: {}", text)
            }
            DeleteTunnelError::GetDNSRecord(text) => {
                write!(f, "Failed to fetch DNS record: {}", text)
            }
            DeleteTunnelError::DeleteDNSRecord(text) => {
                write!(f, "Failed to delete DNS record: {}", text)
            }
        }
    }
}

impl std::error::Error for CreateTunnelError {}
impl std::error::Error for DeleteTunnelError {}

pub async fn delete_tunnel(
    api_token: &str,
    account_id: &str,
    zone_id: &str,
    tunnel_id: &str,
) -> Result<(), DeleteTunnelError> {
    let client = crate::cloudflare_client(api_token);

    delete_tunnel_with_client(&client, account_id, zone_id, tunnel_id).await
}

async fn delete_tunnel_with_client(
    client: &cloudflare::framework::async_api::Client,
    account_id: &str,
    zone_id: &str,
    tunnel_id: &str,
) -> Result<(), DeleteTunnelError> {
    let get_dns_record_req = cloudflare::endpoints::dns::ListDnsRecords {
        zone_identifier: zone_id,
        params: cloudflare::endpoints::dns::ListDnsRecordsParams {
            record_type: Some(cloudflare::endpoints::dns::DnsContent::CNAME {
                content: format!("{}.cfargotunnel.com", tunnel_id),
            }),
            ..Default::default()
        },
    };

    let records = client
        .request(&get_dns_record_req)
        .await
        .map_err(|error| DeleteTunnelError::GetDNSRecord(error.to_string()))?
        .result;

    let record = match records.len() {
        0 => None,
        1 => Some(&records[0]),
        2.. => {
            return Err(DeleteTunnelError::GetDNSRecord(
                "Fetching DNS for tunnel returned more than one record".to_string(),
            ));
        }
    };

    if let Some(record) = record {
        let delete_dns_record_req = cloudflare::endpoints::dns::DeleteDnsRecord {
            zone_identifier: zone_id,
            identifier: &record.id,
        };

        client
            .request(&delete_dns_record_req)
            .await
            .map_err(|error| DeleteTunnelError::DeleteDNSRecord(error.to_string()))?;
    }

    let delete_tunnel_req = cloudflare::endpoints::cfd_tunnel::delete_tunnel::DeleteTunnel {
        account_identifier: account_id,
        tunnel_id,
        params: cloudflare::endpoints::cfd_tunnel::delete_tunnel::Params { cascade: true },
    };

    client
        .request(&delete_tunnel_req)
        .await
        .map_err(|error| DeleteTunnelError::DeleteCloudflareTunnel(error.to_string()))?;

    Ok(())
}

pub async fn upsert_tunnel(state: &WorkerState, session_name: &str) -> Result<TunnelData, String> {
    let kv = &state.tunnels_kv;

    let tunnel_name = format!("{}{}", state.tunnel_prefix, session_name);
    let tunnel_data: Option<TunnelData> = kv
        .get(&tunnel_name)
        .json()
        .await
        .map_err(|e| e.to_string())?;

    match tunnel_data {
        Some(mut tunnel_data) => {
            tunnel_data.last_started = worker::Date::now().as_millis();
            kv.put(&tunnel_name, &tunnel_data)
                .map_err(|e| e.to_string())?
                .execute()
                .await
                .map_err(|e| e.to_string())?;

            Ok(tunnel_data)
        }
        None => {
            let tunnel_data = create_tunnel(
                &state.cloudflare.api_token,
                &state.cloudflare.account_id,
                &state.cloudflare.tunnel_zone_id,
                &tunnel_name,
            )
            .await
            .map_err(|e| e.to_string())?;

            kv.put(&tunnel_name, &tunnel_data)
                .map_err(|e| e.to_string())?
                .execute()
                .await
                .map_err(|e| e.to_string())?;

            Ok(tunnel_data)
        }
    }
}

async fn create_tunnel(
    api_token: &str,
    account_id: &str,
    zone_id: &str,
    tunnel_name: &str,
) -> Result<TunnelData, CreateTunnelError> {
    let client = crate::cloudflare_client(api_token);
    let tunnel_secret = crate::generate_secret();

    let create_tunnel_req = cloudflare::endpoints::cfd_tunnel::create_tunnel::CreateTunnel {
        account_identifier: account_id,
        params: cloudflare::endpoints::cfd_tunnel::create_tunnel::Params {
            name: tunnel_name,
            tunnel_secret: &tunnel_secret,
            config_src: &cloudflare::endpoints::cfd_tunnel::ConfigurationSrc::Local,
            metadata: None,
        },
    };

    let tunnel = client
        .request(&create_tunnel_req)
        .await
        .map_err(|err| CreateTunnelError::CreateCloudflareTunnel(err.to_string()))?
        .result;

    let create_dns_req = cloudflare::endpoints::dns::CreateDnsRecord {
        zone_identifier: zone_id,
        params: cloudflare::endpoints::dns::CreateDnsRecordParams {
            proxied: Some(true),
            name: tunnel_name,
            content: cloudflare::endpoints::dns::DnsContent::CNAME {
                content: format!("{}.cfargotunnel.com", tunnel.id),
            },
            ttl: None,
            priority: None,
        },
    };

    client
        .request(&create_dns_req)
        .await
        .map_err(|err| CreateTunnelError::CreateDNS(err.to_string()))?;

    let get_zone_req = cloudflare::endpoints::zone::ZoneDetails {
        identifier: zone_id,
    };

    let zone = client
        .request(&get_zone_req)
        .await
        .map_err(|err| CreateTunnelError::FetchZone(err.to_string()))?
        .result;

    let tunnel_data = TunnelData {
        account_id: account_id.to_string(),
        name: tunnel_name.to_string(),
        url: format!("https://{}.{}", tunnel_name, zone.name),
        id: tunnel.id.to_string(),
        secret: tunnel_secret,
        last_started: worker::Date::now().as_millis(),
    };

    Ok(tunnel_data)
}

#[cfg(test)]
mod tests {
    use cloudflare::framework::{
        Environment, HttpApiClientConfig, async_api::Client, auth::Credentials,
    };
    use mockito::Matcher;

    use super::delete_tunnel_with_client;

    fn test_client(server_url: &str) -> Client {
        Client::new(
            Credentials::UserAuthToken {
                token: "test-token".to_string(),
            },
            HttpApiClientConfig::default(),
            Environment::Custom(server_url.parse().unwrap()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn leaves_tunnel_intact_when_dns_lookup_fails() {
        let mut server = mockito::Server::new_async().await;
        let dns_lookup = server
            .mock("GET", "/zones/zone-id/dns_records")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("type".into(), "CNAME".into()),
                Matcher::UrlEncoded(
                    "content".into(),
                    "tunnel-id.cfargotunnel.com".into(),
                ),
            ]))
            .with_status(500)
            .with_body(
                r#"{"success":false,"errors":[{"code":1000,"message":"failure"}],"messages":[],"result":null}"#,
            )
            .create_async()
            .await;
        let tunnel_delete = server
            .mock("DELETE", "/accounts/account-id/cfd_tunnel/tunnel-id")
            .expect(0)
            .create_async()
            .await;

        let result = delete_tunnel_with_client(
            &test_client(&server.url()),
            "account-id",
            "zone-id",
            "tunnel-id",
        )
        .await;

        assert!(result.is_err());
        dns_lookup.assert_async().await;
        tunnel_delete.assert_async().await;
    }

    #[tokio::test]
    async fn deletes_tunnel_when_dns_record_is_already_missing() {
        let mut server = mockito::Server::new_async().await;
        let dns_lookup = server
            .mock("GET", "/zones/zone-id/dns_records")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("type".into(), "CNAME".into()),
                Matcher::UrlEncoded("content".into(), "tunnel-id.cfargotunnel.com".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"success":true,"errors":[],"messages":[],"result":[]}"#)
            .create_async()
            .await;
        let tunnel_delete = server
            .mock("DELETE", "/accounts/account-id/cfd_tunnel/tunnel-id")
            .match_query(Matcher::UrlEncoded("cascade".into(), "true".into()))
            .with_status(200)
            .with_body(
                r#"{"success":true,"errors":[],"messages":[],"result":{"id":"00000000-0000-0000-0000-000000000000","created_at":"2026-01-01T00:00:00Z","deleted_at":"2026-01-01T00:00:00Z","name":"test","connections":[],"metadata":{}}}"#,
            )
            .create_async()
            .await;

        let result = delete_tunnel_with_client(
            &test_client(&server.url()),
            "account-id",
            "zone-id",
            "tunnel-id",
        )
        .await;

        assert!(result.is_ok());
        dns_lookup.assert_async().await;
        tunnel_delete.assert_async().await;
    }
}
