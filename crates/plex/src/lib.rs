//! Plex client for Encheo.
//!
//! Covers what a player needs from Plex: signing in with a PIN, finding the user's servers and the
//! best way to reach each one, browsing libraries, getting a direct-play URL for mpv, artwork URLs,
//! intro/credits markers, and reporting progress back so "Continue Watching" stays in sync with
//! every other Plex app.
//!
//! The flow:
//!
//! ```no_run
//! # async fn demo() -> Result<(), encheo_plex::Error> {
//! use encheo_plex::{Plex, ClientInfo};
//! let plex = Plex::new(ClientInfo::new("my-device-uuid", "My Mac"));
//! let pin = plex.create_pin(false).await?;            // show pin.code, user enters it at plex.tv/link
//! let token = plex.wait_for_pin(&pin).await?;         // polls until the user approves
//! let servers = plex.servers(&token).await?;
//! let server = plex.connect(&servers[0]).await?;      // probes connections, local first
//! for lib in server.libraries().await? {
//!     let items = server.library_items(&lib.key, 0, 50).await?;
//! }
//! # Ok(()) }
//! ```

mod auth;
mod model;
mod server;

pub use auth::Pin;
pub use model::*;
pub use server::Server;

use std::time::Duration;

/// Who we are, sent to Plex on every request. Plex shows these in "Authorized Devices".
#[derive(Clone, Debug)]
pub struct ClientInfo {
    /// Stable per install. Generate once and store it; a new one makes a new device in Plex.
    pub client_id: String,
    pub device_name: String,
    pub product: String,
    pub version: String,
    pub platform: String,
}

impl ClientInfo {
    pub fn new(client_id: impl Into<String>, device_name: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            device_name: device_name.into(),
            product: "Encheo".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Plex returned {status} for {url}")]
    Status { status: u16, url: String },
    #[error("the sign-in code expired before it was approved")]
    PinExpired,
    #[error("none of the server's addresses answered")]
    Unreachable,
    #[error("unexpected response from Plex: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Entry point: talks to plex.tv (account, servers) and hands out [`Server`] connections.
#[derive(Clone)]
pub struct Plex {
    http: reqwest::Client,
    info: ClientInfo,
    plex_tv: String,
    clients_plex_tv: String,
    discover: String,
}

impl Plex {
    pub fn new(info: ClientInfo) -> Self {
        let http = with_platform_roots(reqwest::Client::builder())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .build()
            .expect("reqwest client");
        Self {
            http,
            info,
            plex_tv: "https://plex.tv".into(),
            clients_plex_tv: "https://clients.plex.tv".into(),
            discover: "https://discover.provider.plex.tv".into(),
        }
    }

    /// Point the plex.tv calls somewhere else (tests).
    pub fn with_base_urls(mut self, plex_tv: impl Into<String>, clients_plex_tv: impl Into<String>) -> Self {
        self.plex_tv = plex_tv.into();
        let clients = clients_plex_tv.into();
        self.discover = clients.clone();
        self.clients_plex_tv = clients;
        self
    }

    pub fn info(&self) -> &ClientInfo {
        &self.info
    }

    /// Headers Plex expects on every call.
    pub(crate) fn request(&self, method: reqwest::Method, url: &str, token: Option<&str>) -> reqwest::RequestBuilder {
        let mut r = self
            .http
            .request(method, url)
            .header("Accept", "application/json")
            .header("X-Plex-Product", &self.info.product)
            .header("X-Plex-Version", &self.info.version)
            .header("X-Plex-Client-Identifier", &self.info.client_id)
            .header("X-Plex-Platform", &self.info.platform)
            .header("X-Plex-Device-Name", &self.info.device_name);
        if let Some(t) = token {
            r = r.header("X-Plex-Token", t);
        }
        r
    }

    pub(crate) async fn json<T: serde::de::DeserializeOwned>(r: reqwest::RequestBuilder) -> Result<T> {
        let resp = r.send().await?;
        let status = resp.status();
        let url = resp.url().to_string();
        if !status.is_success() {
            return Err(Error::Status { status: status.as_u16(), url: strip_token(&url) });
        }
        let body = resp.bytes().await?;
        serde_json::from_slice(&body).map_err(|e| Error::Decode(e.to_string()))
    }

    /// The signed-in account.
    pub async fn account(&self, token: &str) -> Result<Account> {
        let url = format!("{}/api/v2/user", self.plex_tv);
        Self::json(self.request(reqwest::Method::GET, &url, Some(token))).await
    }

    /// The account's watchlist (kept by plex.tv, not by a server). Items carry their `guid`;
    /// find them on a server with [`Server::find_by_guid`].
    pub async fn watchlist(&self, token: &str) -> Result<Vec<Item>> {
        let url = format!("{}/library/sections/watchlist/all?includeAdvanced=1", self.discover);
        let r = self
            .request(reqwest::Method::GET, &url, Some(token))
            .header("X-Plex-Container-Start", "0")
            .header("X-Plex-Container-Size", "100");
        let e: model::Envelope<model::MetadataPage> = Self::json(r).await?;
        Ok(e.container.metadata)
    }

    /// Every Plex Media Server this account can use, owned or shared.
    pub async fn servers(&self, token: &str) -> Result<Vec<Resource>> {
        let url = format!("{}/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1", self.clients_plex_tv);
        let all: Vec<Resource> = Self::json(self.request(reqwest::Method::GET, &url, Some(token))).await?;
        Ok(all.into_iter().filter(Resource::is_server).collect())
    }

    /// Find a working address for `resource` and return a client for it.
    ///
    /// Tries [`Resource::candidate_urls`]: local before remote, relay last, and stops at the first
    /// one whose `/identity` answers.
    pub async fn connect(&self, resource: &Resource) -> Result<Server> {
        let token = resource.access_token.clone().unwrap_or_default();
        for base in resource.candidate_urls() {
            let url = format!("{base}/identity");
            let probe = self.request(reqwest::Method::GET, &url, Some(&token)).timeout(Duration::from_millis(2500));
            if matches!(probe.send().await, Ok(r) if r.status().is_success()) {
                return Ok(Server::new(self.clone(), resource.name.clone(), base, token));
            }
        }
        Err(Error::Unreachable)
    }

    /// A client for a server at a known address (from a saved session).
    pub fn server_at(&self, name: impl Into<String>, base_url: impl Into<String>, token: impl Into<String>) -> Server {
        Server::new(self.clone(), name.into(), base_url.into(), token.into())
    }
}

/// TLS roots for HTTP clients. Desktop systems verify with their own certificate store; on
/// Android that needs Java glue a native app doesn't have, so there we bundle Mozilla's roots.
pub fn with_platform_roots(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| reqwest::Certificate::from_der(c).ok()),
    );
    builder
}

/// [`with_platform_roots`] for blocking clients.
pub fn with_platform_roots_blocking(builder: reqwest::blocking::ClientBuilder) -> reqwest::blocking::ClientBuilder {
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| reqwest::Certificate::from_der(c).ok()),
    );
    builder
}

/// Keep tokens out of error messages and logs.
pub(crate) fn strip_token(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let pairs: Vec<(String, String)> =
                u.query_pairs().filter(|(k, _)| k != "X-Plex-Token").map(|(k, v)| (k.into(), v.into())).collect();
            if pairs.is_empty() {
                u.set_query(None);
            } else {
                u.query_pairs_mut().clear().extend_pairs(pairs);
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod tests;
