//! File sources in the app: folders, WebDAV and SMB shares shown as libraries you browse by folder.
//!
//! Item ids: `share-dir:<source>:<path>` for folders and `share-file:<source>:<path>` for files,
//! where `<source>` is the index in `Session::sources`.

use super::*;
use encheo_vfs::{Config, Kind, MediaKind, Source, names};
use std::sync::Arc;

const DIR: &str = "share-dir:";
const FILE: &str = "share-file:";

pub(crate) fn is_share_id(id: &str) -> bool {
    id.starts_with(DIR) || id.starts_with(FILE)
}

fn split_id(id: &str) -> Option<(bool, usize, String)> {
    let (is_dir, rest) = if let Some(r) = id.strip_prefix(DIR) {
        (true, r)
    } else {
        (false, id.strip_prefix(FILE)?)
    };
    let (i, path) = rest.split_once(':')?;
    Some((is_dir, i.parse().ok()?, path.to_string()))
}

fn file_id(source: usize, path: &str) -> String {
    format!("{FILE}{source}:{path}")
}

/// Sidebar entries for the file sources, after the server's libraries.
pub(crate) fn library_entries() -> Vec<(String, LibraryEntry)> {
    with_state(|s| {
        s.session
            .sources
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let key = format!("share:{i}");
                (key.clone(), LibraryEntry { key: key.into(), title: c.name.clone().into(), kind: "folder".into() })
            })
            .collect()
    })
}

/// The connected source, connecting on first use (on a worker thread; call from one).
fn connected(index: usize) -> Result<Arc<Source>, String> {
    if let Some(s) = player::STREAM_SOURCES.lock().ok().and_then(|v| v.get(index).cloned().flatten()) {
        return Ok(s);
    }
    let config = SOURCES.lock().ok().and_then(|v| v.get(index).cloned()).ok_or("source removed")?;
    let source = Arc::new(Source::connect(&config).map_err(|e| e.to_string())?);
    if let Ok(mut v) = player::STREAM_SOURCES.lock() {
        if v.len() <= index {
            v.resize(index + 1, None);
        }
        v[index] = Some(source.clone());
    }
    Ok(source)
}

/// A copy of the configs readable from worker threads.
static SOURCES: std::sync::Mutex<Vec<Config>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn sync_configs() {
    let configs = with_state(|s| s.session.sources.clone());
    if let Ok(mut v) = SOURCES.lock() {
        *v = configs;
    }
    // Forget connections; they reconnect with the current settings.
    if let Ok(mut v) = player::STREAM_SOURCES.lock() {
        v.clear();
    }
}

/// Show a folder of a source in the library grid.
pub(crate) fn browse(index: usize, path: String) {
    let ui = ui();
    let name = with_state(|s| s.session.sources.get(index).map(|c| c.name.clone())).unwrap_or_default();
    let title = if path.is_empty() { name.clone() } else { format!("{name} › {}", path.replace('/', " › ")) };
    ui.set_grid_title(title.into());
    ui.set_grid_square(false);
    ui.set_grid_can_remove(path.is_empty());
    ui.set_loading(true);
    let generation = with_state(|s| {
        s.generation += 1;
        s.share_view = Some((index, path.clone()));
        s.generation
    });
    with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    rt().spawn_blocking(move || {
        let listing = connected(index).and_then(|src| src.list(&path).map_err(|e| e.to_string()));
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            ui.set_loading(false);
            let entries = match listing {
                Ok(e) => e,
                Err(e) => return toast(ui, format!("Couldn't open {name}: {e}")),
            };
            let progress = with_state(|s| s.progress.get_or_insert_with(progress::Store::load).clone());
            let mut posters = Vec::new();
            if !path.is_empty() {
                let parent = path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
                posters.push(folder_poster(index, &parent, "Back", "icon:back"));
            }
            let mut items = Vec::new();
            for e in &entries {
                if e.is_dir {
                    posters.push(folder_poster(index, &e.path, &e.name, "icon:folder"));
                } else {
                    let item = file_item(index, &e.path, &e.name, e.size);
                    let p = progress.get(&item.rating_key);
                    posters.push(Poster {
                        id: item.rating_key.clone().into(),
                        title: display_title(&item).into(),
                        subtitle: e.name.clone().into(),
                        art_url: if item.kind == ItemKind::Track { "icon:music" } else { "icon:film" }.into(),
                        image: Image::default(),
                        progress: p.fraction().unwrap_or(0.0),
                        watched: p.watched,
                        square: false,
                    });
                    items.push(item);
                }
            }
            remember(items);
            with_models(|m| m.grid.set_vec(posters));
        });
    });
}

