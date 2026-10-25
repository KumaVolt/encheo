//! PIN sign-in: the app asks plex.tv for a code, the user approves it in a browser, the app polls
//! until a token appears. Works on a TV remote too, because the user types the code on their phone.

use crate::{Error, Plex, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub id: u64,
    /// Short (4 characters) for plex.tv/link, or long when `strong` is set.
    pub code: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Seconds until the code stops working, as reported when it was made.
    #[serde(default)]
    pub expires_in: Option<u64>,
}

impl Plex {
    /// Ask plex.tv for a sign-in code.
    ///
    /// `strong = false` gives a 4-character code for <https://plex.tv/link>, good for TVs.
    /// `strong = true` gives a long code meant for [`Plex::auth_url`], good on desktop where we can
    /// open a browser.
    pub async fn create_pin(&self, strong: bool) -> Result<Pin> {
        let url = format!("{}/api/v2/pins?strong={strong}", self.plex_tv);
        Self::json(self.request(reqwest::Method::POST, &url, None)).await
    }

    /// The page that lets the user approve a strong PIN in their browser.
    pub fn auth_url(&self, pin: &Pin) -> String {
        let mut u = url::Url::parse("https://app.plex.tv/auth").expect("static url");
        let fragment = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("clientID", &self.info.client_id)
            .append_pair("code", &pin.code)
            .append_pair("context[device][product]", &self.info.product)
            .finish();
        u.set_fragment(Some(&format!("?{fragment}")));
        u.to_string()
    }

    /// Check a PIN once. `Some(token)` when the user has approved it.
    pub async fn check_pin(&self, pin: &Pin) -> Result<Option<String>> {
        let url = format!("{}/api/v2/pins/{}", self.plex_tv, pin.id);
        let p: Pin = match Self::json(self.request(reqwest::Method::GET, &url, None)).await {
            Err(Error::Status { status: 404, .. }) => return Err(Error::PinExpired),
            r => r?,
        };
        Ok(p.auth_token.filter(|t| !t.is_empty()))
    }

    /// Poll every two seconds until the user approves the PIN or it expires.
    pub async fn wait_for_pin(&self, pin: &Pin) -> Result<String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(pin.expires_in.unwrap_or(900));
        loop {
            if let Some(token) = self.check_pin(pin).await? {
                return Ok(token);
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::PinExpired);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}
