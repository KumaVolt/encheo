//! Search suggestions, for when the library doesn't have what was typed:
//!
//! - "Did you mean": titles in your libraries that are close to what was typed, typos and all
//!   ("interstelar" → Interstellar), matched here against every movie and show title.
//! - "Not in your library": the title in Plex's catalogue (with a Plex account), to open, watch
//!   the trailer of, or put on the watchlist.
//! - "In your library instead": what you do have that's like it: its collection, its cast and
//!   crew, then its genres around the same years.

use super::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Every movie and show in the libraries, for matching. Built on the first search that needs it
/// and kept a while; a few thousand titles are a few megabytes.
static INDEX: Mutex<Option<(Instant, Arc<Vec<Item>>)>> = Mutex::new(None);
const INDEX_AGE: Duration = Duration::from_secs(15 * 60);

/// The rows under the search results.
#[derive(Default)]
pub(crate) struct Suggestions {
    /// Close to what was typed, from the libraries; and the title to name in "Did you mean".
    pub did_you_mean: Vec<Item>,
    pub did_you_mean_title: Option<String>,
    /// From the catalogue, not on any server.
    pub not_owned: Vec<Item>,
    /// From the libraries, like the first title that isn't owned; and that title.
    pub instead: Vec<Item>,
    pub instead_of: Option<String>,
}

/// Which libraries the index was built from (server and library keys).
static LIBRARIES: Mutex<String> = Mutex::new(String::new());

/// Home loaded its libraries: a different set (signed in or out, another profile or server)
/// means the index is out of date.
pub(crate) fn libraries_loaded(server: &str, keys: &[String]) {
    let now = format!("{server}|{}", keys.join(","));
    let Ok(mut was) = LIBRARIES.lock() else { return };
    if *was != now {
        *was = now;
        if let Ok(mut i) = INDEX.lock() {
            *i = None;
        }
    }
}

/// Look for suggestions after a search's own results are in; they show as rows under them,
/// unless the search changed meanwhile.
pub(crate) fn start(query: String, found: Vec<Item>, generation: u64) {
    let account = with_state(|s| s.session.token.clone().map(|t| (plex(&s.session), t)));
    // Without Plex's catalogue, Seerr's search says what isn't in the library.
    let seerr = if account.is_none() { requests::connection() } else { None };
    let libraries = libraries();
    rt().spawn(async move {
        let found = suggest(&query, &found, account, seerr, libraries).await;
        on_ui(move |ui| {
            if with_state(|s| s.search_generation) == generation {
                show(ui, found);
            }
        });
    });
}

/// No rows (a new search, or an empty one).
pub(crate) fn clear(ui: &AppWindow) {
    ui.set_search_rows(ModelRc::default());
}

fn show(ui: &AppWindow, found: Suggestions) {
    let row = |title: String, items: &[Item]| HomeRow {
        title: title.into(),
        library: -1,
        landscape: false,
        square: false,
        items: ModelRc::new(VecModel::from(items.iter().map(|i| poster(i, false)).collect::<Vec<_>>())),
    };
    let mut rows = Vec::new();
    if let Some(title) = found.did_you_mean_title.filter(|_| !found.did_you_mean.is_empty()) {
        rows.push(row(format!("Did you mean “{title}”?"), &found.did_you_mean));
        remember(found.did_you_mean);
    }
    if !found.not_owned.is_empty() {
        // Catalogue titles open their own page (see `open_catalog`).
        let items: Vec<Item> = found.not_owned.into_iter().map(catalog_id).collect();
        rows.push(row("Not in your library".into(), &items));
        remember(items);
    }
    if let Some(like) = found.instead_of.filter(|_| !found.instead.is_empty()) {
        rows.push(row(format!("In your library, like “{like}”"), &found.instead));
        remember(found.instead);
    }
    ui.set_search_rows(ModelRc::new(VecModel::from(rows)));
}

/// The Plex account to ask the catalogue with (the profile's own, for its watchlist).
pub(crate) fn account() -> Option<(encheo_plex::Plex, String)> {
    with_state(|s| s.session.token.clone().map(|t| (plex(&s.session), t)))
}

