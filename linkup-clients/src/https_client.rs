use hyper_rustls::HttpsConnector;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};

pub type HttpsClient<B> = Client<HttpsConnector<HttpConnector>, B>;

fn tls_client_config() -> rustls::ClientConfig {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut roots = rustls::RootCertStore::empty();
    for cert in
        rustls_native_certs::load_native_certs().expect("should be able to load platform certs")
    {
        roots.add(cert).unwrap();
    }

    rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

pub fn https_client<B>() -> HttpsClient<B>
where
    B: http_body::Body + Send,
    B::Data: Send,
{
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_client_config())
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    Client::builder(TokioExecutor::new()).build(https)
}

pub fn https_client_http1<B>() -> HttpsClient<B>
where
    B: http_body::Body + Send,
    B::Data: Send,
{
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_client_config())
        .https_or_http()
        .enable_http1()
        .build();

    Client::builder(TokioExecutor::new()).build(https)
}
