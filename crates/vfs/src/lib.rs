//! File sources: places with folders of video and music files, as opposed to media servers.
//!
//! Every source can list a folder and open a file for reading (`Read + Seek`, which the player
//! streams through mpv's custom-protocol hook). Sources mpv can reach on its own (local paths,
//! WebDAV over HTTP) also give a direct URL, which is faster.
//!
//! The API is blocking; call it from a worker thread.

mod local;
pub mod names;
mod smb;
mod webdav;

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("couldn't connect: {0}")]
    Connect(String),
    #[error("sign-in failed")]
    Auth,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Local,
    WebDav,
    Smb,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Local => "Folder",
            Kind::WebDav => "WebDAV",
            Kind::Smb => "SMB",
        }
    }
}

/// How to reach a source, as the user entered it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub kind: Kind,
    /// Shown in the sidebar.
    pub name: String,
    /// Local: a folder path. WebDAV: `https://host/path`. SMB: `smb://host[:port]/share[/path]`.
    pub location: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// A folder or file in a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Path inside the source, `/`-separated, no leading slash. Pass back to `list`/`open`.
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

impl Entry {
    pub fn media_kind(&self) -> Option<MediaKind> {
        if self.is_dir {
            None
        } else {
            media_kind(&self.name)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

/// What the player recognises, by extension.
pub fn media_kind(name: &str) -> Option<MediaKind> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    const VIDEO: &[&str] = &[
        "mkv", "mp4", "m4v", "mov", "avi", "wmv", "asf", "flv", "webm", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "ogm",
        "ogv", "rmvb", "3gp", "divx", "iso",
    ];
    const AUDIO: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "alac", "wma", "aiff", "ape", "dsf"];
    if VIDEO.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else if AUDIO.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else {
        None
    }
}

/// A URL mpv can open itself, with request headers (credentials) if it needs any.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectUrl {
    pub url: String,
    pub headers: Vec<String>,
}

pub trait ReadSeek: Read + Seek + Send {
    /// Total size in bytes, when known.
    fn len(&self) -> Option<u64>;
}

/// A connected source.
pub enum Source {
    Local(local::Local),
    WebDav(webdav::WebDav),
    Smb(smb::Smb),
}

impl Source {
    /// Connect and check the credentials (lists the root).
    pub fn connect(config: &Config) -> Result<Source> {
        let source = match config.kind {
            Kind::Local => Source::Local(local::Local::new(&config.location)?),
            Kind::WebDav => Source::WebDav(webdav::WebDav::new(config)?),
            Kind::Smb => Source::Smb(smb::Smb::connect(config)?),
        };
        source.list("")?;
        Ok(source)
    }

    /// Folders first, then files the player understands; both sorted naturally ("2" before "10").
    pub fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let mut entries = match self {
            Source::Local(s) => s.list(path)?,
            Source::WebDav(s) => s.list(path)?,
            Source::Smb(s) => s.list(path)?,
        };
        entries.retain(|e| !e.name.starts_with('.') && (e.is_dir || e.media_kind().is_some()));
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| names::natural_cmp(&a.name, &b.name)));
        Ok(entries)
    }

    /// Everything in a folder, unfiltered (for subtitle files next to a video).
    pub fn list_all(&self, path: &str) -> Result<Vec<Entry>> {
        match self {
            Source::Local(s) => s.list(path),
            Source::WebDav(s) => s.list(path),
            Source::Smb(s) => s.list(path),
        }
    }

    pub fn open(&self, path: &str) -> Result<Box<dyn ReadSeek>> {
        match self {
            Source::Local(s) => s.open(path),
            Source::WebDav(s) => s.open(path),
            Source::Smb(s) => s.open(path),
        }
    }

    /// A URL mpv can play without going through [`Source::open`], if this kind of source has one.
    pub fn direct_url(&self, path: &str) -> Option<DirectUrl> {
        match self {
            Source::Local(s) => Some(s.direct_url(path)),
            Source::WebDav(s) => Some(s.direct_url(path)),
            Source::Smb(_) => None,
        }
    }
}

/// Join a folder path and a name.
pub(crate) fn join(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_media() {
        assert_eq!(media_kind("Movie (2020).MKV"), Some(MediaKind::Video));
        assert_eq!(media_kind("01 - Song.flac"), Some(MediaKind::Audio));
        assert_eq!(media_kind("cover.jpg"), None);
        assert_eq!(media_kind("README"), None);
    }

    #[test]
    fn local_listing_filters_and_sorts() {
        let dir = std::env::temp_dir().join(format!("encheo-vfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Season 10")).unwrap();
        std::fs::create_dir_all(dir.join("Season 2")).unwrap();
        for f in ["Episode 10.mkv", "Episode 2.mkv", "notes.txt", ".hidden.mkv", "Episode 2.srt"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let src = Source::connect(&Config {
            kind: Kind::Local,
            name: "t".into(),
            location: dir.to_string_lossy().into(),
            username: String::new(),
            password: String::new(),
        })
        .unwrap();
        let names: Vec<_> = src.list("").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Season 2", "Season 10", "Episode 2.mkv", "Episode 10.mkv"]);
        let mut f = src.open("Episode 2.mkv").unwrap();
        let mut buf = String::new();
        f.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "x");
        assert_eq!(src.list("Season 2").unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