/// A catalogue title's page has no Play: you don't have it. Its heart is the watchlist, and a
/// badge says it isn't yours.
fn catalog_buttons(ui: &AppWindow) {
    ui.set_play_label(SharedString::new());
    ui.set_detail_can_restart(false);
    ui.set_detail_can_mark_watched(false);
    ui.set_detail_can_shuffle(false);
    ui.set_detail_can_favourite(true);
    ui.set_detail_watchlist_mode(true);
    let mut badges: Vec<SharedString> = ui.get_detail_badges().iter().collect();
    if !badges.iter().any(|b| b == "Not in your library") {
        badges.insert(0, "Not in your library".into());
    }
    ui.set_detail_badges(ModelRc::new(VecModel::from(badges)));
}

/// The detail page for a catalogue title (`cat:` id): the rest comes from the catalogue, in the
/// background: full details and cast, whether it's on the watchlist, its trailer, and titles
/// related to it.
pub(crate) fn catalog_detail(ui: &AppWindow, key: String) {
    catalog_buttons(ui);
    ui.set_detail_favourite(false);
    let Some((plex, token)) = account() else { return };
    let generation = with_state(|s| s.generation);
    let libraries = libraries();
    rt().spawn(async move {
        let index = index(libraries).await;
        let full = plex.catalog_item(&token, &key).await.ok().flatten();
        let on = plex.on_watchlist(&token, &key).await.unwrap_or(false);
        let trailer = plex.catalog_trailer(&token, &key).await.ok().flatten();
        let related = plex.catalog_related(&token, &key).await.unwrap_or_default();
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            if let Some(full) = full.map(catalog_id) {
                fill_detail_text(ui, &full);
                with_models(|m| m.people.set_vec(people(&full)));
                remember([full.clone()]);
                with_state(|s| s.detail = Some(full.clone()));
                requests::catalogue_page(&full);
            }
            catalog_buttons(ui);
            ui.set_detail_favourite(on);
            if let Some(t) = trailer {
                with_state(|s| s.trailer = Some(backend::Trailer::Play(Box::new(t))));
                ui.set_detail_can_trailer(true);
            }
            let mut seen = HashSet::new();
            // Titles you have open your copy; the rest their catalogue page.
            let items: Vec<Item> = related
                .into_iter()
                .flat_map(|r| r.items)
                .map(|c| own_copy(&index, &c).cloned().unwrap_or_else(|| catalog_id(c)))
                .filter(|i| i.rating_key != format!("cat:{key}") && seen.insert(i.rating_key.clone()))
                .take(30)
                .collect();
            let posters: Vec<Poster> = items.iter().map(|i| poster(i, false)).collect();
            remember(items);
            with_models(|m| m.related.set_vec(posters));
            detail_rows(ui);
        });
    });
}

/// The watchlist button on a catalogue title's page.
pub(crate) fn toggle_watchlist() {
    let ui = ui();
    let Some(item) = with_state(|s| s.detail.clone()) else { return };
    let Some(key) = item.rating_key.strip_prefix("cat:").map(String::from) else { return };
    let Some((plex, token)) = account() else { return };
    let on = !ui.get_detail_favourite();
    rt().spawn(async move {
        let result = plex.set_watchlisted(&token, &key, on).await;
        on_ui(move |ui| match result {
            Ok(()) => {
                if with_state(|s| s.detail.as_ref().is_some_and(|d| d.rating_key == format!("cat:{key}"))) {
                    ui.set_detail_favourite(on);
                }
                toast(ui, if on { format!("Added “{}” to your Watchlist", item.title) } else { format!("Removed “{}” from your Watchlist", item.title) });
            }
            Err(e) => toast(ui, format!("Couldn't change your Watchlist: {e}")),
        });
    });
}

/// A catalogue title, under the id Encheo knows it by.
pub(crate) fn catalog_id(mut item: Item) -> Item {
    if !item.rating_key.starts_with("cat:") && !item.rating_key.starts_with("seerr:") {
        item.rating_key = format!("cat:{}", item.rating_key);
    }
    item
}

/// A movie or TV library to index: its key, kind and server.
pub(crate) type Library = (String, String, backend::Backend);

