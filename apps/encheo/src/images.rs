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

/// Nothing on screen needs more: a full-screen backdrop on a 4K TV is drawn from 1920 x 1080.
const MAX_SIDE: u32 = 1920;

pub fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let mut img = image::load_from_memory(bytes).ok()?;
    // A picture that came at full size anyway (servers that don't resize): shrink it, so one
    // photo can't take 16 MB or more of memory, plus as much again on the graphics card.
    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.thumbnail(MAX_SIDE, MAX_SIDE);
    }
    let img = img.into_rgba8();
    let (w, h) = img.dimensions();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), w, h))
}

/// Decoded pictures by URL, so a poster seen again shows at once. Holds up to [`Cache::BUDGET`]
/// of pixels and forgets the least recently used beyond that (a library of a thousand posters
/// would otherwise keep ~600 MB, plus the GPU's copies). A forgotten picture still on screen
/// stays there; it's fetched again only if it's asked for again.
#[derive(Default)]
pub struct Cache {
    map: std::collections::HashMap<String, (Image, u64)>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    pub const BUDGET: usize = 160 << 20;

    pub fn get(&mut self, url: &str) -> Option<Image> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(url).map(|(img, used)| {
            *used = clock;
            img.clone()
        })
    }

    pub fn insert(&mut self, url: String, img: Image) {
        self.clock += 1;
        self.bytes += Self::size(&img);
        if let Some((old, _)) = self.map.insert(url, (img, self.clock)) {
            self.bytes -= Self::size(&old);
        }
        if self.bytes > Self::BUDGET {
            // Forget the oldest until a quarter of the budget is free, so this doesn't run on
            // every new picture.
            let mut by_age: Vec<(u64, String)> = self.map.iter().map(|(u, (_, t))| (*t, u.clone())).collect();
            by_age.sort_unstable();
            for (_, url) in by_age {
                if self.bytes <= Self::BUDGET * 3 / 4 {
                    break;
                }
                if let Some((img, _)) = self.map.remove(&url) {
                    self.bytes -= Self::size(&img);
                }
            }
        }
    }

    fn size(img: &Image) -> usize {
        let s = img.size();
        s.width as usize * s.height as usize * 4
    }
}

pub fn to_image(buf: SharedPixelBuffer<Rgba8Pixel>) -> Image {
    Image::from_rgba8(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(side: u32) -> Image {
        Image::from_rgba8(SharedPixelBuffer::new(side, side))
    }

    #[test]
    fn huge_pictures_are_shrunk() {
        let mut png = Vec::new();
        image::RgbaImage::new(4000, 2000).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let buf = decode(&png).unwrap();
        assert_eq!((buf.width(), buf.height()), (1920, 960));
    }

    #[test]
    fn cache_forgets_least_recently_used() {
        let mut cache = Cache::default();
        // 2048 x 2048 x 4 = 16 MiB each; the budget holds ten.
        for i in 0..10 {
            cache.insert(format!("u{i}"), picture(2048));
        }
        assert!(cache.get("u0").is_some()); // u0 is now the most recent
        cache.insert("u10".into(), picture(2048));
        assert!(cache.bytes <= Cache::BUDGET);
        assert!(cache.get("u0").is_some(), "recently used stays");
        assert!(cache.get("u1").is_none(), "oldest goes");
        assert!(cache.get("u10").is_some());
    }
}
