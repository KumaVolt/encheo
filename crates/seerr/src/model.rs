//! What Seerr sends back, reduced to what a player shows: a title's state (available, requested,
//! downloading and how far along) and what the signed-in user may request.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Film or show, as Seerr names them in paths and requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Movie,
    Tv,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Tv => "tv",
        }
    }
}

/// Seerr's settings anyone can read; also how we know an address is a Seerr at all.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PublicSettings {
    pub initialized: bool,
    pub application_title: String,
    /// 1 Plex, 2 Jellyfin, 3 Emby, 4 not set up.
    pub media_server_type: u8,
    pub local_login: bool,
    pub media_server_login: bool,
    pub movie_4k_enabled: bool,
    pub series_4k_enabled: bool,
    pub partial_requests_enabled: bool,
}

impl PublicSettings {
    pub fn uses_plex(&self) -> bool {
        self.media_server_type == 1
    }
    pub fn uses_jellyfin(&self) -> bool {
        matches!(self.media_server_type, 2 | 3)
    }
}

// Seerr's permission bits (server/lib/permissions.ts).
const ADMIN: u64 = 2;
const REQUEST: u64 = 32;
const AUTO_APPROVE: u64 = 128;
const AUTO_APPROVE_MOVIE: u64 = 256;
const AUTO_APPROVE_TV: u64 = 512;
const REQUEST_4K: u64 = 1024;
const REQUEST_4K_MOVIE: u64 = 2048;
const REQUEST_4K_TV: u64 = 4096;
const REQUEST_MOVIE: u64 = 262_144;
const REQUEST_TV: u64 = 524_288;

/// The signed-in Seerr user.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub id: u64,
    pub display_name: String,
    pub permissions: u64,
}

impl User {
    fn has(&self, any: &[u64]) -> bool {
        self.permissions & ADMIN != 0 || any.iter().any(|p| self.permissions & p != 0)
    }

    /// Whether Seerr lets this user ask for the title at all.
    pub fn can_request(&self, kind: Kind, is_4k: bool) -> bool {
        match (kind, is_4k) {
            (Kind::Movie, false) => self.has(&[REQUEST, REQUEST_MOVIE]),
            (Kind::Tv, false) => self.has(&[REQUEST, REQUEST_TV]),
            (Kind::Movie, true) => self.has(&[REQUEST_4K, REQUEST_4K_MOVIE]),
            (Kind::Tv, true) => self.has(&[REQUEST_4K, REQUEST_4K_TV]),
        }
    }

    /// Whether this user's requests go straight to Radarr or Sonarr, without waiting for approval.
    pub fn auto_approved(&self, kind: Kind) -> bool {
        match kind {
            Kind::Movie => self.has(&[AUTO_APPROVE, AUTO_APPROVE_MOVIE]),
            Kind::Tv => self.has(&[AUTO_APPROVE, AUTO_APPROVE_TV]),
        }
    }
}

/// Seerr's media status (server/constants/media.ts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "u8")]
pub enum MediaStatus {
    #[default]
    Unknown,
    Pending,
    Processing,
    PartiallyAvailable,
    Available,
    Blocklisted,
    Deleted,
}

impl From<u8> for MediaStatus {
    fn from(n: u8) -> Self {
        match n {
            2 => Self::Pending,
            3 => Self::Processing,
            4 => Self::PartiallyAvailable,
            5 => Self::Available,
            6 => Self::Blocklisted,
            7 => Self::Deleted,
            _ => Self::Unknown,
        }
    }
}

/// A request's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "u8")]
pub enum RequestStatus {
    #[default]
    Pending,
    Approved,
    Declined,
    Failed,
    Completed,
}