/// The movie and TV libraries, with their servers (on the UI thread: state lives there).
pub(crate) fn libraries() -> Vec<Library> {
    let keys: Vec<(String, String)> = with_state(|s| {
        s.library_keys
            .iter()
            .zip(&s.library_kinds)
            .filter(|(_, k)| k.as_str() == "movie" || k.as_str() == "show")
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect()
    });
    keys.into_iter().filter_map(|(key, kind)| backend_for(&key).map(|b| (key, kind, b))).collect()
}

pub(crate) async fn index(libraries: Vec<Library>) -> Arc<Vec<Item>> {
    if let Ok(i) = INDEX.lock()
        && let Some((at, list)) = i.as_ref()
        && at.elapsed() < INDEX_AGE
    {
        return list.clone();
    }
    let mut all = Vec::new();
    for (key, kind, server) in libraries {
        let mut start = 0;
        loop {
            let Ok(page) = server.library_items(&key, &kind, &Default::default(), start, 500).await else { break };
            let n = page.items.len() as u32;
            all.extend(page.items);
            if n < 500 || page.total.is_some_and(|t| start + 500 >= t) {
                break;
            }
            start += 500;
        }
    }
    let list = Arc::new(all);
    if let Ok(mut i) = INDEX.lock() {
        *i = Some((Instant::now(), list.clone()));
    }
    list
}

/// Your copy of a catalogue title: the same Plex guid, else the same title, year and kind.
pub(crate) fn own_copy<'a>(index: &'a [Item], c: &Item) -> Option<&'a Item> {
    index
        .iter()
        .find(|i| c.guid.is_some() && i.guid == c.guid)
        .or_else(|| c.external_id("tmdb").and_then(|t| index.iter().find(|i| i.kind == c.kind && i.external_id("tmdb") == Some(t))))
        .or_else(|| index.iter().find(|i| same_title(&i.title, &c.title) && i.year == c.year && i.kind == c.kind))
}

/// Suggestions for `query`. `found` is what the servers' own search returned.
pub(crate) async fn suggest(
    query: &str,
    found: &[Item],
    plex: Option<(encheo_plex::Plex, String)>,
    seerr: Option<encheo_seerr::Seerr>,
    libraries: Vec<Library>,
) -> Suggestions {
    let mut out = Suggestions::default();
    let query = query.trim();
    if normalize(query).chars().count() < 3 {
        return out;
    }
    let index = index(libraries).await;
    let found_ids: HashSet<&str> = found.iter().map(|i| i.rating_key.as_str()).collect();
    let mut shown: HashSet<String> = found_ids.iter().map(|s| s.to_string()).collect();

    // Typos: only when the servers found nothing that looks like it.
    let found_nothing = found.iter().all(|i| i.kind != ItemKind::Movie && i.kind != ItemKind::Show);
    if found_nothing {
        let titles: Vec<&str> = index.iter().map(|i| i.title.as_str()).collect();
        for at in close_titles(query, &titles, 12) {
            let item = &index[at];
            if shown.insert(item.rating_key.clone()) {
                out.did_you_mean.push(item.clone());
            }
        }
        out.did_you_mean_title = out.did_you_mean.first().map(|i| i.title.clone());
    }

    let Some((plex, token)) = plex else {
        if let Some(seerr) = seerr {
            for c in requests::search(&seerr, query).await {
                if own_copy(&index, &c).is_none() && shown.insert(c.rating_key.clone()) {
                    out.not_owned.push(c);
                }
            }
        }
        return out;
    };
    let Ok(catalogue) = plex.catalog_search(&token, query, 12).await else { return out };
    // When its best match is one you have, the catalogue's other hits are loose guesses: only
    // the ones close to what was typed are worth showing.
    let best_owned = catalogue.first().is_some_and(|c| own_copy(&index, c).is_some());
    for (at, c) in catalogue.iter().enumerate() {
        if let Some(mine) = own_copy(&index, c) {
            // The catalogue understood what was meant and you have it: suggest your copy, if the
            // search found nothing and this is its best guess or close to what was typed (it
            // also returns loose matches).
            if !found_nothing || (at > 0 && closeness(&normalize(query), &normalize(&c.title)) > 0.45) {
                continue;
            }
            if shown.insert(mine.rating_key.clone()) {
                out.did_you_mean.push(mine.clone());
                out.did_you_mean_title.get_or_insert_with(|| mine.title.clone());
            }
        } else if !best_owned || closeness(&normalize(query), &normalize(&c.title)) <= 0.35 {
            out.not_owned.push(c.clone());
        }
    }

    // Like the best match you don't have, from what you do have.
    let Some(top) = catalogue.first().filter(|c| own_copy(&index, c).is_none()) else { return out };
    let full = plex.catalog_item(&token, &top.rating_key).await.ok().flatten().unwrap_or_else(|| top.clone());
    let related = plex.catalog_related(&token, &top.rating_key).await.unwrap_or_default();
    let mut weight: HashMap<String, u32> = HashMap::new();
    for row in &related {
        let w = if row.title.contains("Collection") { 100 } else { 40 };
        for r in &row.items {
            if let Some(g) = &r.guid {
                *weight.entry(g.clone()).or_default() += w;
            }
        }
    }
    let ranked = rank_like(&full, &index, &weight, 12, &shown);
    out.instead = ranked.into_iter().map(|i| index[i].clone()).collect();
    if !out.instead.is_empty() {
        out.instead_of = Some(full.title.clone());
    }
    out
}

