//! Plex JSON shapes, trimmed to the fields Encheo uses. Plex sends many more; serde ignores them.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: u64,
    pub username: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub profile: Option<Profile>,
}

/// The account's playback preferences, set in any Plex app.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// ISO 639-1, e.g. "en". Empty when unset.
    #[serde(default)]
    pub default_subtitle_language: Option<String>,
    #[serde(default)]
    pub default_audio_language: Option<String>,
}

/// A device on the account from `/api/v2/resources`. Servers are the ones that provide "server".
/// A person in the account's Plex Home ("Who's watching?"). Each has their own watched list,
/// Continue Watching and watchlist.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeUser {
    pub uuid: String,
    #[serde(default)]
    pub title: String,
    /// Avatar picture (a full plex.tv address).
    #[serde(default)]
    pub thumb: Option<String>,
    /// The account owner.
    #[serde(default)]
    pub admin: bool,
    /// Switching to this user needs their PIN.
    #[serde(default)]
    pub protected: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub name: String,
    pub client_identifier: String,
    #[serde(default)]
    pub provides: String,
    #[serde(default)]
    pub owned: bool,
    /// Token for this server. Differs from the account token on servers shared with the user.
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub uri: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub relay: bool,
    #[serde(default, rename = "IPv6")]
    pub ipv6: bool,
}

impl Resource {
    pub fn is_server(&self) -> bool {
        self.provides.split(',').any(|p| p.trim() == "server")
    }

    /// Addresses in the order worth trying: local first (fast, no bandwidth cap), then direct
    /// remote, then Plex's relay (slow and capped, last resort). IPv4 before IPv6 within each group.
    pub fn connection_order(&self) -> Vec<&Connection> {
        let mut c: Vec<&Connection> = self.connections.iter().collect();
        c.sort_by_key(|c| (c.relay, !c.local, c.ipv6));
        c
    }

    /// URLs to probe, best first. Like [`Resource::connection_order`], plus a plain
    /// `http://<ip>:<port>` right after each local `plex.direct` address: many routers block
    /// plex.direct names that resolve to private IPs (DNS rebinding protection), and without this
    /// fallback we'd stream through the public IP instead.
    pub fn candidate_urls(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in self.connection_order() {
            out.push(c.uri.trim_end_matches('/').to_string());
            if c.local && !c.address.is_empty() && c.port != 0 && c.uri.contains(".plex.direct") {
                let host = if c.ipv6 { format!("[{}]", c.address) } else { c.address.clone() };
                out.push(format!("http://{host}:{}", c.port));
            }
        }
        out.dedup();
        out
    }
}

/// Every Plex response wraps its payload in a `MediaContainer`.
#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<T> {
    #[serde(rename = "MediaContainer")]
    pub container: T,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Directories {
    #[serde(default, rename = "Directory")]
    pub directory: Vec<Library>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataPage {
    #[serde(default)]
    pub total_size: Option<u32>,
    #[serde(default)]
    pub size: u32,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<Item>,
}

/// A library section: Movies, TV Shows, Music.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub key: String,
    pub title: String,
    /// `movie`, `show`, `artist`, `photo`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
}

/// `/hubs/search` groups results by type. Shows and artists come back as `Directory`,
/// everything else as `Metadata`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Hubs {
    #[serde(default, rename = "Hub")]
    pub hub: Vec<Hub>,
}