impl From<u8> for RequestStatus {
    fn from(n: u8) -> Self {
        match n {
            2 => Self::Approved,
            3 => Self::Declined,
            4 => Self::Failed,
            5 => Self::Completed,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SeasonStatus {
    pub season_number: u32,
    pub status: MediaStatus,
    #[serde(rename = "status4k")]
    pub status_4k: MediaStatus,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SeasonRequest {
    pub season_number: u32,
}

/// A request someone made for the title.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaRequest {
    pub id: u64,
    pub status: RequestStatus,
    pub is_4k: bool,
    pub seasons: Vec<SeasonRequest>,
}

impl MediaRequest {
    /// Still on its way: waiting for approval, or approved and not yet in the library.
    pub fn open(&self) -> bool {
        matches!(self.status, RequestStatus::Pending | RequestStatus::Approved)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Episode {
    pub season_number: u32,
    pub episode_number: u32,
}

/// One file Radarr or Sonarr is downloading, as Seerr last saw it in their queues.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Download {
    pub title: String,
    pub size: u64,
    pub size_left: u64,
    /// The download client's state: "downloading", "queued", "paused", "warning", …
    pub status: String,
    /// Radarr's and Sonarr's "hh:mm:ss" or "d.hh:mm:ss".
    pub time_left: Option<String>,
    /// When it should finish, ISO 8601.
    pub estimated_completion_time: Option<String>,
    pub episode: Option<Episode>,
}

/// What Seerr knows about the title on its media server and in Radarr or Sonarr.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaInfo {
    pub status: MediaStatus,
    #[serde(rename = "status4k")]
    pub status_4k: MediaStatus,
    pub seasons: Vec<SeasonStatus>,
    pub requests: Vec<MediaRequest>,
    #[serde(rename = "downloadStatus")]
    pub downloads: Vec<Download>,
    #[serde(rename = "downloadStatus4k")]
    pub downloads_4k: Vec<Download>,
}

/// A season of a show, for choosing which to request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Season {
    pub number: u32,
    pub name: String,
    pub episodes: u32,
}

/// A film or show as Seerr (and TMDB behind it) describes it.
#[derive(Clone, Debug)]
pub struct Title {
    pub kind: Kind,
    pub tmdb_id: u64,
    pub title: String,
    pub year: Option<u32>,
    pub overview: String,
    /// TMDB image paths; see [`image_url`].
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// Only filled by [`crate::Seerr::title`]; specials (season 0) are left out.
    pub seasons: Vec<Season>,
    pub media: Option<MediaInfo>,
}

/// A TMDB image path as an address, `width` one of TMDB's sizes (posters 92, 154, 185, 342, 500,
/// 780; backdrops 300, 780, 1280).
pub fn image_url(path: &str, width: u32) -> String {
    format!("https://image.tmdb.org/t/p/w{width}{path}")
}

/// How far along the downloads are, summed over every file of the title.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub size: u64,
    pub size_left: u64,
    /// Files being fetched (one per episode for a show).
    pub files: usize,
    /// The longest of the files' time left: when the last one should finish.
    pub time_left: Option<Duration>,
    pub estimated_completion: Option<String>,
    /// Every file is paused or waiting in the queue.
    pub stalled: bool,
}

impl Progress {
    pub fn of(downloads: &[Download]) -> Option<Progress> {
        if downloads.is_empty() {
            return None;
        }
        let mut p = Progress { files: downloads.len(), ..Default::default() };
        for d in downloads {
            p.size += d.size;
            p.size_left += d.size_left.min(d.size);
            if let Some(t) = d.time_left.as_deref().and_then(parse_time_left) {
                p.time_left = Some(p.time_left.map_or(t, |m: Duration| m.max(t)));
            }
            if let Some(e) = &d.estimated_completion_time
                && p.estimated_completion.as_ref().is_none_or(|m| e > m)
            {
                p.estimated_completion = Some(e.clone());
            }
        }
        p.stalled = downloads.iter().all(|d| matches!(d.status.as_str(), "paused" | "queued" | "delay" | "warning"));
        Some(p)
    }

    /// Done so far, 0.0 to 1.0; `None` before the size is known.
    pub fn fraction(&self) -> Option<f32> {
        (self.size > 0).then(|| 1.0 - self.size_left as f32 / self.size as f32)
    }
}

/// "00:21:43" or "1.02:03:04" (days.hours:minutes:seconds).
pub fn parse_time_left(s: &str) -> Option<Duration> {
    let (days, rest) = match s.split_once('.') {
        Some((d, r)) if !d.contains(':') => (d.parse::<u64>().ok()?, r),
        _ => (0, s),
    };
    let rest = rest.split('.').next()?; // drop fractions of a second
    let mut parts = rest.split(':').map(|p| p.parse::<u64>().ok());
    let (h, m, sec) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Some(Duration::from_secs(((days * 24 + h) * 60 + m) * 60 + sec))
}

/// Where a title stands for the user, for the button on its page.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Nobody has asked for it.
    NotRequested,
    /// Asked for; waiting for an admin to approve.
    Requested,
    /// Approved; Radarr or Sonarr is looking for it.
    Processing,
    /// Being downloaded.
    Downloading(Progress),
    /// Some seasons or episodes are there; more may be downloading.
    PartlyAvailable(Option<Progress>),
    Available,
    /// An admin blocked it from being requested.
    Blocked,
}

