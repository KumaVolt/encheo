//! Plex's catalogue of films and shows (discover.provider.plex.tv): titles whether or not any of
//! your servers has them. Search finds what you don't own yet, a title's page can show it with
//! its trailer and related titles, and the watchlist is kept there.
//!
//! Needs a Plex account token; nothing here touches a server.

use crate::model::{Envelope, Hubs};
use crate::{Credits, Item, Person, Plex, Result, Row};
use reqwest::Method;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
struct SearchContainer {
    #[serde(default, rename = "SearchResults")]
    results: Vec<SearchGroup>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchGroup {
    #[serde(default, rename = "SearchResult")]
    results: Vec<SearchResult>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchResult {
    #[serde(default, rename = "Metadata")]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct ItemsContainer {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonMetadata {
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    born_at: Option<String>,
    #[serde(default)]
    died_at: Option<String>,
    #[serde(default)]
    birth_place: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PersonContainer {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<PersonMetadata>,
}

#[derive(Debug, Default, Deserialize)]
struct CreditsContainer {
    #[serde(default, rename = "CreditGroup")]
    groups: Vec<CreditGroup>,
}

#[derive(Debug, Default, Deserialize)]
struct CreditGroup {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "Credit")]
    credits: Vec<Credit>,
}

#[derive(Debug, Default, Deserialize)]
struct Credit {
    #[serde(default, rename = "Metadata")]
    metadata: Option<serde_json::Value>,
}

/// Films and shows only; one odd entry is skipped rather than failing the list.
fn items(values: impl IntoIterator<Item = serde_json::Value>) -> Vec<Item> {
    values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Item>(v).ok())
        .filter(|i| matches!(i.kind, crate::ItemKind::Movie | crate::ItemKind::Show))
        .collect()
}

/// Catalogue keys are hex; anything else isn't one of ours.
fn checked(rating_key: &str) -> Option<&str> {
    (!rating_key.is_empty() && rating_key.chars().all(|c| c.is_ascii_alphanumeric())).then_some(rating_key)
}

impl Plex {
    fn discover_get(&self, token: &str, path: &str) -> reqwest::RequestBuilder {
        self.request(Method::GET, &format!("{}{path}", self.discover), Some(token))
    }

    /// Films and shows in the catalogue matching `query`, best match first.
    pub async fn catalog_search(&self, token: &str, query: &str, limit: u32) -> Result<Vec<Item>> {
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let path = format!("/library/search?query={q}&limit={limit}&searchTypes=movies,tv&searchProviders=discover&includeMetadata=1");
        let e: Envelope<SearchContainer> = Self::json(self.discover_get(token, &path)).await?;
        Ok(items(e.container.results.into_iter().flat_map(|g| g.results).filter_map(|r| r.metadata)))
    }

    /// A catalogue title in full: summary, genres, cast, ratings.
    pub async fn catalog_item(&self, token: &str, rating_key: &str) -> Result<Option<Item>> {
        let Some(key) = checked(rating_key) else { return Ok(None) };
        let e: Envelope<ItemsContainer> = Self::json(self.discover_get(token, &format!("/library/metadata/{key}"))).await?;
        Ok(items(e.container.metadata).into_iter().next())
    }