/// Entries are kept as raw JSON: hubs also hold people, tags and other things that aren't items,
/// and one odd entry must not fail the whole search.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Hub {
    #[serde(default)]
    pub title: String,
    /// What the hub is, e.g. `movie.recentlyreleased.1`, `tv.moreingenre.2`.
    #[serde(default, rename = "hubIdentifier")]
    pub identifier: String,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<serde_json::Value>,
    #[serde(default, rename = "Directory")]
    pub directory: Vec<serde_json::Value>,
    /// The hub's full list, which pages with X-Plex-Container-Start/Size.
    #[serde(default)]
    pub key: Option<String>,
    /// Whether the full list has more than the hub carries.
    #[serde(default, deserialize_with = "flag")]
    pub more: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct WithPreferences {
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<PreferencesHolder>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PreferencesHolder {
    #[serde(default, rename = "Preferences")]
    pub preferences: Option<Preferences>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Preferences {
    #[serde(default, rename = "Setting")]
    pub settings: Vec<Setting>,
}

/// One per-item server setting, e.g. a show's `audioLanguage`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub id: String,
    #[serde(default, deserialize_with = "string_or_number")]
    pub value: String,
    /// "-1:Library default|0:Oldest first|…" or ":Account default|ja:日本語|en:English|…".
    #[serde(default)]
    pub enum_values: String,
}

impl Setting {
    /// The allowed values (the part before each ':').
    pub fn choices(&self) -> impl Iterator<Item = &str> {
        self.enum_values.split('|').filter_map(|e| e.split_once(':').map(|(v, _)| v))
    }
}

fn string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        v => v.to_string(),
    })
}

/// A subtitle Plex found online (OpenSubtitles) for an item.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleResult {
    /// Pass to [`Server::pick_subtitle`](crate::Server::pick_subtitle).
    pub key: String,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// Usually the release name the subtitle was made for.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub provider_title: Option<String>,
    /// Higher is a better match (Plex sends it as a string).
    #[serde(default, deserialize_with = "string_or_number")]
    pub score: String,
    #[serde(default)]
    pub hearing_impaired: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SubtitleResults {
    #[serde(default, rename = "Stream")]
    pub streams: Vec<SubtitleResult>,
}

/// A row of titles for Home, and where to page for more of it.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub title: String,
    pub items: Vec<Item>,
    /// A server path that pages (Plex: X-Plex-Container-Start/Size; Jellyfin: StartIndex/Limit).
    pub more: Option<String>,
}

/// A page of a library or list.
#[derive(Clone, Debug)]
pub struct Page {
    pub items: Vec<Item>,
    /// Total items in the full list, when Plex reports it.
    pub total: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Movie,
    Show,
    Season,
    Episode,
    Collection,
    Clip,
    Artist,
    Album,
    Track,
    Playlist,
    /// Not from Plex: a folder in a file source.
    Folder,
    #[default]
    #[serde(other)]
    Other,
}

/// A movie, show, season or episode.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub rating_key: String,
    #[serde(default)]
    pub key: String,
    /// Plex's global id, e.g. `plex://movie/5d77…`; the same on every server.
    #[serde(default)]
    pub guid: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: ItemKind,
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub content_rating: Option<String>,
    #[serde(default)]
    pub audience_rating: Option<f32>,
    #[serde(default)]
    pub originally_available_at: Option<String>,
    /// Poster. A server path; turn it into a URL with [`Server::image_url`](crate::Server::image_url).
    #[serde(default)]
    pub thumb: Option<String>,
    /// Playlists: a picture Plex makes from their albums' covers.
    #[serde(default)]
    pub composite: Option<String>,
    /// Backdrop.
    #[serde(default)]
    pub art: Option<String>,
    /// Transparent title logo, when the server has one.
    #[serde(default)]
    pub logo: Option<String>,
    /// Length in milliseconds.
    #[serde(default)]
    pub duration: Option<u64>,
    /// Resume point in milliseconds.
    #[serde(default)]
    pub view_offset: Option<u64>,
    #[serde(default)]
    pub view_count: Option<u32>,
    /// Unix seconds when the server added it.
    #[serde(default)]
    pub added_at: Option<i64>,

    // Episodes and seasons.
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub parent_index: Option<u32>,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub parent_rating_key: Option<String>,
    /// Album cover, on tracks.
    #[serde(default)]
    pub parent_thumb: Option<String>,
    /// Tracks: the performing artist when it differs from the album artist.
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub grandparent_rating_key: Option<String>,
    #[serde(default)]
    pub grandparent_thumb: Option<String>,
    #[serde(default)]
    pub grandparent_art: Option<String>,
    /// Shows and seasons: number of episodes, and how many are watched.
    #[serde(default)]
    pub leaf_count: Option<u32>,
    #[serde(default)]
    pub viewed_leaf_count: Option<u32>,
    #[serde(default)]
    pub child_count: Option<u32>,

    /// Extras: what kind ("1" is a trailer), and the same as a word ("trailer").
    #[serde(default, deserialize_with = "string_or_number")]
    pub extra_type: String,
    #[serde(default)]
    pub subtype: Option<String>,

    #[serde(default, rename = "Genre")]
    pub genres: Vec<Tag>,
    /// Where it was made (full metadata only).
    #[serde(default, rename = "Country")]
    pub countries: Vec<Tag>,
    #[serde(default, rename = "Director")]
    pub directors: Vec<Tag>,
    #[serde(default, rename = "Writer")]
    pub writers: Vec<Tag>,
    #[serde(default, rename = "Producer")]
    pub producers: Vec<Tag>,
    #[serde(default, rename = "Rating")]
    pub ratings: Vec<Rating>,
    #[serde(default, rename = "Role")]
    pub cast: Vec<Role>,
    #[serde(default, rename = "Media")]
    pub media: Vec<Media>,
    #[serde(default, rename = "Marker")]
    pub markers: Vec<Marker>,
}

