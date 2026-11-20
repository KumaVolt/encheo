//! Seerr client for Encheo: ask for films and shows you don't have, and follow them until they're
//! in your library.
//!
//! [Seerr](https://github.com/seerr-team/seerr) (the merged Overseerr and Jellyseerr; both older
//! apps speak the same API) sits in front of Radarr and Sonarr. Encheo signs in to it with the
//! user's Plex token or Jellyfin name and password, so requests carry the user's own name and
//! limits, and an admin approves them as usual.
//!
//! ```no_run
//! # async fn demo(plex_token: &str) -> Result<(), encheo_seerr::Error> {
//! use encheo_seerr::{Kind, Seerr, Seasons, State};
//! let seerr = Seerr::connect("requests.example.com").await?;  // tries https, then http
//! let me = seerr.sign_in_plex(plex_token).await?;
//! let session = seerr.session();                              // keep it; Seerr::resume(address, session)
//! let dune = seerr.title(Kind::Movie, 693134).await?;
//! if dune.state(false) == State::NotRequested && me.can_request(Kind::Movie, false) {
//!     seerr.request(Kind::Movie, 693134, Seasons::All, false).await?;
//! }
//! # Ok(()) }
//! ```

mod model;
#[cfg(test)]
mod tests;

pub use model::*;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("that isn't a web address")]
    Address,
    #[error("no Seerr, Overseerr or Jellyseerr answered at that address")]
    NotSeerr,
    #[error("signed out of Seerr")]
    SignedOut,
    #[error("Seerr refused: {message}")]
    Refused { status: u16, message: String },
    #[error("unexpected response from Seerr: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Which seasons of a show to ask for. Films ignore it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seasons {
    All,
    Only(Vec<u32>),
}

/// A connection to one Seerr. Cheap to clone; clones share the sign-in.
#[derive(Clone)]
pub struct Seerr {
    http: reqwest::Client,
    /// Scheme, host, port and any path prefix, without `/api/v1` and without a trailing slash.
    address: String,
    cookies: Arc<Mutex<BTreeMap<String, String>>>,
}