fn folder_poster(index: usize, path: &str, title: &str, icon: &str) -> Poster {
    Poster {
        id: format!("{DIR}{index}:{path}").into(),
        title: title.into(),
        subtitle: if icon == "icon:back" { "Up one folder".into() } else { "Folder".into() },
        art_url: icon.into(),
        image: Image::default(),
        progress: 0.0,
        watched: false,
        square: false,
    }
}

/// An item for a file, with what its name says (show and episode, or title and year).
fn file_item(index: usize, path: &str, name: &str, size: Option<u64>) -> Item {
    let id = file_id(index, path);
    let mut item = Item { rating_key: id.clone(), key: path.to_string(), ..Default::default() };
    let part = encheo_plex::Part { id: hash_id(&id), key: path.to_string(), file: Some(name.to_string()), size, ..Default::default() };
    item.media = vec![encheo_plex::Media { parts: vec![part], ..Default::default() }];
    match encheo_vfs::media_kind(name) {
        Some(MediaKind::Audio) => {
            item.kind = ItemKind::Track;
            item.title = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_string();
        }
        _ => match names::parse(name) {
            names::Parsed::Episode { show, season, episode, episode_title } => {
                item.kind = ItemKind::Episode;
                item.grandparent_title = Some(show);
                item.parent_index = Some(season);
                item.index = Some(episode);
                item.title = episode_title.unwrap_or_else(|| format!("Episode {episode}"));
            }
            names::Parsed::Movie { title, year } => {
                item.kind = ItemKind::Movie;
                item.title = title;
                item.year = year;
            }
        },
    }
    item
}

fn display_title(item: &Item) -> String {
    match item.kind {
        ItemKind::Episode => format!(
            "{} · {}",
            item.grandparent_title.clone().unwrap_or_default(),
            item.episode_label().unwrap_or_default()
        ),
        _ => match item.year {
            Some(y) => format!("{} ({y})", item.title),
            None => item.title.clone(),
        },
    }
}

/// A stable number for a file, used where server items have a numeric id (language cache).
fn hash_id(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish() | (1 << 63)
}

/// Handle a click on a share item. Returns false if `id` isn't one.
pub(crate) fn open(id: &str) -> bool {
    let Some((is_dir, index, path)) = split_id(id) else { return false };
    if is_dir {
        browse(index, path);
    } else {
        play_file(index, &path);
    }
    true
}

/// The folder being shown, again (after playback, or Refresh).
pub(crate) fn reload_current() -> bool {
    match with_state(|s| s.share_view.clone()) {
        Some((index, path)) => {
            browse(index, path);
            true
        }
        None => false,
    }
}