impl Item {
    pub fn is_watched(&self) -> bool {
        match self.kind {
            ItemKind::Show | ItemKind::Season => {
                matches!((self.leaf_count, self.viewed_leaf_count), (Some(t), Some(v)) if t > 0 && v >= t)
            }
            _ => self.view_count.unwrap_or(0) > 0,
        }
    }

    /// 0.0–1.0 through the item, for the progress bar on a poster. `None` when not started.
    pub fn progress(&self) -> Option<f32> {
        let (off, dur) = (self.view_offset?, self.duration?);
        (off > 0 && dur > 0).then(|| (off as f32 / dur as f32).min(1.0))
    }

    /// The file to play: the first part of the first media version.
    pub fn primary_part(&self) -> Option<&Part> {
        self.media.first()?.parts.first()
    }

    /// Tracks: who performed it (track artist, else album artist).
    pub fn artist(&self) -> Option<&str> {
        self.original_title.as_deref().or(self.grandparent_title.as_deref())
    }

    /// "S2 · E5" for episodes.
    pub fn episode_label(&self) -> Option<String> {
        Some(format!("S{} · E{}", self.parent_index?, self.index?))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Tag {
    pub tag: String,
    /// Photo, on people (directors, writers, producers).
    #[serde(default)]
    pub thumb: Option<String>,
}

/// A score from an outside source, e.g. `imdb://image.rating` 6.7 or
/// `rottentomatoes://image.rating.ripe` 9.4 (= 94%).
#[derive(Clone, Debug, Deserialize)]
pub struct Rating {
    pub image: String,
    pub value: f32,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Role {
    pub tag: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
}

/// One version of an item (a movie can have a 4K and a 1080p file).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub video_resolution: Option<String>,
    #[serde(default)]
    pub video_codec: Option<String>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub audio_channels: Option<u32>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub video_dynamic_range: Option<String>,
    /// "24p", "PAL", "NTSC", "60p".
    #[serde(default)]
    pub video_frame_rate: Option<String>,
    #[serde(default, rename = "Part")]
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    #[serde(default)]
    pub id: u64,
    /// Server path of the file, e.g. `/library/parts/123/1700000000/file.mkv`.
    pub key: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub container: Option<String>,
    /// Video, audio and subtitle streams. Only present on full metadata, not in lists.
    #[serde(default, rename = "Stream")]
    pub streams: Vec<Stream>,
}

impl Part {
    /// "Dolby Vision", "HDR10", "HLG", or `None` for SDR, from the video stream.
    pub fn dynamic_range(&self) -> Option<&'static str> {
        let v = self.streams_of(StreamKind::Video).next()?;
        if v.dovi_present {
            Some("Dolby Vision")
        } else {
            match v.color_trc.as_deref() {
                Some("smpte2084") => Some("HDR10"),
                Some("arib-std-b67") => Some("HLG"),
                _ => None,
            }
        }
    }

    pub fn streams_of(&self, kind: StreamKind) -> impl Iterator<Item = &Stream> {
        self.streams.iter().filter(move |s| s.stream_type == kind as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Video = 1,
    Audio = 2,
    Subtitle = 3,
}

/// A track inside a file, as Plex knows it: Plex's language detection and naming, and which one
/// the user last chose (`selected`), shared by every Plex app.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub id: u64,
    pub stream_type: u8,
    /// Position in the container. `None` for external subtitle files next to the video.
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub language_code: Option<String>,
    /// "English (EAC3 Stereo)", "English Forced (SRT)".
    #[serde(default)]
    pub extended_display_title: Option<String>,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub selected: bool,
    /// Video: transfer function; `smpte2084` = PQ (HDR10), `arib-std-b67` = HLG.
    #[serde(default)]
    pub color_trc: Option<String>,
    #[serde(default, rename = "DOVIPresent")]
    pub dovi_present: bool,
    #[serde(default)]
    pub frame_rate: Option<f32>,
    /// For external subtitle files: server path to fetch it from.
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkerKind {
    Intro,
    Credits,
    Commercial,
    #[serde(other)]
    Other,
}

/// A skippable segment, times in milliseconds.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    #[serde(rename = "type")]
    pub kind: MarkerKind,
    pub start_time_offset: u64,
    pub end_time_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

impl PlayState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Stopped => "stopped",
        }
    }
}

