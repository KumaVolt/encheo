//! One Plex Media Server at a chosen address.

use crate::model::{Directories, Envelope, Hubs, MetadataPage};
use crate::{Item, Library, Page, PlayState, Plex, Result};
use reqwest::Method;

const LIBRARY_ID: &str = "com.plexapp.plugins.library";

#[derive(Clone)]
pub struct Server {
    plex: Plex,
    name: String,
    base: String,
    token: String,
}

impl Server {
    pub(crate) fn new(plex: Plex, name: String, base: String, token: String) -> Self {
        Self { plex, name, base: base.trim_end_matches('/').to_string(), token }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The address in use, e.g. `https://192-168-1-10.<hash>.plex.direct:32400`.
    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    fn get(&self, path_and_query: &str) -> reqwest::RequestBuilder {
        self.plex.request(Method::GET, &format!("{}{}", self.base, path_and_query), Some(&self.token))
    }

    async fn page(&self, path: &str, start: u32, size: u32) -> Result<Page> {
        let r = self
            .get(path)
            .header("X-Plex-Container-Start", start.to_string())
            .header("X-Plex-Container-Size", size.to_string());
        let e: Envelope<MetadataPage> = Plex::json(r).await?;
        Ok(Page { total: e.container.total_size.or(Some(e.container.size)), items: e.container.metadata })
    }

    async fn list(&self, path: &str) -> Result<Vec<Item>> {
        let e: Envelope<MetadataPage> = Plex::json(self.get(path)).await?;
        Ok(e.container.metadata)
    }

    /// Movie, TV and music libraries. Photo libraries are left out.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let e: Envelope<Directories> = Plex::json(self.get("/library/sections")).await?;
        Ok(e.container.directory.into_iter().filter(|l| matches!(l.kind.as_str(), "movie" | "show" | "artist")).collect())
    }

    /// A page of a library, sorted by title.
    pub async fn library_items(&self, library_key: &str, start: u32, size: u32) -> Result<Page> {
        self.page(&format!("/library/sections/{library_key}/all?sort=titleSort"), start, size).await
    }

    pub async fn recently_added(&self, library_key: &str, size: u32) -> Result<Vec<Item>> {
        Ok(self.page(&format!("/library/sections/{library_key}/recentlyAdded"), 0, size).await?.items)
    }

    /// Partly watched movies and next episodes, across all libraries.
    pub async fn on_deck(&self) -> Result<Vec<Item>> {
        self.list("/library/onDeck").await
    }

    /// Full details of one item, including media versions and intro/credits markers.
    pub async fn metadata(&self, rating_key: &str) -> Result<Option<Item>> {
        Ok(self.list(&format!("/library/metadata/{rating_key}?includeMarkers=1")).await?.into_iter().next())
    }

    /// "More like this": titles Plex relates to this one.
    pub async fn related(&self, rating_key: &str) -> Result<Vec<Item>> {
        let e: Envelope<Hubs> = Plex::json(self.get(&format!("/hubs/metadata/{rating_key}/related?count=20"))).await?;
        let mut items = flatten_hubs(e.container);
        let mut seen = std::collections::HashSet::new();
        items.retain(|i| i.rating_key != rating_key && seen.insert(i.rating_key.clone()));
        Ok(items)
    }

    /// The server's copy of a title from its global `guid` (e.g. a watchlist entry), if it has it.
    pub async fn find_by_guid(&self, guid: &str) -> Result<Option<Item>> {
        let g: String = url::form_urlencoded::byte_serialize(guid.as_bytes()).collect();
        Ok(self.list(&format!("/library/all?guid={g}")).await?.into_iter().next())
    }

    /// Seasons of a show, episodes of a season, albums of an artist, or tracks of an album.
    pub async fn children(&self, rating_key: &str) -> Result<Vec<Item>> {
        self.list(&format!("/library/metadata/{rating_key}/children")).await
    }

    /// Every episode of a show, in order.
    pub async fn all_episodes(&self, show_rating_key: &str) -> Result<Vec<Item>> {
        self.list(&format!("/library/metadata/{show_rating_key}/allLeaves")).await
    }

    /// Search every library. Results come grouped by type (movies, shows, episodes, artists,
    /// albums, tracks); this flattens them in that order and drops other kinds (people, tags).
    pub async fn search(&self, query: &str) -> Result<Vec<Item>> {
        let q = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
        let e: Envelope<crate::model::Hubs> = Plex::json(self.get(&format!("/hubs/search?query={q}&limit=20"))).await?;
        Ok(flatten_hubs(e.container))
    }

    /// URL mpv can open to play the file as is (direct play, no transcoding).
    pub fn stream_url(&self, part: &crate::Part) -> String {
        format!("{}{}?X-Plex-Token={}", self.base, part.key, self.token)
    }

    /// URL of an external subtitle file Plex knows about (a `.srt` next to the video).
    pub fn subtitle_url(&self, stream: &crate::Stream) -> Option<String> {
        stream.key.as_ref().map(|k| format!("{}{k}?X-Plex-Token={}", self.base, self.token))
    }

    /// Poster or backdrop resized by the server, so we don't download 4K art for a 200 px tile.
    pub fn image_url(&self, path: &str, width: u32, height: u32) -> String {
        let p = url::form_urlencoded::byte_serialize(path.as_bytes()).collect::<String>();
        format!(
            "{}/photo/:/transcode?width={width}&height={height}&minSize=1&upscale=1&url={p}&X-Plex-Token={}",
            self.base, self.token
        )
    }