/// Play a file. Its folder's other videos become the episode list (previous/next, autoplay).
fn play_file(index: usize, path: &str) {
    let id = file_id(index, path);
    let Some(item) = with_state(|s| s.items.get(&id).cloned()) else { return };
    let siblings: Vec<Item> = with_state(|s| {
        let folder = path.rsplit_once('/').map(|(f, _)| f).unwrap_or("");
        let prefix = format!("{FILE}{index}:");
        let mut v: Vec<Item> = s
            .items
            .values()
            .filter(|i| {
                i.rating_key.starts_with(&prefix)
                    && i.kind == item.kind
                    && i.key.rsplit_once('/').map(|(f, _)| f).unwrap_or("") == folder
            })
            .cloned()
            .collect();
        v.sort_by(|a, b| names::natural_cmp(&a.key, &b.key));
        v
    });
    let resume = with_state(|s| s.progress.get_or_insert_with(progress::Store::load).get(&id));
    let ui = ui();
    let back_to = ui.get_screen().to_string();
    if back_to != "player" {
        with_state(|s| s.return_to = back_to);
    }
    ps(&ui).set_buffering(true);
    let path = path.to_string();
    rt().spawn_blocking(move || {
        let target = connected(index).map(|src| src.direct_url(&path));
        on_ui(move |ui| {
            let target = match target {
                Ok(t) => t,
                Err(e) => {
                    ps(ui).set_buffering(false);
                    return toast(ui, format!("Couldn't open the file: {e}"));
                }
            };
            let Some(p) = player() else { return toast(ui, "The video player didn't start (libmpv missing?).") };
            if item.kind == ItemKind::Track {
                // Music from a folder: play it and the rest of the folder as a queue.
                let start = siblings.iter().position(|s| s.rating_key == item.rating_key).unwrap_or(0);
                return play_music(siblings, start);
            }
            if with_state(|s| s.music.take()).is_some() {
                ui.set_music_active(false);
            }
            let ps = ps(ui);
            ps.set_title(display_title(&item).into());
            ps.set_subtitle(item.primary_part().and_then(|p| p.file.clone()).unwrap_or_default().into());
            ps.set_position(0.0);
            ps.set_duration(0.0);
            ps.set_skip_label(SharedString::new());
            ps.set_controls_visible(true);
            ps.set_panel(SharedString::new());
            ui.set_screen("player".into());
            let start = resume.fraction().map(|_| resume.position);
            match target {
                Some(direct) => p.load_with_headers(&direct.url, start, &direct.headers),
                None => p.load(&player::stream_url(index, &path), start),
            }
            let episodes = if item.kind == ItemKind::Episode { siblings } else { vec![] };
            with_state(|s| {
                s.now_playing = Some(NowPlaying {
                    item,
                    episodes,
                    streams_applied: false,
                    detected: HashMap::new(),
                    user_picked_sub: false,
                    user_picked_audio: false,
                    show_key: None,
                    show_languages: langs::ShowLanguages::default(),
                    show_settings: vec![],
                    last_report: Instant::now(),
                    started: Instant::now(),
                });
                s.last_poke = Some(Instant::now());
            });
            update_neighbours(ui);
        });
    });
}

/// In-progress files for the home screen's Continue Watching, most recent first. Only for
/// sources that still exist.
pub(crate) fn continue_watching() -> Vec<(Poster, Item)> {
    let (store, sources) = with_state(|s| (s.progress.get_or_insert_with(progress::Store::load).clone(), s.session.sources.len()));
    store
        .in_progress()
        .into_iter()
        .filter_map(|(id, p)| {
            let (false, index, path) = split_id(&id)? else { return None };
            if index >= sources {
                return None;
            }
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            let item = file_item(index, &path, &name, None);
            let poster = Poster {
                id: id.clone().into(),
                title: display_title(&item).into(),
                subtitle: format!("{} left", clock((p.duration - p.position).max(0.0))).into(),
                art_url: "icon:film".into(),
                image: Image::default(),
                progress: p.fraction().unwrap_or(0.0),
                watched: false,
                square: false,
            };
            Some((poster, item))
        })
        .take(12)
        .collect()
}

/// Play a neighbouring file (previous/next/list in the player).
pub(crate) fn play_item(item: &Item) -> bool {
    match split_id(&item.rating_key) {
        Some((false, index, path)) => {
            play_file(index, &path);
            true
        }
        _ => false,
    }
}

/// Save where playback is, for a file.
pub(crate) fn record_progress(item: &Item, position: f64) {
    let duration = player().map(|p| p.status().duration).filter(|d| *d > 0.0).unwrap_or(0.0);
    with_state(|s| s.progress.get_or_insert_with(progress::Store::load).record(&item.rating_key, position, duration));
}

/// What mpv opens for a music file in a queue (through `encheo://`, so credentials need no headers).
pub(crate) fn music_url(item: &Item) -> Option<String> {
    let (_, index, path) = split_id(&item.rating_key)?;
    Some(player::stream_url(index, &path))
}

/// A reader for the playing file, for subtitle language detection.
pub(crate) fn open_reader(item: &Item) -> Option<Box<dyn encheo_vfs::ReadSeek>> {
    let (_, index, path) = split_id(&item.rating_key)?;
    connected(index).ok()?.open(&path).ok()
}