/// A song's words. Shared by every source (Plex, Jellyfin, LRCLIB).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// Each line carries the moment it's sung. Otherwise the words are just text.
    pub synced: bool,
    /// Who wrote or licensed the words ("Lyrics © …"), when the source says.
    pub credit: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricLine {
    /// Seconds into the song. 0 for every line when not synced.
    pub start: f64,
    pub text: String,
}

/// Plex's lyrics, as JSON: `MediaContainer.Lyrics[].Line[]`, each line with a `startOffset` in
/// milliseconds and its words in `Span`s.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct LyricsContainer {
    #[serde(default, rename = "Lyrics")]
    pub lyrics: Vec<RawLyrics>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawLyrics {
    #[serde(default, deserialize_with = "flag")]
    timed: bool,
    #[serde(default)]
    by: Option<String>,
    #[serde(default, rename = "Line")]
    lines: Vec<RawLine>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLine {
    #[serde(default)]
    start_offset: Option<f64>,
    #[serde(default, rename = "Span")]
    spans: Vec<RawSpan>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSpan {
    #[serde(default)]
    text: String,
}

/// Plex writes booleans as `true`, `1` or `"1"` depending on the endpoint.
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_f64() != Some(0.0),
        serde_json::Value::String(s) => matches!(s.as_str(), "1" | "true"),
        _ => false,
    })
}

impl RawLyrics {
    pub(crate) fn into_lyrics(self) -> Lyrics {
        let synced = self.timed && self.lines.iter().any(|l| l.start_offset.is_some());
        let lines = self
            .lines
            .into_iter()
            .map(|l| LyricLine {
                start: if synced { l.start_offset.unwrap_or(0.0) / 1000.0 } else { 0.0 },
                text: l.spans.into_iter().map(|s| s.text).collect::<String>().trim().to_string(),
            })
            .collect::<Vec<_>>();
        Lyrics { lines, synced, credit: self.by.filter(|b| !b.trim().is_empty()) }
    }
}
