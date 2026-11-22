//! File sources in the app: folders, WebDAV and SMB shares shown as libraries you browse by folder.
//!
//! Item ids: `share-dir:<source>:<path>` for folders and `share-file:<source>:<path>` for files,
//! where `<source>` is the source's number (`Session::source_ids`), not its place in the list.

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

/// `split_id`, with the source's place in the list in place of its number. None once it's removed.
fn locate(id: &str) -> Option<(bool, usize, String)> {
    let (is_dir, number, path) = split_id(id)?;
    let index = SOURCES.lock().ok()?.iter().position(|(n, _)| *n == number)?;
    Some((is_dir, index, path))
}

/// The number of the source at `index`, which ids carry.
fn number(index: usize) -> usize {
    SOURCES.lock().ok().and_then(|v| v.get(index).map(|(n, _)| *n)).unwrap_or(index)
}

fn file_id(index: usize, path: &str) -> String {
    format!("{FILE}{}:{path}", number(index))
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
                (key.clone(), LibraryEntry { key: key.into(), title: c.name.clone().into(), kind: "folder".into(), group: "Sources".into() })
            })
            .collect()
    })
}

/// The connected source, connecting on first use (on a worker thread; call from one).
fn connected(index: usize) -> Result<Arc<Source>, String> {
    if let Some(s) = player::STREAM_SOURCES.lock().ok().and_then(|v| v.get(index).cloned().flatten()) {
        return Ok(s);
    }
    let config = SOURCES.lock().ok().and_then(|v| v.get(index).map(|(_, c)| c.clone())).ok_or("source removed")?;
    let source = Arc::new(Source::connect(&config).map_err(|e| e.to_string())?);
    if let Ok(mut v) = player::STREAM_SOURCES.lock() {
        if v.len() <= index {
            v.resize(index + 1, None);
        }
        v[index] = Some(source.clone());
    }
    Ok(source)
}

/// A copy of the sources (number, config) readable from worker threads.
static SOURCES: std::sync::Mutex<Vec<(usize, Config)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn sync_configs() {
    let configs: Vec<(usize, Config)> = with_state(|s| s.session.source_ids.iter().copied().zip(s.session.sources.iter().cloned()).collect());
    if let Ok(mut v) = SOURCES.lock() {
        *v = configs;
    }
    // Forget connections; they reconnect with the current settings.
    if let Ok(mut v) = player::STREAM_SOURCES.lock() {
        v.clear();
    }
    forget_index();
}

