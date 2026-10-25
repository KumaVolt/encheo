//! Posters and backdrops: downloaded and decoded off the UI thread, at most a few at a time,
//! cached in memory by URL.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Loader {
    http: reqwest::Client,
    limit: Arc<Semaphore>,
}

impl Loader {
    pub fn new() -> Self {
        // Timeouts matter: without them a few stuck requests hold every permit and no other
        // artwork ever loads.
        let http = encheo_plex::with_platform_roots(reqwest::Client::builder())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("http client");
        Self { http, limit: Arc::new(Semaphore::new(8)) }
    }

    /// Fetch and decode. Returns a pixel buffer, which (unlike `Image`) can cross threads.
    pub async fn fetch(&self, url: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
        let _permit = self.limit.acquire().await.ok()?;
        let resp = self.http.get(url).header("Accept", "image/*").send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        tokio::task::spawn_blocking(move || decode(&bytes)).await.ok()?
    }
}

pub fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), w, h))
}

pub fn to_image(buf: SharedPixelBuffer<Rgba8Pixel>) -> Image {
    Image::from_rgba8(buf)
}
