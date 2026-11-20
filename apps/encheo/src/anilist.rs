//! Anime trailers from AniList (anilist.co, a free anime database; no account or key needed).
//! Plex and Jellyfin rarely have trailers for anime; AniList has YouTube trailers for most
//! series and films. Only used for titles that look like anime, and only when the server has no
//! trailer of its own; what's sent is the title.

use encheo_plex::{Item, ItemKind};
use serde_json::{Value, json};

/// Anime: the genre says so, or it's animation from Japan, or its original title is Japanese.
pub fn is_anime(item: &Item) -> bool {
    let has = |list: &[encheo_plex::Tag], word: &str| list.iter().any(|t| t.tag.eq_ignore_ascii_case(word));
    let japanese_title = item
        .original_title
        .as_deref()
        .is_some_and(|t| t.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}')));
    has(&item.genres, "anime")
        || (has(&item.genres, "animation") && (has(&item.countries, "japan") || japanese_title))
}

/// The trailer's web address (YouTube or Dailymotion) for the best match, if AniList has one.
pub async fn trailer(item: &Item) -> Option<String> {
    let mut titles = vec![item.title.clone()];
    if let Some(t) = item.original_title.clone().filter(|t| *t != item.title) {
        titles.push(t);
    }
    for title in titles {
        if let Some(url) = search(&title, item).await {
            return Some(url);
        }
    }
    None
}

async fn search(title: &str, item: &Item) -> Option<String> {
    let query = "query ($search: String) { Page(perPage: 8) { media(search: $search, type: ANIME) { \
                 format startDate { year } trailer { id site } } } }";
    let body = json!({ "query": query, "variables": { "search": title } }).to_string();
    // (Android has no system certificate store for a native app: use the bundled roots.)
    let client = encheo_plex::with_platform_roots(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let response = client
        .post("https://graphql.anilist.co")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", concat!("Encheo/", env!("CARGO_PKG_VERSION")))
        .body(body)
        .send()
        .await
        .ok()?;
    let v: Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
    let results = v["data"]["Page"]["media"].as_array()?;
    // AniList ranks by relevance; prefer the right kind (film or series) and year.
    let film = item.kind == ItemKind::Movie;
    let score = |m: &Value| {
        let format = m["format"].as_str().unwrap_or_default();
        let kind_ok = if film { format == "MOVIE" } else { matches!(format, "TV" | "TV_SHORT" | "ONA") };
        let year_ok = match (item.year, m["startDate"]["year"].as_i64()) {
            (Some(want), Some(got)) => (want as i64 - got).abs() <= 1,
            _ => false,
        };
        usize::from(kind_ok) + 2 * usize::from(year_ok)
    };
    let best = results
        .iter()
        .enumerate()
        .filter(|(_, m)| m["trailer"]["id"].is_string())
        .max_by_key(|(i, m)| (score(m), std::cmp::Reverse(*i)))?
        .1;
    // Without a year to go on, only the top result is trusted.
    if item.year.is_some() && score(best) < 2 && !std::ptr::eq(best, &results[0]) {
        return None;
    }
    let id = best["trailer"]["id"].as_str()?;
    match best["trailer"]["site"].as_str()? {
        "youtube" => Some(format!("https://www.youtube.com/watch?v={id}")),
        "dailymotion" => Some(format!("https://www.dailymotion.com/video/{id}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encheo_plex::Tag;

    fn tags(words: &[&str]) -> Vec<Tag> {
        words.iter().map(|w| Tag { tag: w.to_string(), thumb: None }).collect()
    }

    #[test]
    fn spots_anime() {
        let mut item = Item { genres: tags(&["Animation", "Action", "Anime"]), ..Default::default() };
        assert!(is_anime(&item));
        item.genres = tags(&["Animation", "Comedy"]);
        assert!(!is_anime(&item), "animation alone isn't anime (Pixar)");
        item.countries = tags(&["Japan"]);
        assert!(is_anime(&item));
        item.countries.clear();
        item.original_title = Some("転生したら剣でした".into());
        assert!(is_anime(&item));
        item.genres = tags(&["Drama"]);
        assert!(!is_anime(&item), "Japanese live action isn't anime");
    }
}