impl Title {
    /// The title's state in standard quality, or in 4K.
    pub fn state(&self, is_4k: bool) -> State {
        let Some(m) = &self.media else { return State::NotRequested };
        let (status, downloads) = if is_4k { (m.status_4k, &m.downloads_4k) } else { (m.status, &m.downloads) };
        let progress = Progress::of(downloads);
        match status {
            MediaStatus::Available => State::Available,
            MediaStatus::PartiallyAvailable => State::PartlyAvailable(progress),
            MediaStatus::Blocklisted => State::Blocked,
            MediaStatus::Processing | MediaStatus::Pending if progress.is_some() => State::Downloading(progress.unwrap()),
            MediaStatus::Processing => State::Processing,
            MediaStatus::Pending => State::Requested,
            MediaStatus::Unknown | MediaStatus::Deleted => {
                // A request can exist before Seerr has a status for the media.
                match m.requests.iter().filter(|r| r.is_4k == is_4k && r.open()).map(|r| r.status).max_by_key(|s| *s as u8) {
                    Some(RequestStatus::Approved) => State::Processing,
                    Some(_) => State::Requested,
                    None => State::NotRequested,
                }
            }
        }
    }

    /// One season's state, for the season picker: already there, on its way, or free to request.
    pub fn season_state(&self, number: u32, is_4k: bool) -> SeasonState {
        let Some(m) = &self.media else { return SeasonState::Missing };
        let status = m.seasons.iter().find(|s| s.season_number == number).map(|s| if is_4k { s.status_4k } else { s.status });
        match status {
            Some(MediaStatus::Available) => return SeasonState::Available,
            Some(MediaStatus::PartiallyAvailable) => return SeasonState::Partial,
            Some(MediaStatus::Pending | MediaStatus::Processing) => return SeasonState::Requested,
            _ => {}
        }
        let asked = m.requests.iter().any(|r| r.is_4k == is_4k && r.open() && r.seasons.iter().any(|s| s.season_number == number));
        if asked { SeasonState::Requested } else { SeasonState::Missing }
    }

    /// Seasons nobody has asked for yet: what "Request" would ask for.
    pub fn missing_seasons(&self, is_4k: bool) -> Vec<u32> {
        self.seasons.iter().map(|s| s.number).filter(|&n| self.season_state(n, is_4k) == SeasonState::Missing).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeasonState {
    Missing,
    Requested,
    Partial,
    Available,
}

// Wire shapes ------------------------------------------------------------------------------------

/// A search result or a details page: films call their name `title`, shows `name`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RawTitle {
    pub id: u64,
    pub media_type: Option<String>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub seasons: Vec<RawSeason>,
    pub media_info: Option<MediaInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RawSeason {
    pub season_number: u32,
    pub name: String,
    pub episode_count: u32,
}

impl RawTitle {
    pub fn into_title(self, kind: Kind) -> Title {
        let date = self.release_date.or(self.first_air_date).unwrap_or_default();
        Title {
            kind,
            tmdb_id: self.id,
            title: self.title.or(self.name).unwrap_or_default(),
            year: date.get(..4).and_then(|y| y.parse().ok()),
            overview: self.overview.unwrap_or_default(),
            poster_path: self.poster_path.filter(|p| p.starts_with('/')),
            backdrop_path: self.backdrop_path.filter(|p| p.starts_with('/')),
            seasons: self
                .seasons
                .into_iter()
                .filter(|s| s.season_number > 0)
                .map(|s| Season { number: s.season_number, name: s.name, episodes: s.episode_count })
                .collect(),
            media: self.media_info,
        }
    }

    pub fn kind(&self) -> Option<Kind> {
        match self.media_type.as_deref() {
            Some("movie") => Some(Kind::Movie),
            Some("tv") => Some(Kind::Tv),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SearchPage {
    pub results: Vec<RawTitle>,
}