fn http_client() -> reqwest::Client {
    encheo_plex::with_platform_roots(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .user_agent(concat!("Encheo/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// The addresses to try for what the user typed: as given when it has a scheme, else https then
/// http. Seerr's own `/api/v1` and trailing slashes are dropped; a path prefix (a reverse proxy's
/// `/seerr`) is kept.
pub fn candidates(typed: &str) -> Vec<String> {
    let t = typed.trim().trim_end_matches('/');
    let t = t.strip_suffix("/api/v1").unwrap_or(t).trim_end_matches('/');
    if t.is_empty() || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return vec![];
    }
    let with = |scheme: &str| format!("{scheme}://{t}");
    let list = if t.contains("://") { vec![t.to_string()] } else { vec![with("https"), with("http")] };
    list.into_iter()
        .filter(|a| url::Url::parse(a).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()))
        .collect()
}

impl Seerr {
    fn at(address: String) -> Self {
        Self { http: http_client(), address, cookies: Default::default() }
    }

    /// Find the Seerr at what the user typed ("requests.example.com", "192.168.1.5:5055", a full
    /// address) and check it is one.
    pub async fn connect(typed: &str) -> Result<Self> {
        let list = candidates(typed);
        if list.is_empty() {
            return Err(Error::Address);
        }
        let mut last = Error::NotSeerr;
        for address in list {
            let s = Self::at(address);
            match s.public_settings().await {
                Ok(_) => return Ok(s),
                Err(e @ Error::Http(_)) => last = e,
                Err(_) => last = Error::NotSeerr,
            }
        }
        Err(last)
    }

    /// Pick up a sign-in kept from [`Seerr::session`]. Nothing is checked until the first call.
    pub fn resume(address: &str, session: &str) -> Self {
        let s = Self::at(address.trim_end_matches('/').to_string());
        s.keep_cookies(session.split(';').map(str::trim));
        s
    }

    /// The address this connection uses, to store with [`Seerr::session`].
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The sign-in, to store and hand to [`Seerr::resume`]. Holds the session cookie: keep it as
    /// secret as a password. Seerr lets it last 30 days.
    pub fn session(&self) -> String {
        let jar = self.cookies.lock().unwrap();
        jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }

    fn keep_cookies<'a>(&self, pairs: impl Iterator<Item = &'a str>) {
        let mut jar = self.cookies.lock().unwrap();
        for pair in pairs {
            let Some((k, v)) = pair.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k.is_empty() || k.contains([' ', ';', ',']) || v.contains([';', ',', '\r', '\n']) {
                continue;
            }
            if v.is_empty() {
                jar.remove(k);
            } else {
                jar.insert(k.to_string(), v.to_string());
            }
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<serde_json::Value>) -> Result<reqwest::Response> {
        let mut r = self.http.request(method, format!("{}/api/v1{path}", self.address)).header("Accept", "application/json");
        {
            let jar = self.cookies.lock().unwrap();
            if !jar.is_empty() {
                let cookie: Vec<String> = jar.iter().map(|(k, v)| format!("{k}={v}")).collect();
                r = r.header("Cookie", cookie.join("; "));
            }
            // With CSRF protection on, Seerr wants its token echoed back.
            if let Some(t) = jar.get("XSRF-TOKEN") {
                r = r.header("X-XSRF-TOKEN", t);
            }
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await?;
        // Only the first part of a Set-Cookie is the cookie; the rest is its attributes.
        let set: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.split(';').next())
            .map(str::to_string)
            .collect();
        self.keep_cookies(set.iter().map(String::as_str));
        Ok(resp)
    }

    async fn call<T: serde::de::DeserializeOwned>(&self, method: Method, path: &str, body: Option<serde_json::Value>) -> Result<T> {
        let resp = self.send(method, path, body).await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(refusal(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }

    /// Seerr's public settings; also proves the address is a Seerr that has been set up.
    pub async fn public_settings(&self) -> Result<PublicSettings> {
        let v: serde_json::Value = self.call(Method::GET, "/settings/public", None).await?;
        if v.get("initialized").is_none() {
            return Err(Error::NotSeerr);
        }
        serde_json::from_value(v).map_err(|e| Error::Decode(e.to_string()))
    }

    /// Sign in with a Plex account token. The account must be one Seerr knows (it imports the
    /// server's users) or one it lets in on first sign-in.
    pub async fn sign_in_plex(&self, plex_token: &str) -> Result<User> {
        self.sign_in("/auth/plex", serde_json::json!({ "authToken": plex_token })).await
    }

    /// Sign in with a Jellyfin or Emby user's name and password (Seerr checks them with the server).
    pub async fn sign_in_jellyfin(&self, username: &str, password: &str) -> Result<User> {
        self.sign_in("/auth/jellyfin", serde_json::json!({ "username": username, "password": password })).await
    }

    /// Sign in with a Seerr account of its own (email and password).
    pub async fn sign_in_local(&self, email: &str, password: &str) -> Result<User> {
        self.sign_in("/auth/local", serde_json::json!({ "email": email, "password": password })).await
    }

    /// A refused sign-in is a refusal (wrong password, not allowed in), not a lost session.
    async fn sign_in(&self, path: &str, body: serde_json::Value) -> Result<User> {
        self.cookies.lock().unwrap().remove("connect.sid");
        match self.call(Method::POST, path, Some(body)).await {
            Err(Error::SignedOut) => Err(Error::Refused { status: 403, message: "Access denied.".into() }),
            r => r,
        }
    }

    /// Who is signed in. [`Error::SignedOut`] when the session ran out.
    pub async fn me(&self) -> Result<User> {
        self.call(Method::GET, "/auth/me", None).await
    }

    /// End the session on Seerr and forget it here.
    pub async fn sign_out(&self) -> Result<()> {
        let r = self.send(Method::POST, "/auth/logout", Some(serde_json::json!({}))).await;
        self.cookies.lock().unwrap().clear();
        r.map(|_| ())
    }

    /// Films and shows matching `query` (people are left out), each with its state.
    pub async fn search(&self, query: &str, page: u32) -> Result<Vec<Title>> {
        // Seerr's validator rejects some characters left as they are; encode everything.
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>().replace('+', "%20");
        let p: SearchPage = self.call(Method::GET, &format!("/search?query={q}&page={}", page.max(1)), None).await?;
        Ok(p.results.into_iter().filter_map(|r| r.kind().map(|k| r.into_title(k))).collect())
    }

    /// A title by its TMDB id, with seasons, status, requests and download progress.
    pub async fn title(&self, kind: Kind, tmdb_id: u64) -> Result<Title> {
        let raw: RawTitle = self.call(Method::GET, &format!("/{}/{tmdb_id}", kind.as_str()), None).await?;
        Ok(raw.into_title(kind))
    }

    /// Ask for a title. Seerr applies the user's quota and approval rules; the answer says
    /// whether the request waits for approval.
    pub async fn request(&self, kind: Kind, tmdb_id: u64, seasons: Seasons, is_4k: bool) -> Result<MediaRequest> {
        let mut body = serde_json::json!({ "mediaType": kind.as_str(), "mediaId": tmdb_id, "is4k": is_4k });
        if kind == Kind::Tv {
            body["seasons"] = match seasons {
                Seasons::All => serde_json::json!("all"),
                Seasons::Only(list) => serde_json::json!(list),
            };
        }
        let resp = self.send(Method::POST, "/request", Some(body)).await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        // 202 is Seerr's "no seasons left to request": nothing was made.
        if !status.is_success() || status == StatusCode::ACCEPTED {
            return Err(refusal(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }
}

/// Seerr's error bodies carry `message` (or `error`); a 401, or a 403 on a signed-in call, means
/// the session is gone.
fn refusal(status: StatusCode, body: &[u8]) -> Error {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Body {
        message: Option<String>,
        error: Option<String>,
    }
    let b: Body = serde_json::from_slice(body).unwrap_or_default();
    let message = b.message.or(b.error).unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_string());
    let signed_out = status == StatusCode::UNAUTHORIZED
        || (status == StatusCode::FORBIDDEN && message.to_ascii_lowercase().contains("permission to access this endpoint"));
    if signed_out { Error::SignedOut } else { Error::Refused { status: status.as_u16(), message } }
}