/// The add-source form was submitted: connect, and keep it if that works.
pub(crate) fn add_source(kind: i32, name: String, location: String, username: String, password: String) {
    let kind = match kind {
        0 => Kind::Local,
        1 => Kind::Smb,
        _ => Kind::WebDav,
    };
    let location = location.trim().to_string();
    let name = if name.trim().is_empty() { default_name(kind, &location) } else { name.trim().to_string() };
    let config = Config { kind, name, location, username: username.trim().to_string(), password };
    let ui = ui();
    ui.set_add_source_busy(true);
    ui.set_add_source_status("Connecting…".into());
    rt().spawn_blocking(move || {
        let result = Source::connect(&config);
        on_ui(move |ui| {
            ui.set_add_source_busy(false);
            match result {
                Ok(_) => {
                    ui.set_add_source_open(false);
                    ui.set_add_source_status(SharedString::new());
                    let index = with_state(|s| {
                        s.session.sources.push(config);
                        s.session.save();
                        s.session.sources.len() - 1
                    });
                    sync_configs();
                    refresh_sidebar();
                    let lib = with_state(|s| s.library_keys.iter().position(|k| *k == format!("share:{index}")));
                    if let Some(i) = lib {
                        select_library(i as i32);
                    }
                }
                Err(e) => ui.set_add_source_status(
                    match e {
                        encheo_vfs::Error::Auth => "Wrong user name or password.".to_string(),
                        e => format!("Couldn't connect: {e}"),
                    }
                    .into(),
                ),
            }
        });
    });
}

fn default_name(kind: Kind, location: &str) -> String {
    let trimmed = location.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if last.is_empty() || last.contains(':') { kind.label().to_string() } else { last.to_string() }
}

/// Remove the source being shown.
pub(crate) fn remove_current() {
    let Some((index, _)) = with_state(|s| s.share_view.clone()) else { return };
    with_state(|s| {
        if index < s.session.sources.len() {
            s.session.sources.remove(index);
            s.session.save();
        }
        s.share_view = None;
    });
    sync_configs();
    refresh_sidebar();
    select_library(-1);
}

/// Rebuild the sidebar: server libraries, then file sources.
pub(crate) fn refresh_sidebar() {
    with_state(|s| {
        let server_libs = s.library_kinds.iter().filter(|k| k.as_str() != "folder").count();
        s.library_keys.truncate(server_libs);
        s.library_kinds.truncate(server_libs);
    });
    with_models(|m| {
        let server_libs = with_state(|s| s.library_keys.len());
        while m.libraries.row_count() > server_libs {
            m.libraries.remove(m.libraries.row_count() - 1);
        }
        for (key, entry) in library_entries() {
            with_state(|s| {
                s.library_keys.push(key);
                s.library_kinds.push("folder".into());
            });
            m.libraries.push(entry);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(split_id("share-file:2:TV/Show/S01E01.mkv"), Some((false, 2, "TV/Show/S01E01.mkv".into())));
        assert_eq!(split_id("share-dir:0:"), Some((true, 0, "".into())));
        assert_eq!(split_id("12345"), None);
    }

    #[test]
    fn items_from_names() {
        let ep = file_item(1, "TV/The.Expanse.S02E05.Home.mkv", "The.Expanse.S02E05.Home.mkv", Some(5));
        assert_eq!(ep.kind, ItemKind::Episode);
        assert_eq!(display_title(&ep), "The Expanse · S2 · E5");
        let mv = file_item(1, "Heat (1995).mkv", "Heat (1995).mkv", None);
        assert_eq!(display_title(&mv), "Heat (1995)");
        assert_eq!(file_item(0, "a/01 Song.flac", "01 Song.flac", None).kind, ItemKind::Track);
    }

    #[test]
    fn names_for_new_sources() {
        assert_eq!(default_name(Kind::Smb, "smb://nas/Media/"), "Media");
        assert_eq!(default_name(Kind::Local, "/Users/me/Movies"), "Movies");
        assert_eq!(default_name(Kind::WebDav, "https://dav.example.com/"), "dav.example.com");
    }
}