    /// A person (`id` from [`crate::Role::person_id`]): who they are, and their films and shows
    /// by kind of credit, newest first. Credits carry no guid; they get the one Plex gives the
    /// same title on a server (`plex://movie/<key>`), so your own copies can be found.
    pub async fn catalog_person(&self, token: &str, id: &str) -> Result<Option<Person>> {
        let Some(id) = checked(id) else { return Ok(None) };
        let e: Envelope<PersonContainer> = Self::json(self.discover_get(token, &format!("/library/people/{id}"))).await?;
        let Some(p) = e.container.metadata.into_iter().next() else { return Ok(None) };
        let c: Envelope<CreditsContainer> = Self::json(self.discover_get(token, &format!("/library/people/{id}/credits"))).await?;
        let credits = c
            .container
            .groups
            .into_iter()
            .map(|g| {
                let mut seen = std::collections::HashSet::new();
                let mut items: Vec<Item> = items(g.credits.into_iter().filter_map(|c| c.metadata))
                    .into_iter()
                    .filter(|i| seen.insert(i.rating_key.clone()))
                    .map(|mut i| {
                        if i.guid.is_none() {
                            let kind = if i.kind == crate::ItemKind::Show { "show" } else { "movie" };
                            i.guid = Some(format!("plex://{kind}/{}", i.rating_key));
                        }
                        i
                    })
                    .collect();
                // Newest first; titles without a year (not out yet) at the top.
                items.sort_by_key(|i| std::cmp::Reverse(i.year.unwrap_or(u32::MAX)));
                Credits { title: g.title, items }
            })
            .filter(|g| !g.items.is_empty())
            .collect();
        Ok(Some(Person {
            id: id.to_string(),
            name: p.title,
            thumb: p.thumb,
            summary: p.summary.filter(|s| !s.trim().is_empty()),
            born: p.born_at,
            died: p.died_at,
            birth_place: p.birth_place,
            credits,
        }))
    }

    /// Titles related to a catalogue title: its collection, then more from its cast and crew.
    /// (Plex's own "You might like" picks are left out: they advertise its free streaming
    /// catalogue rather than anything like the title.)
    pub async fn catalog_related(&self, token: &str, rating_key: &str) -> Result<Vec<Row>> {
        let Some(key) = checked(rating_key) else { return Ok(vec![]) };
        let e: Envelope<Hubs> = Self::json(self.discover_get(token, &format!("/hubs/metadata/{key}/related"))).await?;
        let mut rows = Vec::new();
        for hub in e.container.hub {
            let wanted = hub.title.contains("Collection") || hub.title.contains("Cast & Crew");
            let Some(path) = hub.key.filter(|k| wanted && k.starts_with('/')) else { continue };
            let list = if hub.metadata.is_empty() {
                let e: Envelope<ItemsContainer> = match Self::json(self.discover_get(token, &path)).await {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                items(e.container.metadata)
            } else {
                items(hub.metadata)
            };
            if !list.is_empty() {
                rows.push(Row { title: hub.title, items: list, more: None });
            }
        }
        // The collection first: the other parts of a series are the closest relatives.
        rows.sort_by_key(|r| !r.title.contains("Collection"));
        Ok(rows)
    }

    /// A catalogue title's trailer, ready to play: its file's address is made absolute (an HLS
    /// playlist on the catalogue, with the account token), so the player opens it as it is.
    pub async fn catalog_trailer(&self, token: &str, rating_key: &str) -> Result<Option<Item>> {
        let Some(key) = checked(rating_key) else { return Ok(None) };
        let e: Envelope<Hubs> = Self::json(self.discover_get(token, &format!("/hubs/metadata/{key}/related"))).await?;
        let Some(path) = e.container.hub.into_iter().find(|h| h.title == "Extras").and_then(|h| h.key) else { return Ok(None) };
        if !path.starts_with('/') {
            return Ok(None);
        }
        let e: Envelope<ItemsContainer> = Self::json(self.discover_get(token, &path)).await?;
        let Some(mut trailer) = items_any(e.container.metadata)
            .into_iter()
            .find(|i| (i.subtype.as_deref() == Some("trailer") || i.extra_type == "1") && i.primary_part().is_some_and(|p| p.key.starts_with('/')))
        else {
            return Ok(None);
        };
        let part = &mut trailer.media[0].parts[0];
        let sep = if part.key.contains('?') { '&' } else { '?' };
        part.key = format!("{}{}{sep}X-Plex-Token={token}", self.discover, part.key);
        trailer.rating_key = format!("cat:{key}:trailer");
        Ok(Some(trailer))
    }

    /// Whether the title is on the account's watchlist.
    pub async fn on_watchlist(&self, token: &str, rating_key: &str) -> Result<bool> {
        let Some(key) = checked(rating_key) else { return Ok(false) };
        let v: serde_json::Value = Self::json(self.discover_get(token, &format!("/library/metadata/{key}/userState"))).await?;
        let state = &v["MediaContainer"]["UserState"];
        let state = if state.is_array() { &state[0] } else { state };
        Ok(state.get("watchlistedAt").is_some_and(|w| !w.is_null()))
    }

    /// Add a catalogue title to the account's watchlist, or take it off.
    pub async fn set_watchlisted(&self, token: &str, rating_key: &str, on: bool) -> Result<()> {
        let Some(key) = checked(rating_key) else { return Ok(()) };
        let action = if on { "addToWatchlist" } else { "removeFromWatchlist" };
        let url = format!("{}/actions/{action}?ratingKey={key}", self.discover);
        let resp = self.request(Method::PUT, &url, Some(token)).send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::Status { status: resp.status().as_u16(), url: crate::strip_token(resp.url().as_str()) });
        }
        Ok(())
    }
}