    /// Report where playback is. Call every ~10 s while playing and on pause/stop; this is what
    /// moves "Continue Watching" in every Plex app. Plex marks the item watched near the end.
    pub async fn report_progress(&self, item: &Item, state: PlayState, position_ms: u64) -> Result<()> {
        let duration = item.duration.unwrap_or(0);
        let path = format!(
            "/:/timeline?ratingKey={rk}&key=%2Flibrary%2Fmetadata%2F{rk}&state={s}&time={position_ms}&duration={duration}&identifier={LIBRARY_ID}",
            rk = item.rating_key,
            s = state.as_str(),
        );
        self.send_ok(&path).await
    }

    /// Remember the audio and/or subtitle track for a file, for every Plex app. `Some(0)` as the
    /// subtitle means "no subtitles".
    pub async fn set_streams(&self, part_id: u64, audio: Option<u64>, subtitle: Option<u64>) -> Result<()> {
        let mut q = String::from("allParts=1");
        if let Some(a) = audio {
            q.push_str(&format!("&audioStreamID={a}"));
        }
        if let Some(s) = subtitle {
            q.push_str(&format!("&subtitleStreamID={s}"));
        }
        let url = format!("{}/library/parts/{part_id}?{q}", self.base);
        let resp = self.plex.request(Method::PUT, &url, Some(&self.token)).send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::Status { status: resp.status().as_u16(), url: crate::strip_token(resp.url().as_str()) });
        }
        Ok(())
    }

    /// Per-item server settings, e.g. a show's preferred audio/subtitle language.
    pub async fn preferences(&self, rating_key: &str) -> Result<Vec<crate::Setting>> {
        let e: Envelope<crate::model::WithPreferences> =
            Plex::json(self.get(&format!("/library/metadata/{rating_key}?includePreferences=1"))).await?;
        Ok(e.container.metadata.into_iter().next().and_then(|m| m.preferences).map(|p| p.settings).unwrap_or_default())
    }

    /// Change per-item settings. Needs a server admin; shared users get a 401/403.
    pub async fn set_preferences(&self, rating_key: &str, values: &[(&str, &str)]) -> Result<()> {
        let q: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(values).finish();
        let url = format!("{}/library/metadata/{rating_key}/prefs?{q}", self.base);
        let resp = self.plex.request(Method::PUT, &url, Some(&self.token)).send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::Status { status: resp.status().as_u16(), url: crate::strip_token(resp.url().as_str()) });
        }
        Ok(())
    }

    /// Search online subtitles (Plex's OpenSubtitles integration) in a language (ISO 639-1, e.g.
    /// "en"). Best matches first.
    pub async fn search_subtitles(&self, rating_key: &str, language: &str) -> Result<Vec<crate::SubtitleResult>> {
        let lang: String = url::form_urlencoded::byte_serialize(language.as_bytes()).collect();
        let r = self.get(&format!("/library/metadata/{rating_key}/subtitles?language={lang}&hearingImpaired=0&forced=0")).timeout(std::time::Duration::from_secs(60));
        let e: Envelope<crate::model::SubtitleResults> = Plex::json(r).await?;
        let mut v = e.container.streams;
        v.sort_by_key(|s| std::cmp::Reverse(s.score.parse::<i64>().unwrap_or(0)));
        Ok(v)
    }

    /// Download a found subtitle to the server and make it the item's subtitle, for every Plex app.
    pub async fn pick_subtitle(&self, rating_key: &str, key: &str) -> Result<()> {
        let k: String = url::form_urlencoded::byte_serialize(key.as_bytes()).collect();
        let url = format!("{}/library/metadata/{rating_key}/subtitles?key={k}&hearingImpaired=0&forced=0", self.base);
        let resp = self.plex.request(Method::PUT, &url, Some(&self.token)).send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::Status { status: resp.status().as_u16(), url: crate::strip_token(resp.url().as_str()) });
        }
        Ok(())
    }

    /// URL of a found subtitle's text, to show it right away.
    pub fn subtitle_result_url(&self, key: &str) -> String {
        format!("{}{key}?X-Plex-Token={}", self.base, self.token)
    }

    pub async fn mark_watched(&self, rating_key: &str) -> Result<()> {
        self.send_ok(&format!("/:/scrobble?key={rating_key}&identifier={LIBRARY_ID}")).await
    }

    pub async fn mark_unwatched(&self, rating_key: &str) -> Result<()> {
        self.send_ok(&format!("/:/unscrobble?key={rating_key}&identifier={LIBRARY_ID}")).await
    }

    async fn send_ok(&self, path: &str) -> Result<()> {
        let resp = self.get(path).send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::Status { status: resp.status().as_u16(), url: crate::strip_token(resp.url().as_str()) });
        }
        Ok(())
    }
}

pub(crate) fn flatten_hubs(hubs: Hubs) -> Vec<Item> {
    use crate::ItemKind::*;
    hubs.hub
        .into_iter()
        .flat_map(|h| h.metadata.into_iter().chain(h.directory))
        .filter_map(|v| serde_json::from_value::<Item>(v).ok())
        .filter(|i| matches!(i.kind, Movie | Show | Episode | Artist | Album | Track))
        .collect()
}
