//! Jellyfin and Emby client. Jellyfin started as a fork of Emby and both still speak the same
//! user-scoped API, so one client covers both; differences are noted where they matter.
//!
//! Items come back as [`encheo_plex::Item`], the app's item shape, with ids prefixed so the app
//! can tell which server an item belongs to. Image fields hold full URLs.

use encheo_plex::{Item, ItemKind, Library, Marker, MarkerKind, Media, Part, PlayState, Rating, Role, Stream, Tag};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("wrong user name or password")]
    Auth,
    #[error("server answered {0}")]
    Status(u16),
    #[error("not a Jellyfin or Emby server")]
    NotAServer,
    #[error("unexpected response: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Flavor {
    Jellyfin,
    Emby,
}

/// Who we are; shown in the server's device list.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub device_id: String,
    pub device_name: String,
    pub version: String,
}

/// A signed-in session, ready to save.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Login {
    pub base_url: String,
    pub server_name: String,
    pub flavor: Flavor,
    pub user_id: String,
    pub user_name: String,
    pub token: String,
}

#[derive(Clone)]
pub struct Server {
    http: reqwest::Client,
    login: Login,
    client: ClientInfo,
    /// Prepended to every item id, e.g. "jf0:".
    prefix: String,
}

const FIELDS: &str = "Overview,Genres,People,MediaSources,MediaStreams,ProviderIds,OfficialRating,CriticRating,CommunityRating,ProductionYear,DateCreated,ChildCount,RecursiveItemCount,ParentId,Path,Taglines";