/// Extras keep their own kinds (clips); parse them all. Their stream lists are dropped: the
/// catalogue's stream ids are text where a server's are numbers, and a trailer doesn't need them.
fn items_any(values: Vec<serde_json::Value>) -> Vec<Item> {
    values
        .into_iter()
        .filter_map(|mut v| {
            for media in v.get_mut("Media").and_then(|m| m.as_array_mut()).into_iter().flatten() {
                for part in media.get_mut("Part").and_then(|p| p.as_array_mut()).into_iter().flatten() {
                    if let Some(part) = part.as_object_mut() {
                        part.remove("Stream");
                    }
                }
            }
            serde_json::from_value::<Item>(v).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_keep_films_and_shows() {
        let body = serde_json::json!({"MediaContainer": {"SearchResults": [
            {"SearchResult": [
                {"Metadata": {"ratingKey": "5eb8", "guid": "plex://movie/5eb8", "type": "movie", "title": "Dune: Part Two", "year": 2024}},
                {"Metadata": {"ratingKey": "p1", "type": "person", "title": "Zendaya"}},
                {"score": 0.5}
            ]},
            {"SearchResult": [{"Metadata": {"ratingKey": "5d9c", "type": "show", "title": "Dune: Prophecy", "year": 2024}}]}
        ]}});
        let e: Envelope<SearchContainer> = serde_json::from_value(body).unwrap();
        let found = items(e.container.results.into_iter().flat_map(|g| g.results).filter_map(|r| r.metadata));
        let titles: Vec<&str> = found.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Dune: Part Two", "Dune: Prophecy"]);
        assert_eq!(found[0].guid.as_deref(), Some("plex://movie/5eb8"));
    }

    #[test]
    fn catalogue_trailers_parse() {
        let clip = serde_json::json!({
            "ratingKey": "6453", "type": "clip", "subtype": "trailer", "extraType": 1, "title": "Dune: Part Two", "userState": false,
            "Media": [{"protocol": "hls", "Part": [{"key": "/library/metadata/5eb8/extras/6453/parts/hls.m3u8",
                "Stream": [{"id": "6453-6afe", "streamType": 1, "codec": "h264"}]}]}]
        });
        let found = items_any(vec![clip]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subtype.as_deref(), Some("trailer"));
        assert!(found[0].primary_part().is_some_and(|p| p.key.ends_with("hls.m3u8")));
    }

    #[test]
    fn only_catalogue_keys_go_into_paths() {
        assert_eq!(checked("5eb8f22237ccbe0041723253"), Some("5eb8f22237ccbe0041723253"));
        assert_eq!(checked("../library"), None);
        assert_eq!(checked("a?b=c"), None);
        assert_eq!(checked(""), None);
    }
}