/// Library titles most like `target`: related titles (by guid weight), then shared genres,
/// the same kind, a close year, and how well rated they are. Unrelated titles are left out.
fn rank_like(target: &Item, index: &[Item], weight: &HashMap<String, u32>, n: usize, skip: &HashSet<String>) -> Vec<usize> {
    let genres: HashSet<String> = target.genres.iter().map(|g| normalize(&g.tag)).collect();
    let mut scored: Vec<(i64, usize)> = index
        .iter()
        .enumerate()
        .filter(|(_, i)| !skip.contains(&i.rating_key))
        .filter_map(|(at, i)| {
            let related = i.guid.as_ref().and_then(|g| weight.get(g)).copied().unwrap_or(0) as i64;
            let shared = i.genres.iter().filter(|g| genres.contains(&normalize(&g.tag))).count() as i64;
            if related == 0 && shared == 0 {
                return None;
            }
            let mut score = related * 10 + shared * 30;
            if i.kind == target.kind {
                score += 15;
            }
            if let (Some(a), Some(b)) = (i.year, target.year) {
                score += 15 - (a.abs_diff(b) as i64).min(15);
            }
            score += (i.audience_rating.unwrap_or(0.0) * 2.0) as i64;
            Some((score, at))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(n).map(|(_, at)| at).collect()
}

fn same_title(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

/// Lowercase, accents folded, punctuation as spaces, no leading "the", "a" or "an".
pub(crate) fn normalize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase).map(fold) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
    }
    let out = out.trim_end();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = out.strip_prefix(article) {
            return rest.to_string();
        }
    }
    out.to_string()
}

/// A lower-case letter without its accent (é → e), for matching and sorting.
pub(crate) fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ł' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'ř' => 'r',
        'ś' | 'š' => 's',
        'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