fn http() -> reqwest::Client {
    encheo_plex::with_platform_roots(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .build()
        .expect("http client")
}

fn auth_header(client: &ClientInfo, token: Option<&str>) -> String {
    let mut h = format!(
        r#"MediaBrowser Client="Encheo", Device="{}", DeviceId="{}", Version="{}""#,
        client.device_name.replace('"', ""),
        client.device_id,
        client.version
    );
    if let Some(t) = token {
        h.push_str(&format!(r#", Token="{t}""#));
    }
    h
}

/// Normalise what the user typed into a base URL: add http://, drop trailing slashes.
pub fn normalize_url(input: &str) -> String {
    let t = input.trim().trim_end_matches('/');
    if t.contains("://") { t.to_string() } else { format!("http://{t}") }
}

/// Check the address and sign in.
pub async fn sign_in(base_url: &str, username: &str, password: &str, client: &ClientInfo) -> Result<Login> {
    let base = normalize_url(base_url);
    let http = http();
    // Jellyfin answers here at the root; Emby at the root and under /emby.
    let mut info: Option<(String, Value)> = None;
    for candidate in [base.clone(), format!("{base}/emby")] {
        let r = http.get(format!("{candidate}/System/Info/Public")).header("Accept", "application/json").send().await;
        if let Ok(r) = r
            && r.status().is_success()
            && let Ok(v) = r.json::<Value>().await
        {
            info = Some((candidate, v));
            break;
        }
    }
    let (base, info) = info.ok_or(Error::NotAServer)?;
    let product = info["ProductName"].as_str().unwrap_or_default();
    let flavor = if product.to_ascii_lowercase().contains("jellyfin") { Flavor::Jellyfin } else { Flavor::Emby };
    let resp = http
        .post(format!("{base}/Users/AuthenticateByName"))
        .header("Accept", "application/json")
        .header("X-Emby-Authorization", auth_header(client, None))
        .header("Authorization", auth_header(client, None))
        .json(&serde_json::json!({ "Username": username, "Pw": password }))
        .send()
        .await?;
    match resp.status().as_u16() {
        401 | 403 => return Err(Error::Auth),
        s if !(200..300).contains(&s) => return Err(Error::Status(s)),
        _ => {}
    }
    let v: Value = resp.json().await?;
    let token = v["AccessToken"].as_str().ok_or_else(|| Error::Decode("no AccessToken".into()))?;
    Ok(Login {
        base_url: base,
        server_name: info["ServerName"].as_str().unwrap_or("Media server").to_string(),
        flavor,
        user_id: v["User"]["Id"].as_str().unwrap_or_default().to_string(),
        user_name: v["User"]["Name"].as_str().unwrap_or(username).to_string(),
        token: token.to_string(),
    })
}

#[derive(Deserialize)]
struct ItemsPage {
    #[serde(default, rename = "Items")]
    items: Vec<Value>,
    #[serde(default, rename = "TotalRecordCount")]
    total: Option<u32>,
}

impl Server {
    pub fn new(login: Login, client: ClientInfo, prefix: impl Into<String>) -> Self {
        Self { http: http(), login, client, prefix: prefix.into() }
    }

    pub fn login(&self) -> &Login {
        &self.login
    }

    pub fn name(&self) -> &str {
        &self.login.server_name
    }

    /// The server's own id for one of our item ids.
    pub fn raw_id<'a>(&self, id: &'a str) -> &'a str {
        id.strip_prefix(&self.prefix).unwrap_or(id)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.login.base_url)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(self.url(path))
            .header("Accept", "application/json")
            .header("X-Emby-Authorization", auth_header(&self.client, Some(&self.login.token)))
            .header("Authorization", auth_header(&self.client, Some(&self.login.token)))
            .header("X-Emby-Token", &self.login.token)
    }

    fn send(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.url(path))
            .header("X-Emby-Authorization", auth_header(&self.client, Some(&self.login.token)))
            .header("Authorization", auth_header(&self.client, Some(&self.login.token)))
            .header("X-Emby-Token", &self.login.token)
    }

    async fn json(&self, r: reqwest::RequestBuilder) -> Result<Value> {
        let resp = r.send().await?;
        match resp.status().as_u16() {
            401 | 403 => Err(Error::Auth),
            s if !(200..300).contains(&s) => Err(Error::Status(s)),
            _ => resp.json().await.map_err(|e| Error::Decode(e.to_string())),
        }
    }

    async fn items(&self, path: &str) -> Result<Vec<Item>> {
        let v = self.json(self.get(path)).await?;
        let list = match v {
            Value::Array(a) => a,
            v => serde_json::from_value::<ItemsPage>(v).map(|p| p.items).unwrap_or_default(),
        };
        Ok(list.iter().map(|i| self.to_item(i)).collect())
    }

    fn uid(&self) -> &str {
        &self.login.user_id
    }

    /// Movie, TV and music libraries.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let v = self.json(self.get(&format!("/Users/{}/Views", self.uid()))).await?;
        let page: ItemsPage = serde_json::from_value(v).map_err(|e| Error::Decode(e.to_string()))?;
        Ok(page
            .items
            .iter()
            .filter_map(|l| {
                let kind = match l["CollectionType"].as_str()? {
                    "movies" => "movie",
                    "tvshows" => "show",
                    "music" => "artist",
                    _ => return None,
                };
                Some(Library {
                    key: format!("{}{}", self.prefix, l["Id"].as_str()?),
                    title: l["Name"].as_str().unwrap_or_default().to_string(),
                    kind: kind.to_string(),
                    thumb: None,
                    art: None,
                })
            })
            .collect())
    }

    fn library_types(kind: &str) -> &'static str {
        match kind {
            "show" => "Series",
            "artist" => "MusicArtist,MusicAlbum",
            _ => "Movie",
        }
    }

    /// A page of a library, sorted by name. `kind` is the library's kind ("movie", "show", "artist").
    pub async fn library_items(&self, library_key: &str, kind: &str, start: u32, size: u32) -> Result<encheo_plex::Page> {
        let types = if kind == "artist" { "MusicAlbum" } else { Self::library_types(kind) };
        let path = format!(
            "/Users/{}/Items?ParentId={}&Recursive=true&IncludeItemTypes={types}&SortBy=SortName&SortOrder=Ascending&StartIndex={start}&Limit={size}&Fields={FIELDS}",
            self.uid(),
            self.raw_id(library_key)
        );
        let v = self.json(self.get(&path)).await?;
        let page: ItemsPage = serde_json::from_value(v).map_err(|e| Error::Decode(e.to_string()))?;
        Ok(encheo_plex::Page { total: page.total, items: page.items.iter().map(|i| self.to_item(i)).collect() })
    }

    pub async fn recently_added(&self, library_key: &str, size: u32) -> Result<Vec<Item>> {
        self.items(&format!(
            "/Users/{}/Items/Latest?ParentId={}&Limit={size}&Fields={FIELDS}",
            self.uid(),
            self.raw_id(library_key)
        ))
        .await
    }

    /// Partly watched items, then the next episode of shows in progress.
    pub async fn on_deck(&self) -> Result<Vec<Item>> {
        let mut out = self
            .items(&format!("/Users/{}/Items/Resume?Limit=20&MediaTypes=Video&Fields={FIELDS}", self.uid()))
            .await?;
        if let Ok(next) = self.items(&format!("/Shows/NextUp?UserId={}&Limit=20&Fields={FIELDS}", self.uid())).await {
            for n in next {
                if !out.iter().any(|o| o.grandparent_rating_key == n.grandparent_rating_key && n.grandparent_rating_key.is_some()) {
                    out.push(n);
                }
            }
        }
        Ok(out)
    }

    pub async fn metadata(&self, id: &str) -> Result<Option<Item>> {
        let v = self.json(self.get(&format!("/Users/{}/Items/{}?Fields={FIELDS}", self.uid(), self.raw_id(id)))).await?;
        let mut item = self.to_item(&v);
        if item.kind == ItemKind::Episode || item.kind == ItemKind::Movie {
            item.markers = self.markers(id).await.unwrap_or_default();
        }
        Ok(Some(item))
    }

    /// Intro and credits, from Jellyfin's media segments (10.10+).
    async fn markers(&self, id: &str) -> Result<Vec<Marker>> {
        if self.login.flavor != Flavor::Jellyfin {
            return Ok(vec![]);
        }
        let v = self.json(self.get(&format!("/MediaSegments/{}", self.raw_id(id)))).await?;
        Ok(v["Items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let kind = match s["Type"].as_str()? {
                    "Intro" => MarkerKind::Intro,
                    "Outro" => MarkerKind::Credits,
                    _ => return None,
                };
                Some(Marker {
                    kind,
                    start_time_offset: s["StartTicks"].as_u64()? / 10_000,
                    end_time_offset: s["EndTicks"].as_u64()? / 10_000,
                })
            })
            .collect())
    }

    /// Seasons of a show, episodes of a season, albums of an artist, tracks of an album.
    pub async fn children(&self, id: &str) -> Result<Vec<Item>> {
        let raw = self.raw_id(id);
        let parent = self.json(self.get(&format!("/Users/{}/Items/{raw}", self.uid()))).await?;
        let path = match parent["Type"].as_str().unwrap_or_default() {
            "Series" => format!("/Shows/{raw}/Seasons?UserId={}&Fields={FIELDS}", self.uid()),
            "Season" => format!(
                "/Shows/{}/Episodes?UserId={}&SeasonId={raw}&Fields={FIELDS}",
                parent["SeriesId"].as_str().unwrap_or_default(),
                self.uid()
            ),
            "MusicArtist" => format!(
                "/Users/{}/Items?ArtistIds={raw}&IncludeItemTypes=MusicAlbum&Recursive=true&SortBy=ProductionYear,SortName&Fields={FIELDS}",
                self.uid()
            ),
            _ => format!("/Users/{}/Items?ParentId={raw}&SortBy=ParentIndexNumber,IndexNumber,SortName&Fields={FIELDS}", self.uid()),
        };
        self.items(&path).await
    }

    pub async fn all_episodes(&self, show_id: &str) -> Result<Vec<Item>> {
        self.items(&format!("/Shows/{}/Episodes?UserId={}&Fields={FIELDS}", self.raw_id(show_id), self.uid())).await
    }

    pub async fn related(&self, id: &str) -> Result<Vec<Item>> {
        self.items(&format!("/Items/{}/Similar?UserId={}&Limit=20&Fields={FIELDS}", self.raw_id(id), self.uid())).await
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Item>> {
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        self.items(&format!(
            "/Users/{}/Items?SearchTerm={q}&Recursive=true&IncludeItemTypes=Movie,Series,Episode,MusicArtist,MusicAlbum,Audio&Limit=40&Fields={FIELDS}",
            self.uid()
        ))
        .await
    }

    /// Direct play: the file as it is.
    pub fn stream_url(&self, item: &Item) -> String {
        let raw = self.raw_id(&item.rating_key);
        let kind = if item.kind == ItemKind::Track { "Audio" } else { "Videos" };
        let source = item.media.first().and_then(|m| m.parts.first()).and_then(|p| p.file.clone()).unwrap_or_default();
        format!(
            "{}/{kind}/{raw}/stream?static=true&MediaSourceId={}&api_key={}",
            self.login.base_url,
            if source.is_empty() { raw.to_string() } else { source },
            self.login.token
        )
    }

    /// Where an external subtitle file can be fetched (`DeliveryUrl`), if the stream is one.
    pub fn subtitle_url(&self, stream: &Stream) -> Option<String> {
        stream.key.as_ref().map(|k| {
            let sep = if k.contains('?') { '&' } else { '?' };
            format!("{}{k}{sep}api_key={}", self.login.base_url, self.login.token)
        })
    }

    /// Tell the server where playback is (start, progress every few seconds, stop).
    ///
    /// Jellyfin only keeps a resume point when a start was reported first ([`Server::report_start`]),
    /// and never for videos under 5 minutes (those count as watched).
    pub async fn report_progress(&self, item: &Item, state: PlayState, position_ms: u64) -> Result<()> {
        let raw = self.raw_id(&item.rating_key);
        let body = serde_json::json!({
            "PlaySessionId": self.play_session(raw),
            "ItemId": raw,
            "MediaSourceId": item.media.first().and_then(|m| m.parts.first()).and_then(|p| p.file.clone()).unwrap_or_else(|| raw.to_string()),
            "PositionTicks": position_ms * 10_000,
            "IsPaused": state == PlayState::Paused,
            "CanSeek": true,
            "PlayMethod": "DirectPlay",
        });
        let path = match state {
            PlayState::Stopped => "/Sessions/Playing/Stopped",
            _ => "/Sessions/Playing/Progress",
        };
        let resp = self.send(reqwest::Method::POST, path).json(&body).send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(Error::Status(resp.status().as_u16()));
        }
        Ok(())
    }

    /// Tell the server playback started (makes it show in "now playing" on the dashboard).
    pub async fn report_start(&self, item: &Item) -> Result<()> {
        let raw = self.raw_id(&item.rating_key);
        let body = serde_json::json!({
            "ItemId": raw,
            "PlaySessionId": self.play_session(raw),
            "MediaSourceId": item.media.first().and_then(|m| m.parts.first()).and_then(|p| p.file.clone()).unwrap_or_else(|| raw.to_string()),
            "PositionTicks": item.view_offset.unwrap_or(0) * 10_000,
            "CanSeek": true,
            "PlayMethod": "DirectPlay",
        });
        self.send(reqwest::Method::POST, "/Sessions/Playing").json(&body).send().await?;
        Ok(())
    }

    /// One play session per item and device, so start, progress and stop line up.
    fn play_session(&self, raw_item: &str) -> String {
        format!("{}-{raw_item}", self.client.device_id)
    }

    pub async fn mark_watched(&self, id: &str) -> Result<()> {
        self.played(id, true).await
    }

    pub async fn mark_unwatched(&self, id: &str) -> Result<()> {
        self.played(id, false).await
    }

    async fn played(&self, id: &str, played: bool) -> Result<()> {
        let method = if played { reqwest::Method::POST } else { reqwest::Method::DELETE };
        let resp = self.send(method, &format!("/Users/{}/PlayedItems/{}", self.uid(), self.raw_id(id))).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Status(resp.status().as_u16()));
        }
        Ok(())
    }

    fn image(&self, id: &str, kind: &str, tag: Option<&str>) -> Option<String> {
        let tag = tag?;
        Some(format!("{}/Items/{id}/Images/{kind}?tag={tag}&quality=90", self.login.base_url))
    }

    /// Convert a server item to the app's item shape.
    pub fn to_item(&self, v: &Value) -> Item {
        let s = |k: &str| v[k].as_str().map(String::from);
        let id = s("Id").unwrap_or_default();
        let kind = match v["Type"].as_str().unwrap_or_default() {
            "Movie" => ItemKind::Movie,
            "Series" => ItemKind::Show,
            "Season" => ItemKind::Season,
            "Episode" => ItemKind::Episode,
            "MusicAlbum" => ItemKind::Album,
            "MusicArtist" => ItemKind::Artist,
            "Audio" => ItemKind::Track,
            "Folder" | "CollectionFolder" => ItemKind::Folder,
            _ => ItemKind::Other,
        };
        let pre = |x: Option<String>| x.map(|x| format!("{}{x}", self.prefix));
        let ticks_ms = |k: &str| v[k].as_u64().map(|t| t / 10_000);
        let user = &v["UserData"];
        let tags = &v["ImageTags"];
        let primary = self.image(&id, "Primary", tags["Primary"].as_str());
        let backdrop = v["BackdropImageTags"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|t| self.image(&id, "Backdrop", t.as_str()))
            .or_else(|| {
                let pid = v["ParentBackdropItemId"].as_str()?;
                let tag = v["ParentBackdropImageTags"].as_array()?.first()?.as_str()?;
                self.image(pid, "Backdrop", Some(tag))
            });
        let series_thumb = v["SeriesId"].as_str().and_then(|sid| self.image(sid, "Primary", v["SeriesPrimaryImageTag"].as_str()));
        // Tracks show their album cover.
        let album_thumb = v["AlbumId"].as_str().and_then(|aid| self.image(aid, "Primary", v["AlbumPrimaryImageTag"].as_str()));

        let mut item = Item {
            rating_key: format!("{}{id}", self.prefix),
            key: id.clone(),
            kind: kind.clone(),
            title: s("Name").unwrap_or_default(),
            year: v["ProductionYear"].as_u64().map(|y| y as u32),
            summary: s("Overview"),
            tagline: v["Taglines"].as_array().and_then(|t| t.first()).and_then(|t| t.as_str()).map(String::from),
            content_rating: s("OfficialRating"),
            audience_rating: v["CommunityRating"].as_f64().map(|r| r as f32),
            thumb: primary.clone(),
            art: backdrop,
            duration: ticks_ms("RunTimeTicks"),
            view_offset: user["PlaybackPositionTicks"].as_u64().map(|t| t / 10_000).filter(|o| *o > 0),
            view_count: user["Played"].as_bool().map(|p| u32::from(p)),
            index: v["IndexNumber"].as_u64().map(|n| n as u32),
            parent_index: v["ParentIndexNumber"].as_u64().map(|n| n as u32),
            parent_title: s("SeasonName").or_else(|| s("Album")).or_else(|| s("AlbumArtist")),
            parent_rating_key: pre(s("SeasonId").or_else(|| s("AlbumId"))),
            parent_thumb: album_thumb,
            grandparent_title: s("SeriesName").or_else(|| s("AlbumArtist")),
            grandparent_rating_key: pre(s("SeriesId")),
            grandparent_thumb: series_thumb,
            grandparent_art: None,
            original_title: v["Artists"].as_array().and_then(|a| a.first()).and_then(|a| a.as_str()).map(String::from),
            leaf_count: v["RecursiveItemCount"].as_u64().map(|n| n as u32).filter(|_| kind == ItemKind::Show || kind == ItemKind::Season),
            viewed_leaf_count: match (v["RecursiveItemCount"].as_u64(), user["UnplayedItemCount"].as_u64()) {
                (Some(t), Some(u)) => Some(t.saturating_sub(u) as u32),
                _ => None,
            },
            child_count: v["ChildCount"].as_u64().map(|n| n as u32),
            added_at: None,
            ..Default::default()
        };
        if kind == ItemKind::Album {
            item.parent_title = s("AlbumArtist");
        }
        item.genres = v["Genres"].as_array().into_iter().flatten().filter_map(|g| g.as_str()).map(|g| Tag { tag: g.into(), thumb: None }).collect();
        for p in v["People"].as_array().into_iter().flatten() {
            let name = p["Name"].as_str().unwrap_or_default().to_string();
            let thumb = p["Id"].as_str().and_then(|pid| self.image(pid, "Primary", p["PrimaryImageTag"].as_str()));
            match p["Type"].as_str() {
                Some("Actor") | Some("GuestStar") => item.cast.push(Role { tag: name, role: p["Role"].as_str().map(String::from), thumb }),
                Some("Director") => item.directors.push(Tag { tag: name, thumb }),
                Some("Writer") => item.writers.push(Tag { tag: name, thumb }),
                Some("Producer") => item.producers.push(Tag { tag: name, thumb }),
                _ => {}
            }
        }
        if let Some(c) = v["CriticRating"].as_f64() {
            let image = if c >= 60.0 { "rottentomatoes://image.rating.ripe" } else { "rottentomatoes://image.rating.rotten" };
            item.ratings.push(Rating { image: image.into(), value: (c / 10.0) as f32, kind: "critic".into() });
        }
        if let Some(c) = v["CommunityRating"].as_f64() {
            item.ratings.push(Rating { image: "community://rating".into(), value: c as f32, kind: "audience".into() });
        }
        if let Some(ms) = v["MediaSources"].as_array().and_then(|m| m.first()) {
            item.media = vec![to_media(ms, &self.login.base_url)];
        }
        item
    }
}

