//! iOS, for now: no playback. Slint draws with Metal on iOS, so mpv can't draw into its frame
//! the way it does elsewhere (OpenGL); video there needs mpv in its own view under the UI, with an
//! iOS build of libmpv. Until then this stands in for the player with the same interface.

#![allow(dead_code, unused_variables)]

type ProcLoader<'a> = &'a dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void;

/// File sources mpv can read through `encheo://<source>/<path>`, by source number.
pub static STREAM_SOURCES: std::sync::Mutex<Vec<Option<std::sync::Arc<encheo_vfs::Source>>>> = std::sync::Mutex::new(Vec::new());
/// The URL mpv plays for a file in a source.
pub fn stream_url(source: usize, path: &str) -> String {
    let enc: String = path.split('/').map(|seg| url_encode(seg)).collect::<Vec<_>>().join("/");
    format!("encheo://{source}/{enc}")
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
/// An audio or subtitle track of the current file, as mpv numbers it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub label: String,
    pub selected: bool,
    /// Stream index inside the file (matches Plex's `Stream.index`); `None` for added files.
    pub ff_index: Option<i64>,
    /// Title mpv has for it; for subtitles we add ourselves this is our own tag.
    pub title: Option<String>,
    /// Language tag in the file, if any.
    pub lang: Option<String>,
    /// Short codec name, e.g. "SRT", "ASS", "EAC3".
    pub codec: Option<String>,
    pub forced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start: f64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub buffering: bool,
    pub ended: bool,
}

/// `1:02:03` / `42:07`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub struct Player;

impl Player {
    pub fn new() -> Result<Self, String> {
        Err("playback isn't available on iOS yet".into())
    }
    pub fn setup_gl(&self, loader: ProcLoader<'_>, on_frame: impl Fn() + Send + 'static) {}
    pub fn draw(&self, width: u32, height: u32) {}
    pub fn teardown(&self) {}
    pub fn load(&self, url: &str, start_secs: Option<f64>) {}
    pub fn load_with_headers(&self, url: &str, start_secs: Option<f64>, headers: &[String]) {}
    pub fn append(&self, url: &str) {}
    pub fn set_playlist_pos(&self, index: usize) {}
    pub fn playlist_pos(&self) -> Option<usize> { None }
    pub fn next(&self) {}
    pub fn prev(&self) {}
    pub fn stop(&self) {}
    pub fn toggle_pause(&self) {}
    pub fn seek_to(&self, secs: f64) {}
    pub fn seek_by(&self, secs: f64) {}
    pub fn status(&self) -> Status { Status::default() }
    pub fn tracks(&self, kind: &str) -> Vec<Track> { vec![] }
    pub fn select_track(&self, kind: &str, id: Option<i64>) {}
    pub fn add_subtitle(&self, url: &str, title: &str) {}
    pub fn add_subtitle_selected(&self, url: &str, title: &str) {}
    pub fn chapters(&self) -> Vec<Chapter> { vec![] }
    pub fn current_chapter(&self) -> Option<usize> { None }
    pub fn step_chapter(&self, delta: i64) {}
    pub fn volume(&self) -> f64 { 100.0 }
    pub fn set_volume(&self, v: f64) {}
    pub fn muted(&self) -> bool { false }
    pub fn toggle_mute(&self) {}
    pub fn speed(&self) -> f64 { 1.0 }
    pub fn set_speed(&self, speed: f64) {}
    pub fn set_sub_scale(&self, scale: f64) {}
    pub fn sub_delay(&self) -> f64 { 0.0 }
    pub fn set_sub_delay(&self, secs: f64) {}
    pub fn audio_delay(&self) -> f64 { 0.0 }
    pub fn set_audio_delay(&self, secs: f64) {}
    pub fn seek_fast(&self, secs: f64) {}
}