/// Edits between two strings: insertions, deletions, substitutions and swapped neighbours.
fn distance(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

/// Edits between `a` and `b` for their length, with a little extra when the first letters
/// differ: people seldom mistype the first one ("bodi" is The Body before Zodiac).
fn ratio(a: &[char], b: &[char]) -> f32 {
    let r = distance(a, b) as f32 / a.len().max(b.len()).max(1) as f32;
    if a.first() == b.first() { r } else { r + 0.1 }
}

/// How far `title` is from what was typed, 0 (same) to 1 (nothing alike): the whole title, its
/// beginning (still typing), or word by word, where every typed word must be close to one of
/// the title's (words in another order, a few words of a long title).
fn closeness(query: &str, title: &str) -> f32 {
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = title.chars().collect();
    if q.is_empty() || t.is_empty() {
        return 1.0;
    }
    let whole = ratio(&q, &t);
    // The title's start, a letter shorter or longer than what was typed.
    let start = (q.len().saturating_sub(1)..=q.len() + 1)
        .filter(|&k| k > 0 && k <= t.len())
        .map(|k| ratio(&q, &t[..k]))
        .fold(1.0f32, f32::min);
    let words: Vec<Vec<char>> = title.split(' ').map(|w| w.chars().collect()).collect();
    let typed: Vec<Vec<char>> = query.split(' ').filter(|w| w.chars().count() >= 3).map(|w| w.chars().collect()).collect();
    let by_word = if typed.is_empty() {
        1.0
    } else {
        typed.iter().map(|w| words.iter().map(|tw| ratio(w, tw)).fold(1.0f32, f32::min)).fold(0.0f32, f32::max)
    };
    whole.min(start).min(by_word)
}

/// Titles close to `query`, best first, at most `n`.
pub(crate) fn close_titles(query: &str, titles: &[&str], n: usize) -> Vec<usize> {
    let q = normalize(query);
    if q.chars().count() < 3 {
        return vec![];
    }
    // Longer words allow more mistakes: about one in four letters.
    let limit = if q.chars().count() <= 4 { 0.25 } else { 0.3 };
    let mut hits: Vec<(f32, usize, usize)> = titles
        .iter()
        .enumerate()
        .filter_map(|(at, t)| {
            let t = normalize(t);
            let c = closeness(&q, &t);
            (c <= limit).then_some((c, t.len(), at))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut seen = HashSet::new();
    hits.into_iter().filter(|h| seen.insert(normalize(titles[h.2]))).take(n).map(|h| h.2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLES: [&str; 9] = [
        "Interstellar",
        "The Matrix",
        "Dune",
        "Dunkirk",
        "Amélie",
        "Spirited Away",
        "The Lord of the Rings: The Fellowship of the Ring",
        "Inception",
        "Interstate 60",
    ];

    fn best(q: &str) -> Vec<&'static str> {
        close_titles(q, &TITLES, 3).into_iter().map(|i| TITLES[i]).collect()
    }

    #[test]
    fn typos_find_the_title() {
        assert_eq!(best("interstelar")[0], "Interstellar");
        assert_eq!(best("intersteller")[0], "Interstellar");
        assert_eq!(best("matirx")[0], "The Matrix");
        assert_eq!(best("the matrx")[0], "The Matrix");
        assert_eq!(best("amelie")[0], "Amélie");
        assert_eq!(best("spirted away")[0], "Spirited Away");
        assert_eq!(best("inceptoin")[0], "Inception");
    }

    #[test]
    fn a_word_of_a_long_title_is_enough() {
        assert_eq!(best("fellowship")[0], "The Lord of the Rings: The Fellowship of the Ring");
        assert_eq!(best("felowship")[0], "The Lord of the Rings: The Fellowship of the Ring");
    }

    #[test]
    fn unrelated_words_find_nothing() {
        assert!(best("zombie").is_empty());
        assert!(best("korean drama").is_empty());
        // Too short to guess from.
        assert!(best("du").is_empty());
    }

    #[test]
    fn normalize_drops_articles_and_marks() {
        assert_eq!(normalize("The Lord of the Rings: The Fellowship"), "lord of the rings the fellowship");
        assert_eq!(normalize("Amélie!"), "amelie");
        assert_eq!(normalize("A Quiet Place"), "quiet place");
    }

    #[test]
    fn related_and_genres_rank_first() {
        let g = |t: &str| encheo_plex::Tag { tag: t.into(), ..Default::default() };
        let item = |key: &str, title: &str, guid: Option<&str>, genres: &[&str], year: u32| Item {
            rating_key: key.into(),
            title: title.into(),
            guid: guid.map(String::from),
            genres: genres.iter().map(|x| g(x)).collect(),
            year: Some(year),
            kind: ItemKind::Movie,
            ..Default::default()
        };
        let target = item("t", "Dune: Part Two", None, &["Science Fiction", "Adventure"], 2024);
        let index = vec![
            item("1", "Notting Hill", None, &["Romance", "Comedy"], 1999),
            item("2", "Dune", Some("plex://movie/dune"), &["Science Fiction"], 2021),
            item("3", "Arrival", None, &["Science Fiction", "Drama"], 2016),
            item("4", "Blade Runner 2049", None, &["Science Fiction", "Adventure"], 2017),
        ];
        let weight = HashMap::from([("plex://movie/dune".to_string(), 100)]);
        let ranked = rank_like(&target, &index, &weight, 5, &HashSet::new());
        let titles: Vec<&str> = ranked.iter().map(|&i| index[i].title.as_str()).collect();
        assert_eq!(titles, ["Dune", "Blade Runner 2049", "Arrival"]);
    }
}