fn to_media(ms: &Value, base: &str) -> Media {
    let streams = ms["MediaStreams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["Type"] == "Video");
    let audio = streams.iter().find(|s| s["Type"] == "Audio" && s["IsDefault"].as_bool().unwrap_or(false)).or_else(|| streams.iter().find(|s| s["Type"] == "Audio"));
    let height = video.and_then(|v| v["Height"].as_u64());
    let res = height.map(|h| if h >= 1600 { "4k".to_string() } else if h >= 1000 { "1080".into() } else if h >= 700 { "720".into() } else { "sd".into() });
    let part_streams: Vec<Stream> = streams
        .iter()
        .filter_map(|s| {
            let stream_type = match s["Type"].as_str()? {
                "Video" => 1,
                "Audio" => 2,
                "Subtitle" => 3,
                _ => return None,
            };
            let external = s["IsExternal"].as_bool().unwrap_or(false);
            Some(Stream {
                id: s["Index"].as_u64().unwrap_or(0) + 1_000_000,
                stream_type,
                index: if external { None } else { s["Index"].as_u64().map(|i| i as u32) },
                codec: s["Codec"].as_str().map(String::from),
                language_code: s["Language"].as_str().map(String::from),
                extended_display_title: s["DisplayTitle"].as_str().map(String::from),
                display_title: s["DisplayTitle"].as_str().map(String::from),
                title: s["Title"].as_str().map(String::from),
                forced: s["IsForced"].as_bool().unwrap_or(false),
                selected: s["IsDefault"].as_bool().unwrap_or(false),
                key: if external { s["DeliveryUrl"].as_str().map(String::from) } else { None },
                color_trc: s["ColorTransfer"].as_str().map(String::from),
                dovi_present: s["VideoRangeType"].as_str().is_some_and(|r| r.starts_with("DOVI")),
                frame_rate: s["RealFrameRate"].as_f64().map(|f| f as f32),
            })
        })
        .collect();
    let _ = base;
    Media {
        id: 0,
        duration: ms["RunTimeTicks"].as_u64().map(|t| t / 10_000),
        bitrate: ms["Bitrate"].as_u64().map(|b| (b / 1000) as u32),
        width: video.and_then(|v| v["Width"].as_u64()).map(|w| w as u32),
        height: height.map(|h| h as u32),
        video_resolution: res,
        video_codec: video.and_then(|v| v["Codec"].as_str()).map(String::from),
        audio_codec: audio.and_then(|a| a["Codec"].as_str()).map(String::from),
        audio_channels: audio.and_then(|a| a["Channels"].as_u64()).map(|c| c as u32),
        container: ms["Container"].as_str().map(String::from),
        video_dynamic_range: video.and_then(|v| v["VideoRange"].as_str()).map(String::from),
        video_frame_rate: video.and_then(|v| v["RealFrameRate"].as_f64()).map(|f| format!("{}p", f.round())),
        parts: vec![Part {
            id: 0,
            // The media source id; the stream URL needs it.
            file: ms["Id"].as_str().map(String::from),
            key: ms["Path"].as_str().unwrap_or_default().to_string(),
            size: ms["Size"].as_u64(),
            container: ms["Container"].as_str().map(String::from),
            streams: part_streams,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server::new(
            Login {
                base_url: "http://jf:8096".into(),
                server_name: "JF".into(),
                flavor: Flavor::Jellyfin,
                user_id: "u1".into(),
                user_name: "me".into(),
                token: "tok".into(),
            },
            ClientInfo { device_id: "d".into(), device_name: "Mac".into(), version: "0.1".into() },
            "jf0:",
        )
    }

    #[test]
    fn urls() {
        assert_eq!(normalize_url("192.168.1.5:8096/"), "http://192.168.1.5:8096");
        assert_eq!(normalize_url("https://jf.example.com"), "https://jf.example.com");
    }

    #[test]
    fn converts_an_episode() {
        let v = serde_json::json!({
            "Id": "e1", "Type": "Episode", "Name": "Pilot", "IndexNumber": 1, "ParentIndexNumber": 2,
            "SeriesName": "Show", "SeriesId": "s1", "SeasonId": "se2", "SeriesPrimaryImageTag": "t9",
            "RunTimeTicks": 26_000_000_000u64, "ImageTags": {"Primary": "t1"},
            "UserData": {"PlaybackPositionTicks": 6_000_000_000u64, "Played": false},
            "Genres": ["Drama"], "CriticRating": 91, "CommunityRating": 8.4,
            "People": [{"Name": "Ann", "Type": "Actor", "Role": "Lead", "Id": "p1", "PrimaryImageTag": "pt"}],
            "MediaSources": [{"Id": "ms1", "Container": "mkv", "Size": 123, "Bitrate": 8_000_000,
                "MediaStreams": [
                    {"Type": "Video", "Index": 0, "Codec": "hevc", "Height": 2160, "Width": 3840, "VideoRange": "HDR", "ColorTransfer": "smpte2084"},
                    {"Type": "Audio", "Index": 1, "Codec": "eac3", "Channels": 6, "Language": "eng", "IsDefault": true, "DisplayTitle": "English - EAC3 - 5.1"},
                    {"Type": "Subtitle", "Index": 2, "Codec": "subrip", "IsExternal": true, "DeliveryUrl": "/Videos/e1/ms1/Subtitles/2/0/Stream.srt"}
                ]}]
        });
        let s = server();
        let i = s.to_item(&v);
        assert_eq!(i.rating_key, "jf0:e1");
        assert_eq!(i.kind, ItemKind::Episode);
        assert_eq!(i.episode_label().as_deref(), Some("S2 · E1"));
        assert_eq!(i.grandparent_rating_key.as_deref(), Some("jf0:s1"));
        assert_eq!(i.duration, Some(2_600_000));
        assert_eq!(i.view_offset, Some(600_000));
        assert_eq!(i.thumb.as_deref(), Some("http://jf:8096/Items/e1/Images/Primary?tag=t1&quality=90"));
        assert_eq!(i.grandparent_thumb.as_deref(), Some("http://jf:8096/Items/s1/Images/Primary?tag=t9&quality=90"));
        assert_eq!(i.cast[0].role.as_deref(), Some("Lead"));
        assert_eq!(i.ratings[0].value, 9.1);
        let m = &i.media[0];
        assert_eq!(m.video_resolution.as_deref(), Some("4k"));
        assert_eq!(i.primary_part().unwrap().dynamic_range(), Some("HDR10"));
        assert_eq!(m.audio_channels, Some(6));
        assert_eq!(s.stream_url(&i), "http://jf:8096/Videos/e1/stream?static=true&MediaSourceId=ms1&api_key=tok");
        let sub = i.primary_part().unwrap().streams.iter().find(|s| s.stream_type == 3).unwrap();
        assert_eq!(sub.index, None);
        assert_eq!(s.subtitle_url(sub).unwrap(), "http://jf:8096/Videos/e1/ms1/Subtitles/2/0/Stream.srt?api_key=tok");
    }
}