/// Show a folder of a source in the library grid.
pub(crate) fn browse(index: usize, path: String) {
    let ui = ui();
    let name = with_state(|s| s.session.sources.get(index).map(|c| c.name.clone())).unwrap_or_default();
    let title = if path.is_empty() { name.clone() } else { format!("{name} › {}", path.replace('/', " › ")) };
    ui.set_grid_title(title.into());
    ui.set_grid_square(false);
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
                    posters.push(file_poster(&item, &e.name, &progress));
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
        id: format!("{DIR}{}:{path}", number(index)).into(),
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

pub(crate) fn file_poster(item: &Item, file_name: &str, progress: &progress::Store) -> Poster {
    let p = progress.get(&item.rating_key);
    Poster {
        id: item.rating_key.clone().into(),
        title: display_title(item).into(),
        subtitle: file_name.into(),
        art_url: if item.kind == ItemKind::Track { "icon:music" } else { "icon:film" }.into(),
        image: Image::default(),
        progress: p.fraction().unwrap_or(0.0),
        watched: p.watched,
        square: false,
    }
}

// ---------- Search ----------

/// Every media file of a source, collected the first time it's searched (the walk can take a while
/// on a big NAS) and kept until Refresh or a settings change.
type FileIndex = Arc<Vec<encheo_vfs::Entry>>;
static INDEX: std::sync::Mutex<Vec<Arc<std::sync::Mutex<Option<FileIndex>>>>> = std::sync::Mutex::new(Vec::new());

/// Upper bounds for one walk, so a huge or looping share can't hang search.
const MAX_FOLDERS: usize = 3000;
const MAX_DEPTH: usize = 12;

pub(crate) fn forget_index() {
    if let Ok(mut v) = INDEX.lock() {
        v.clear();
    }
}

fn file_index(index: usize) -> Result<FileIndex, String> {
    let slot = {
        let mut v = INDEX.lock().map_err(|e| e.to_string())?;
        if v.len() <= index {
            v.resize_with(index + 1, Default::default);
        }
        v[index].clone()
    };
    // Held while walking: a second search of the same source waits for this walk and reuses it.
    let mut slot = slot.lock().map_err(|e| e.to_string())?;
    if let Some(files) = slot.as_ref() {
        return Ok(files.clone());
    }
    let src = connected(index)?;
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::from([(String::new(), 0usize)]);
    let mut folders = 0;
    while let Some((path, depth)) = queue.pop_front() {
        folders += 1;
        if folders > MAX_FOLDERS {
            break;
        }
        let Ok(entries) = src.list(&path) else { continue };
        for e in entries {
            if e.is_dir {
                if depth < MAX_DEPTH {
                    queue.push_back((e.path, depth + 1));
                }
            } else if seen.insert((e.name.clone(), e.size)) {
                // DLNA servers list the same file under several views; keep it once.
                files.push(e);
            }
        }
    }
    let files = Arc::new(files);
    *slot = Some(files.clone());
    Ok(files)
}

/// Lowercase words, punctuation and dots as separators ("The.Bear.S01E02" → the bear s01e02).
fn words(text: &str) -> String {
    text.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' }).collect()
}

fn matches(query: &[String], haystack: &str) -> bool {
    let hay: Vec<&str> = haystack.split_whitespace().collect();
    query.iter().all(|q| hay.iter().any(|h| h.starts_with(q.as_str())))
}

/// Files in all sources whose name or parsed title matches. Runs on a worker thread; returns
/// items with their file names (posters are built on the UI thread).
pub(crate) fn search_files(query: &str) -> Vec<(Item, String)> {
    let q: Vec<String> = words(query).split_whitespace().map(str::to_string).collect();
    if q.is_empty() {
        return vec![];
    }
    let count = SOURCES.lock().map(|v| v.len()).unwrap_or(0);
    let mut out = Vec::new();
    for index in 0..count {
        let Ok(files) = file_index(index) else { continue };
        for e in files.iter() {
            let item = file_item(index, &e.path, &e.name, e.size);
            let hay = words(&format!("{} {} {}", e.name, display_title(&item), item.grandparent_title.as_deref().unwrap_or("")));
            if matches(&q, &hay) {
                out.push((item, e.name.clone()));
                if out.len() >= 200 {
                    return out;
                }
            }
        }
    }
    out
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
    let Some((is_dir, index, path)) = locate(id) else { return is_share_id(id) };
    if is_dir {
        scroll::grid_to_top(&ui());
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
        let prefix = format!("{FILE}{}:", number(index));
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
        let target = connected(index).and_then(|src| {
            if encheo_vfs::is_strm(&path) { strm_url(&src, &path).map(Some) } else { Ok(src.direct_url(&path)) }
        });
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
            ps.set_can_find_subs(false);
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
                    skipped: vec![],
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
    let store = with_state(|s| s.progress.get_or_insert_with(progress::Store::load).clone());
    store
        .in_progress()
        .into_iter()
        .filter_map(|(id, p)| {
            let (false, index, path) = locate(&id)? else { return None };
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
    match locate(&item.rating_key) {
        Some((false, index, path)) => {
            play_file(index, &path);
            true
        }
        _ => false,
    }
}

/// A `.strm` file: the stream it points at, played as it is.
fn strm_url(src: &Source, path: &str) -> Result<encheo_vfs::DirectUrl, String> {
    use std::io::Read;
    let mut text = String::new();
    // An address fits in a few hundred bytes; don't read a mislabelled video.
    src.open(path).map_err(|e| e.to_string())?.take(16 * 1024).read_to_string(&mut text).map_err(|_| "it isn't a text file".to_string())?;
    let url = encheo_vfs::strm_target(&text).ok_or("it doesn't hold an http, https, rtsp or rtmp address")?;
    Ok(encheo_vfs::DirectUrl { url, headers: vec![] })
}

/// Save where playback is, for a file.
pub(crate) fn record_progress(item: &Item, position: f64) {
    let duration = player().map(|p| p.status().duration).filter(|d| *d > 0.0).unwrap_or(0.0);
    with_state(|s| s.progress.get_or_insert_with(progress::Store::load).record(&item.rating_key, position, duration));
}

/// What mpv opens for a music file in a queue (through `encheo://`, so credentials need no headers).
pub(crate) fn music_url(item: &Item) -> Option<String> {
    let (_, index, path) = locate(&item.rating_key)?;
    Some(player::stream_url(index, &path))
}

/// A reader for the playing file, for subtitle language detection.
pub(crate) fn open_reader(item: &Item) -> Option<Box<dyn encheo_vfs::ReadSeek>> {
    let (_, index, path) = locate(&item.rating_key)?;
    connected(index).ok()?.open(&path).ok()
}

/// The form's numbers for the kinds of file source (3 is Jellyfin/Emby, 8 Plex).
const FORM_KINDS: [(i32, Kind); 7] =
    [(0, Kind::Local), (1, Kind::Smb), (2, Kind::WebDav), (4, Kind::Sftp), (5, Kind::Ftp), (6, Kind::Nfs), (7, Kind::Dlna)];

/// The add-source form was submitted: connect, and keep it (or the edited one) if that works.
pub(crate) fn add_source(kind: i32, name: String, location: String, username: String, password: String) {
    let editing = with_state(|s| s.editing);
    if kind == 8 {
        let ui = ui();
        ui.set_add_source_open(false);
        ui.set_sources_open(false);
        return begin_signin();
    }
    if kind == 9 {
        return crate::requests::sign_in(location, username, password);
    }
    if kind == 3 {
        let replace = match editing {
            Some(Editing::Server(i)) => Some(i),
            _ => None,
        };
        return add_media_server(location, username, password, replace);
    }
    let kind = FORM_KINDS.iter().find(|(n, _)| *n == kind).map_or(Kind::WebDav, |(_, k)| *k);
    let location = location.trim().to_string();
    let name = if name.trim().is_empty() { default_name(kind, &location) } else { name.trim().to_string() };
    let mut config = Config { kind, name, location, username: username.trim().to_string(), password, host_key: None };
    let editing = match editing {
        Some(Editing::Files(i)) => with_state(|s| s.session.sources.get(i).cloned()).map(|old| (i, old)),
        _ => None,
    };
    if let Some((_, old)) = &editing {
        // An empty password field means "unchanged"; a server key stays pinned to its address.
        if config.password.is_empty() {
            config.password = old.password.clone();
        }
        if config.location == old.location {
            config.host_key = old.host_key.clone();
        }
    }
    let ui = ui();
    ui.set_add_source_busy(true);
    ui.set_add_source_status("Connecting…".into());
    rt().spawn_blocking(move || {
        let result = Source::connect(&config);
        on_ui(move |ui| {
            ui.set_add_source_busy(false);
            match result {
                Ok(source) => {
                    let mut config = config;
                    config.host_key = source.host_key(); // SFTP: pin the server's key
                    ui.set_add_source_open(false);
                    ui.set_add_source_status(SharedString::new());
                    if let Some((index, _)) = editing {
                        with_state(|s| {
                            s.session.sources[index] = config;
                            s.session.save();
                            s.editing = None;
                        });
                        sync_configs();
                        refresh_sidebar();
                        return fill_sources();
                    }
                    ui.set_sources_open(false);
                    let index = with_state(|s| s.session.add_source(config));
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

/// Sign in to a Jellyfin or Emby server and add its libraries (or replace the login at `replace`).
fn add_media_server(address: String, username: String, password: String, replace: Option<usize>) {
    let ui = ui();
    ui.set_add_source_busy(true);
    ui.set_add_source_status("Signing in…".into());
    let client = with_state(|s| backend::jf_client(&s.session));
    rt().spawn(async move {
        let result = encheo_jellyfin::sign_in(&address, username.trim(), &password, &client).await;
        on_ui(move |ui| {
            ui.set_add_source_busy(false);
            match result {
                Ok(login) => {
                    ui.set_add_source_open(false);
                    ui.set_add_source_status(SharedString::new());
                    let edited = replace.is_some();
                    with_state(|s| {
                        s.session.add_media_server(login, replace);
                        s.items.clear();
                        s.editing = None;
                    });
                    backend::load_jf_servers();
                    enter_home();
                    if edited {
                        fill_sources();
                    } else {
                        ui.set_sources_open(false);
                    }
                }
                Err(e) => ui.set_add_source_status(format!("{e}").into()),
            }
        });
    });
}

fn default_name(kind: Kind, location: &str) -> String {
    let trimmed = location.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if last.is_empty() || last.contains(':') { kind.label().to_string() } else { last.to_string() }
}

// ---------- Sources window ----------

/// What the add/edit form is changing: a file source or a Jellyfin/Emby login, by position.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Editing {
    Files(usize),
    Server(usize),
}

/// Open the Sources window.
pub(crate) fn open_sources() {
    fill_sources();
    let ui = ui();
    ui.set_sources_confirm(-1);
    ui.set_sources_open(true);
    crate::focus_settings();
}

/// The Sources window's list: Plex, then Jellyfin/Emby servers, then file sources.
pub(crate) fn fill_sources() {
    let who = crate::profiles::current_name();
    let rows = with_state(|s| {
        let plex = match (&s.session.token, &s.session.server) {
            (Some(_), Some(server)) => SourceRow {
                kind: "plex".into(),
                title: server.name.clone().into(),
                label: "Plex".into(),
                detail: if who.is_empty() { server.base_url.clone() } else { format!("{who} · {}", server.base_url) }
                .into(),
                signed_in: true,
                ..Default::default()
            },
            (Some(_), None) => SourceRow { kind: "plex".into(), title: "Plex".into(), label: "Plex".into(), detail: "Signed in, no server yet".into(), signed_in: true, ..Default::default() },
            _ => SourceRow { kind: "plex".into(), title: "Plex".into(), label: "Plex".into(), detail: "Not signed in".into(), ..Default::default() },
        };
        let servers = s.session.media_servers.iter().enumerate().map(|(i, l)| SourceRow {
            kind: "server".into(),
            index: i as i32,
            title: l.server_name.clone().into(),
            label: if l.flavor == encheo_jellyfin::Flavor::Emby { "Emby" } else { "Jellyfin" }.into(),
            detail: format!("{} · {}", l.user_name, l.base_url).into(),
            signed_in: true,
        });
        let files = s.session.sources.iter().enumerate().map(|(i, c)| SourceRow {
            kind: "files".into(),
            index: i as i32,
            title: c.name.clone().into(),
            label: c.kind.label().into(),
            detail: c.location.clone().into(),
            signed_in: true,
        });
        std::iter::once(plex).chain(servers).chain(files).collect::<Vec<_>>()
    });
    let rows: Vec<SourceRow> = rows.into_iter().chain(crate::requests::source_row()).collect();
    ui().set_sources(ModelRc::new(VecModel::from(rows)));
}

/// Open the form for a new source. Plex comes first until the user has signed in to it.
pub(crate) fn new_source() {
    let plex = with_state(|s| {
        s.editing = None;
        s.session.token.is_some()
    });
    let ui = ui();
    ui.set_form_editing(false);
    ui.set_form_kind(if plex { 1 } else { 8 });
    for set in [AppWindow::set_form_name, AppWindow::set_form_location, AppWindow::set_form_user, AppWindow::set_form_password] {
        set(&ui, SharedString::new());
    }
    ui.set_add_source_status(SharedString::new());
    ui.set_add_source_open(true);
    scan_network();
}

/// Open the form filled in with a source's settings. The password stays blank: blank keeps it.
pub(crate) fn edit_source(kind: &str, index: usize) {
    let ui = ui();
    if kind == "seerr" {
        let Some((address, login)) = crate::requests::form() else { return };
        with_state(|s| s.editing = None);
        ui.set_form_editing(true);
        ui.set_form_kind(9);
        ui.set_form_name(SharedString::new());
        ui.set_form_location(address.into());
        ui.set_form_user(login.into());
        ui.set_form_password(SharedString::new());
        ui.set_add_source_status(SharedString::new());
        ui.set_add_source_open(true);
        return;
    }
    let found = with_state(|s| match kind {
        "files" => s.session.sources.get(index).map(|c| {
            let n = FORM_KINDS.iter().find(|(_, k)| *k == c.kind).map_or(2, |(n, _)| *n);
            (Editing::Files(index), n, c.name.clone(), c.location.clone(), c.username.clone())
        }),
        "server" => s.session.media_servers.get(index).map(|l| (Editing::Server(index), 3, l.server_name.clone(), l.base_url.clone(), l.user_name.clone())),
        _ => None,
    });
    let Some((editing, form_kind, name, location, user)) = found else { return };
    with_state(|s| s.editing = Some(editing));
    ui.set_form_editing(true);
    ui.set_form_kind(form_kind);
    ui.set_form_name(name.into());
    ui.set_form_location(location.into());
    ui.set_form_user(user.into());
    ui.set_form_password(SharedString::new());
    ui.set_add_source_status(SharedString::new());
    ui.set_add_source_open(true);
}

/// Remove a source (the user has confirmed). Plex: sign out.
pub(crate) fn remove_source(kind: &str, index: usize) {
    match kind {
        "seerr" => crate::requests::remove(),
        "plex" => {
            let others = with_state(|s| {
                s.session.sign_out();
                s.server = None;
                s.items.clear();
                let others = !s.session.media_servers.is_empty() || !s.session.sources.is_empty();
                s.session.skip_plex = others;
                s.session.save();
                others
            });
            ui().set_can_switch_profile(false);
            if !others {
                ui().set_sources_open(false);
                return begin_signin();
            }
            enter_home();
        }
        "server" => {
            with_state(|s| {
                s.session.remove_media_server(index);
                s.items.clear();
            });
            backend::load_jf_servers();
            enter_home();
        }
        _ => {
            with_state(|s| {
                s.session.remove_source(index);
                s.share_view = None;
            });
            sync_configs();
            refresh_sidebar();
            select_library(-1);
        }
    }
    fill_sources();
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

#[cfg(test)]
mod search_tests {
    use super::{matches, words};

    fn q(s: &str) -> Vec<String> {
        words(s).split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn matches_word_prefixes() {
        let hay = words("The.Bear.S01E02.1080p.mkv The Bear · S1 · E2");
        assert!(matches(&q("bear"), &hay));
        assert!(matches(&q("the be"), &hay));
        assert!(matches(&q("BEAR s01e02"), &hay));
        assert!(!matches(&q("bears"), &hay));
        assert!(!matches(&q("ear"), &hay));
    }
}

#[cfg(test)]
mod walk_tests {
    /// Against the MiniDLNA test container, if it's running: every file once, though DLNA lists
    /// each under several views.
    #[test]
    fn searches_a_real_dlna_server() {
        if std::net::TcpStream::connect("127.0.0.1:8200").is_err() {
            return;
        }
        let config = encheo_vfs::Config {
            kind: encheo_vfs::Kind::Dlna,
            name: "t".into(),
            location: "http://127.0.0.1:8200/rootDesc.xml".into(),
            username: String::new(),
            password: String::new(),
            host_key: None,
        };
        *super::SOURCES.lock().unwrap() = vec![(0, config)];
        let found = super::search_files("big test");
        let names: Vec<_> = found.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["Big.Test.2024.1080p.WEB.mkv"], "{names:?}");
        assert_eq!(found[0].0.title, "Big Test");
    }
}
