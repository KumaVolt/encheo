//! Encheo desktop app: Plex sign-in, browsing, details and playback.
//!
//! Threads: the UI thread owns all Slint objects and the app state (thread-locals below). Network
//! and decoding run on a tokio runtime; results come back through `on_ui`.

#[cfg(target_os = "android")]
mod android_media;
#[cfg(target_os = "android")]
mod android_video;
mod anilist;
mod backend;
mod favourites;
mod images;
mod langs;
mod lyrics;
mod profiles;
mod progress;
mod shares;
mod player;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod screenshots;
#[cfg(target_os = "ios")]
mod ios_video;
mod session;

use encheo_plex::{ClientInfo, Item, ItemKind, MarkerKind, PlayState, Plex, Server, Stream, StreamKind};
use player::{Player, clock};
use backend::{all_backends, backend_for};
use session::{SavedServer, Session};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

slint::include_modules!();

const PAGE: u32 = 200;

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static IMAGES: OnceLock<images::Loader> = OnceLock::new();

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio"))
}

struct Models {
    libraries: Rc<VecModel<LibraryEntry>>,
    continue_watching: Rc<VecModel<Poster>>,
    home_rows: Rc<VecModel<HomeRow>>,
    /// The poster models inside `home_rows`, kept so artwork can be filled in as it arrives.
    row_models: RefCell<Vec<Rc<VecModel<Poster>>>>,
    /// Where each of those rows fetches more titles when scrolled near its end.
    row_more: RefCell<Vec<RowMore>>,
    grid: Rc<VecModel<Poster>>,
    albums: Rc<VecModel<Poster>>,
    tracks: Rc<VecModel<Track>>,
    /// What's queued, for Now Playing.
    queue: Rc<VecModel<Track>>,
    /// A music library's Songs tab.
    songs: Rc<VecModel<Track>>,
    people: Rc<VecModel<Person>>,
    related: Rc<VecModel<Poster>>,
    seasons: Rc<VecModel<SharedString>>,
    /// Per season: every episode watched.
    seasons_watched: Rc<VecModel<bool>>,
    episodes: Rc<VecModel<Episode>>,
}

#[derive(Default)]
struct State {
    session: Session,
    server: Option<Server>,
    /// Jellyfin/Emby servers, in `session.media_servers` order.
    jf_servers: Vec<encheo_jellyfin::Server>,
    library_keys: Vec<String>,
    library_kinds: Vec<String>,
    /// The file source and folder shown in the grid, if a file source is selected.
    share_view: Option<(usize, String)>,
    progress: Option<progress::Store>,
    favourites: Option<favourites::Store>,
    /// Everything we've seen, by rating key, so clicks can find the full item.
    items: HashMap<String, Item>,
    images: images::Cache,
    pending_images: HashSet<String>,
    /// Bumped when the user navigates; late results for an older view are dropped.
    generation: u64,
    signin_generation: u64,
    search_generation: u64,
    /// Where the keyboard/remote focus was on the home screen before opening a details page.
    home_nav: Option<(SharedString, i32, i32)>,
    /// The account's preferred subtitle language (ISO 639-3), from the Plex profile.
    subtitle_language: Option<String>,
    language_cache: Option<langs::Cache>,
    show_store: Option<langs::ShowStore>,
    detail: Option<Item>,
    detail_backdrop_url: String,
    /// The details page's trailer, once its server has said whether there is one.
    trailer: Option<backend::Trailer>,
    /// The title in Home's billboard.
    billboard: Option<Item>,
    /// Where the remote's focus was before Settings opened.
    dialog_return: Option<(SharedString, i32, i32)>,
    /// What the add/edit source form is editing; None when it adds.
    editing: Option<shares::Editing>,
    season_items: Vec<Item>,
    next_up: Option<Item>,
    now_playing: Option<NowPlaying>,
    /// Tracks of the album on the detail screen.
    album_tracks: Vec<Item>,
    /// Music libraries: the tab shown (see `MUSIC_TABS`).
    music_tab: i32,
    /// The Songs tab's songs, in order, for playing on from one of them.
    songs: Vec<Item>,
    /// TV Home: the title the remote rested on last tick, about to fill the billboard.
    hero_candidate: Option<String>,
    /// The collection on the collection page.
    collection: Option<Item>,
    /// The artist page's albums, for Play and Shuffle.
    artist_albums: Vec<Item>,
    /// Music repeat: 0 off, 1 the whole queue, 2 the current track.
    music_repeat: u8,
    /// Where the remote focus was before it moved to the music bar.
    music_return: Option<(SharedString, i32, i32)>,
    music: Option<MusicQueue>,
    return_to: String,
    last_poke: Option<Instant>,
    /// Window size and position to go back to when leaving Picture in Picture.
    #[cfg(not(target_os = "android"))]
    pip_restore: Option<(slint::winit_030::winit::dpi::PhysicalSize<u32>, Option<slint::winit_030::winit::dpi::PhysicalPosition<i32>>)>,
}

/// Music plays from an mpv playlist; this mirrors it so we know which Plex item each entry is.
struct MusicQueue {
    items: Vec<Item>,
    index: usize,
    last_report: Instant,
    art_url: String,
    /// The cover for Now Playing.
    art_large_url: String,
    /// Lyrics fetched for this queue's songs, by id (None: the song has none).
    lyrics: HashMap<String, Option<encheo_plex::Lyrics>>,
    /// The song whose lyrics are being fetched.
    lyrics_loading: Option<String>,
    /// The song whose lyrics (or "Finding the words…") are on screen.
    lyrics_shown: String,
}

struct NowPlaying {
    item: Item,
    /// The show's episodes, in order, when playing an episode.
    episodes: Vec<Item>,
    /// Whether the user's Plex track choices have been applied to this file yet.
    streams_applied: bool,
    /// Languages detected from the text of untagged subtitle tracks, by stream index.
    detected: HashMap<i64, langs::Detected>,
    /// Markers (by start, ms) already auto-skipped, so seeking back into one doesn't skip again.
    skipped: Vec<u64>,
    /// The user chose a subtitle track themselves; don't second-guess it.
    user_picked_sub: bool,
    user_picked_audio: bool,
    /// For episodes: "<server>:<show>" and the languages to pick for this show.
    show_key: Option<String>,
    show_languages: langs::ShowLanguages,
    /// The show's Plex settings, to write language changes back in the server's own values.
    show_settings: Vec<encheo_plex::Setting>,
    last_report: Instant,
    started: Instant,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static MODELS: RefCell<Option<Models>> = const { RefCell::new(None) };
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    static PLAYER: RefCell<Option<Rc<Player>>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn with_models<R>(f: impl FnOnce(&Models) -> R) -> R {
    MODELS.with(|m| f(m.borrow().as_ref().expect("models")))
}

fn ui() -> AppWindow {
    UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade())).expect("window")
}

fn player() -> Option<Rc<Player>> {
    PLAYER.with(|p| p.borrow().clone())
}

/// Run `f` on the UI thread.
fn on_ui(f: impl FnOnce(&AppWindow) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade())) {
            f(&w);
        }
    });
}

fn toast(ui: &AppWindow, msg: impl Into<SharedString>) {
    ui.set_toast(msg.into());
    slint::Timer::single_shot(Duration::from_secs(5), || ui_clear_toast());
}

fn ui_clear_toast() {
    ui().set_toast(SharedString::new());
}

fn plex(session: &Session) -> Plex {
    let plex = Plex::new(ClientInfo::new(session.client_id.clone(), format!("Encheo ({})", std::env::consts::OS)));
    match plex_tv_stand_in() {
        Some(base) => plex.with_base_urls(base, base),
        None => plex,
    }
}

/// Development: something else answering for plex.tv, such as the demo server (profiles,
/// servers). ENCHEO_PLEX_TV=<url>, or on Android `setprop debug.encheo.plextv <url>`.
fn plex_tv_stand_in() -> Option<&'static str> {
    static BASE: OnceLock<Option<String>> = OnceLock::new();
    BASE.get_or_init(|| {
        #[cfg(target_os = "android")]
        let v = player::android_prop_value("debug.encheo.plextv");
        #[cfg(not(target_os = "android"))]
        let v = std::env::var("ENCHEO_PLEX_TV").unwrap_or_default();
        Some(v.trim_end_matches('/').to_string()).filter(|v| v.starts_with("http"))
    })
    .as_deref()
}

// ---------- Sign-in and server ----------

fn begin_signin() {
    let (plex, generation) = with_state(|s| {
        s.signin_generation += 1;
        (plex(&s.session), s.signin_generation)
    });
    let ui = ui();
    ui.set_screen("signin".into());
    ui.set_pin_code(SharedString::new());
    ui.set_signin_status("Getting a code…".into());
    rt().spawn(async move {
        let pin = match plex.create_pin(false).await {
            Ok(p) => p,
            Err(e) => return on_ui(move |ui| ui.set_signin_status(format!("Couldn't reach plex.tv: {e}").into())),
        };
        let code = pin.code.clone();
        on_ui(move |ui| {
            ui.set_pin_code(code.into());
            ui.set_signin_status("Waiting for you to approve…".into());
        });
        let result = plex.wait_for_pin(&pin).await;
        on_ui(move |ui| {
            if with_state(|s| s.signin_generation) != generation {
                return; // a newer code replaced this one
            }
            match result {
                Ok(token) => {
                    with_state(|s| {
                        s.session.token = Some(token.clone());
                        s.session.skip_plex = false;
                        s.session.save();
                    });
                    ui.set_signin_status("Signed in. Finding your servers…".into());
                    choose_server(token, None);
                    profiles::after_signin();
                }
                Err(e) => ui.set_signin_status(format!("{e}. Get a new code to try again.").into()),
            }
        });
    });
}

/// Find the account's servers and connect to `prefer` (by client identifier) or the best one.
fn choose_server(token: String, prefer: Option<String>) {
    let plex = with_state(|s| plex(&s.session));
    rt().spawn(async move {
        let servers = match plex.servers(&token).await {
            Ok(s) => s,
            Err(e) => return on_ui(move |ui| server_failed(ui, format!("Couldn't list your servers: {e}"))),
        };
        let mut ordered = servers.clone();
        ordered.sort_by_key(|r| (Some(&r.client_identifier) != prefer.as_ref(), !r.owned));
        if ordered.is_empty() {
            return on_ui(|ui| server_failed(ui, "This Plex account has no media servers.".into()));
        }
        for r in &ordered {
            if let Ok(server) = plex.connect(r).await {
                let saved = SavedServer {
                    name: server.name().to_string(),
                    client_identifier: r.client_identifier.clone(),
                    base_url: server.base_url().to_string(),
                    token: server.token().to_string(),
                };
                return on_ui(move |_| {
                    with_state(|s| {
                        s.session.server = Some(saved);
                        s.session.save();
                        s.server = Some(server);
                    });
                    enter_home();
                });
            }
        }
        on_ui(|ui| server_failed(ui, "Found your server, but couldn't reach it from this network.".into()));
    });
}

fn server_failed(ui: &AppWindow, msg: String) {
    if ui.get_screen() == "signin" {
        ui.set_signin_status(msg.into());
    } else {
        toast(ui, msg);
    }
}

/// Start up from a saved session: try the last address first, rediscover if it moved.
fn resume_session() {
    let (plex, token, saved, skip) = with_state(|s| (plex(&s.session), s.session.token.clone(), s.session.server.clone(), s.session.skip_plex));
    let Some(token) = token else {
        // Without Plex: straight home if the user chose that (or has other sources).
        let others = with_state(|s| !s.session.media_servers.is_empty() || !s.session.sources.is_empty());
        return if skip || others { enter_home() } else { begin_signin() };
    };
    profiles::at_start();
    let Some(saved) = saved else { return choose_server(token, None) };
    let server = plex.server_at(&saved.name, &saved.base_url, &saved.token);
    ui().set_screen("home".into());
    ui().set_loading(true);
    rt().spawn(async move {
        match server.libraries().await {
            Ok(_) => on_ui(move |_| {
                with_state(|s| s.server = Some(server));
                enter_home();
            }),
            Err(encheo_plex::Error::Status { status: 401, .. }) => on_ui(|_| {
                with_state(|s| s.session.sign_out());
                begin_signin();
            }),
            Err(_) => on_ui(move |_| choose_server(token, Some(saved.client_identifier))),
        }
    });
}

// ---------- Home and libraries ----------

fn server() -> Option<Server> {
    with_state(|s| s.server.clone())
}

/// The account's playback preferences (subtitle language), once per session.
fn load_profile() {
    let (plex, token, loaded) = with_state(|s| (plex(&s.session), s.session.token.clone(), s.subtitle_language.is_some()));
    let Some(token) = token.filter(|_| !loaded) else { return };
    rt().spawn(async move {
        if let Ok(account) = plex.account(&token).await {
            let lang = account.profile.and_then(|p| p.default_subtitle_language).filter(|l| !l.is_empty());
            on_ui(move |_| with_state(|s| s.subtitle_language = lang.map(|l| langs::normalize(&l))));
        }
    });
}

fn enter_home() {
    load_profile();
    with_state(|s| s.share_view = None);
    let ui = ui();
    let backends = all_backends();
    ui.set_screen("home".into());
    ui.set_selected_library(-1);
    ui.set_loading(true);
    let generation = with_state(|s| {
        s.generation += 1;
        s.generation
    });
    rt().spawn(async move {
        // Per server: its libraries (with their recently added items) and what's in progress.
        let several = backends.len() > 1;
        let mut libs: Vec<(String, encheo_plex::Library, Vec<Item>)> = Vec::new();
        // Each library's server and where its Recently Added row pages for more.
        let mut lib_more: Vec<(backend::Backend, Option<String>)> = Vec::new();
        let mut on_deck = Vec::new();
        let mut collections = Vec::new();
        // The servers' suggested rows (Top Rated, More in <genre>, …), by library index.
        let mut discover: Vec<(usize, backend::Rows)> = Vec::new();
        let mut errors = Vec::new();
        let watchlist = match with_state(|s| (plex(&s.session), s.session.token.clone())) {
            (plex, Some(token)) => plex.watchlist(&token).await.unwrap_or_default(),
            _ => vec![],
        };
        for b in &backends {
            match b.libraries().await {
                Ok(list) => {
                    collections.extend(b.collections(&list).await);
                    for l in list {
                        let recent = b.recently_added(&l.key, b.recent_size()).await.unwrap_or_default();
                        if matches!(l.kind.as_str(), "movie" | "show") {
                            discover.push((libs.len(), b.discover(&l.key, &l.kind).await));
                        }
                        lib_more.push((b.clone(), b.recent_more(&l.key).filter(|_| recent.len() as u32 >= b.recent_size())));
                        libs.push((b.label(), l, recent));
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", b.label())),
            }
            on_deck.extend(b.on_deck().await.unwrap_or_default());
        }
        on_ui(move |ui| {
            ui.set_loading(false);
            if !errors.is_empty() {
                toast(ui, format!("Couldn't load {}", errors.join("; ")));
            }
            with_state(|s| {
                s.library_keys = libs.iter().map(|(_, l, _)| l.key.clone()).collect();
                s.library_kinds = libs.iter().map(|(_, l, _)| l.kind.clone()).collect();
            });
            with_models(|m| {
                m.libraries.set_vec(
                    libs.iter()
                        .map(|(group, l, _)| LibraryEntry {
                            key: l.key.clone().into(),
                            title: l.title.clone().into(),
                            kind: l.kind.clone().into(),
                            group: group.clone().into(),
                        })
                        .collect::<Vec<_>>(),
                )
            });
            shares::refresh_sidebar();
            if with_state(|s| s.generation) != generation {
                return;
            }
            // Files from shares first (most recent), then the servers' in-progress items.
            let (file_posters, file_items): (Vec<Poster>, Vec<Item>) = shares::continue_watching().into_iter().unzip();
            remember(file_items);
            let cw: Vec<Poster> = file_posters.into_iter().chain(on_deck.iter().map(|i| poster(i, true))).collect();
            // TV: wide cards in every row, like Netflix (music stays square).
            let wide = ui.global::<Theme>().get_tv();
            let mut rows = Vec::new();
            let mut row_models = Vec::new();
            let mut row_more: Vec<RowMore> = Vec::new();
            let none = || RowMore::default();
            // Watchlist entries live on plex.tv; their ids say so, and opening one finds it on the server.
            let watch: Vec<Item> = watchlist
                .into_iter()
                .filter_map(|mut i| {
                    i.rating_key = format!("watch:{}", i.guid.clone()?);
                    Some(i)
                })
                .collect();
            let favs = favourite_posters();
            if !favs.is_empty() {
                let model = Rc::new(VecModel::from(favs));
                rows.push(HomeRow { title: "Favourites".into(), library: -1, landscape: false, square: false, items: ModelRc::from(model.clone()) });
                row_models.push(model);
                row_more.push(none());
            }
            if !watch.is_empty() {
                let model = Rc::new(VecModel::from(watch.iter().map(|i| poster(i, wide)).collect::<Vec<_>>()));
                rows.push(HomeRow { title: "Watchlist".into(), library: -1, landscape: wide, square: false, items: ModelRc::from(model.clone()) });
                row_models.push(model);
                row_more.push(none());
                remember(watch);
            }
            for (index, (group, l, items)) in libs.iter().enumerate() {
                if items.is_empty() {
                    continue;
                }
                let square = l.kind == "artist";
                let land = wide && !square;
                let model = Rc::new(VecModel::from(group_recent(items).iter().map(|r| recent_poster(r, land)).collect::<Vec<_>>()));
                let title = if several { format!("Recently Added in {} · {group}", l.title) } else { format!("Recently Added in {}", l.title) };
                rows.push(HomeRow {
                    title: title.into(),
                    library: index as i32,
                    landscape: land,
                    square,
                    items: ModelRc::from(model.clone()),
                });
                row_models.push(model);
                let (b, path) = lib_more[index].clone();
                row_more.push(RowMore::new(b, path, items.len(), true, land, generation));
            }
            // The suggested rows, taking turns between libraries so films and TV mix.
            let longest = discover.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
            let mut suggested = Vec::new();
            for n in 0..longest {
                for (index, lib_rows) in &discover {
                    if let Some(encheo_plex::Row { title, items, more }) = lib_rows.get(n) {
                        // Episode rows (Recently Released Episodes) are wide everywhere.
                        let land = wide || items.iter().all(|i| i.kind == ItemKind::Episode);
                        let title = if several { format!("{title} · {}", libs[*index].0) } else { title.clone() };
                        let model = Rc::new(VecModel::from(items.iter().map(|i| poster(i, land)).collect::<Vec<_>>()));
                        rows.push(HomeRow { title: title.into(), library: -1, landscape: land, square: false, items: ModelRc::from(model.clone()) });
                        row_models.push(model);
                        row_more.push(RowMore::new(lib_more[*index].0.clone(), more.clone(), items.len(), false, land, generation));
                        suggested.extend(items.iter().cloned());
                    }
                }
            }
            remember(suggested);
            if !collections.is_empty() {
                let model = Rc::new(VecModel::from(collections.iter().map(|i| poster(i, wide)).collect::<Vec<_>>()));
                rows.push(HomeRow { title: "Collections".into(), library: -1, landscape: wide, square: false, items: ModelRc::from(model.clone()) });
                row_models.push(model);
                row_more.push(none());
                remember(collections);
            }
            let grouped: Vec<Item> = libs.iter().flat_map(|(_, _, items)| group_recent(items).into_iter().map(|(i, _)| i)).collect();
            set_billboard(ui, pick_billboard(&libs, &grouped, &on_deck));
            remember(on_deck.into_iter().chain(libs.into_iter().flat_map(|(_, _, items)| items)).chain(grouped));
            with_models(|m| {
                m.continue_watching.set_vec(cw);
                *m.row_models.borrow_mut() = row_models;
                *m.row_more.borrow_mut() = row_more;
                m.home_rows.set_vec(rows);
            });
        });
    });
}

/// Rows stop growing here: past this, See All (the library) is the better way to browse.
const ROW_MAX: usize = 40;
const ROW_PAGE: u32 = 20;

/// A Home row's next page: its server, the path that pages, and how far it got.
#[derive(Default)]
struct RowMore {
    backend: Option<backend::Backend>,
    path: Option<String>,
    /// Titles fetched so far (before grouping), the next page's start.
    loaded: usize,
    busy: bool,
    /// Recently Added: episodes grouped under their show, like the first page.
    grouped: bool,
    landscape: bool,
    /// Home's generation the row belongs to; a reload replaces the rows.
    generation: u64,
}

impl RowMore {
    fn new(b: backend::Backend, path: Option<String>, loaded: usize, grouped: bool, landscape: bool, generation: u64) -> Self {
        Self { backend: Some(b), path, loaded, busy: false, grouped, landscape, generation }
    }
}

/// Whether Home row `ri` can still grow (more titles to fetch, or some on their way).
fn row_has_more(ri: usize) -> bool {
    with_models(|m| m.row_more.borrow().get(ri).is_some_and(|r| r.busy || (r.path.is_some() && r.loaded < ROW_MAX)))
}

/// Home row `ri` was scrolled near its end: fetch its next titles and add them.
fn more_in_row(ri: usize) {
    let job = with_models(|m| {
        let mut all = m.row_more.borrow_mut();
        let r = all.get_mut(ri)?;
        if r.busy || r.loaded >= ROW_MAX {
            return None;
        }
        let (b, path) = (r.backend.clone()?, r.path.clone()?);
        r.busy = true;
        Some((b, path, r.loaded, r.generation))
    });
    let Some((b, path, start, generation)) = job else { return };
    rt().spawn(async move {
        let page = b.row_page(&path, start as u32, ROW_PAGE).await;
        on_ui(move |_| {
            let fresh = with_models(|m| {
                let mut all = m.row_more.borrow_mut();
                let r = all.get_mut(ri).filter(|r| r.generation == generation)?;
                r.busy = false;
                let items = match page {
                    Ok(items) => items,
                    Err(e) => {
                        eprintln!("more titles for a row: {e}");
                        r.path = None;
                        return None;
                    }
                };
                r.loaded += items.len();
                if items.len() < ROW_PAGE as usize || r.loaded >= ROW_MAX {
                    r.path = None;
                }
                let model = m.row_models.borrow().get(ri).cloned()?;
                let have: std::collections::HashSet<SharedString> = model.iter().map(|p| p.id).collect();
                let grouped = if r.grouped { group_recent(&items) } else { items.iter().map(|i| (i.clone(), 1)).collect() };
                for g in &grouped {
                    let p = if r.grouped { recent_poster(g, r.landscape) } else { poster(&g.0, r.landscape) };
                    if !have.contains(&p.id) {
                        model.push(p);
                    }
                }
                Some(items.into_iter().chain(grouped.into_iter().map(|(i, _)| i)).collect::<Vec<_>>())
            });
            if let Some(items) = fresh {
                remember(items);
            }
        });
    });
}

/// Home's big title: the newest film not yet watched, else the newest show, else what's in
/// progress. Only titles with a backdrop; a billboard without a picture looks broken.
fn pick_billboard(libs: &[(String, encheo_plex::Library, Vec<Item>)], grouped: &[Item], on_deck: &[Item]) -> Option<Item> {
    let has_art = |i: &&Item| i.art.as_deref().is_some_and(|a| !a.is_empty());
    let films = libs.iter().filter(|(_, l, _)| l.kind == "movie").flat_map(|(_, _, items)| items);
    let film = films.clone().filter(has_art).find(|i| !i.is_watched()).or_else(|| films.clone().find(has_art));
    film.or_else(|| grouped.iter().filter(|i| i.kind == ItemKind::Show).find(has_art))
        .or_else(|| on_deck.iter().find(|i| i.kind == ItemKind::Movie && has_art(i)))
        .cloned()
}

fn set_billboard(ui: &AppWindow, item: Option<Item>) {
    let Some(item) = item else {
        with_state(|s| s.billboard = None);
        return ui.set_billboard(Billboard::default());
    };
    let mut meta = Vec::new();
    if let Some(y) = item.year {
        meta.push(y.to_string());
    }
    let genres: Vec<&str> = item.genres.iter().take(2).map(|g| g.tag.as_str()).collect();
    if !genres.is_empty() {
        meta.push(genres.join(", "));
    }
    match item.kind {
        ItemKind::Movie => meta.extend(item.duration.map(runtime)),
        ItemKind::Show => meta.extend(item.child_count.map(|n| format!("{n} season{}", if n == 1 { "" } else { "s" }))),
        ItemKind::Collection => meta.extend(item.child_count.map(|n| format!("Collection of {n}"))),
        ItemKind::Album => meta.splice(0..0, item.parent_title.clone()).for_each(drop),
        _ => {}
    }
    // Titles without a backdrop (collections, albums): their poster, filling the space.
    let art_url = match backdrop_url(&item) {
        b if b.is_empty() => art_url(item.thumb.as_deref().or(item.composite.as_deref()), 1920, 1080),
        b => b,
    };
    let playable = matches!(item.kind, ItemKind::Movie | ItemKind::Show | ItemKind::Episode);
    let logo_url = art_url_for_logo(&item);
    if item.kind == ItemKind::Episode {
        meta.insert(0, format!("{} · {}", item.episode_label().unwrap_or_default(), item.title));
    }
    let title = if item.kind == ItemKind::Episode { item.grandparent_title.clone().unwrap_or_else(|| item.title.clone()) } else { item.title.clone() };
    // More Info opens this id: an episode's show (opening an episode would play it).
    let id = if item.kind == ItemKind::Episode { item.grandparent_rating_key.clone().unwrap_or_else(|| item.rating_key.clone()) } else { item.rating_key.clone() };
    let b = Billboard {
        id: id.into(),
        title: title.into(),
        meta: meta.join("  ·  ").into(),
        summary: short_summary(item.summary.as_deref().unwrap_or_default(), 200).into(),
        image: want_image(&art_url),
        art_url: art_url.into(),
        logo: want_image(&logo_url),
        logo_url: logo_url.into(),
        play_label: if !playable {
            "Open"
        } else if item.view_offset.is_some_and(|o| o > 0) {
            "Resume"
        } else {
            "Play"
        }
        .into(),
    };
    with_state(|s| s.billboard = Some(item));
    ui.set_billboard(b);
}

/// TV Home: the billboard shows the film or show the remote is on, once the focus has rested
/// there for a tick (so scrolling past titles doesn't fetch every backdrop).
fn tv_hero_follow(ui: &AppWindow) {
    let nav = ui.global::<Nav>();
    if !ui.global::<Theme>().get_tv()
        || ui.get_screen() != "home"
        || ui.get_selected_library() != -1
        || ui.get_billboard().id.is_empty()
        || !nav.get_active()
        || nav.get_zone() != "content"
        || nav.get_row() == 0
    {
        return;
    }
    let Some(id) = focused_home_id(ui, nav.get_row(), nav.get_col()) else { return };
    let rested = with_state(|s| {
        let same = s.hero_candidate.as_deref() == Some(id.as_str());
        s.hero_candidate = Some(id.to_string());
        same
    });
    let current = with_state(|s| s.billboard.as_ref().map(|b| b.rating_key.clone()));
    if !rested || current.as_deref() == Some(id.as_str()) {
        return;
    }
    let item = with_state(|s| s.items.get(id.as_str()).cloned());
    if let Some(item) = item {
        set_billboard(ui, Some(item));
    }
}

/// The id of the card at this remote position on Home (None on the billboard row).
fn focused_home_id(ui: &AppWindow, row: i32, col: i32) -> Option<SharedString> {
    let mut r = row;
    if !ui.get_billboard().id.is_empty() {
        if r == 0 {
            return None;
        }
        r -= 1;
    }
    with_models(|m| {
        if m.continue_watching.row_count() > 0 {
            if r == 0 {
                return m.continue_watching.row_data(col as usize).map(|p| p.id);
            }
            r -= 1;
        }
        m.home_rows.row_data(r as usize).and_then(|hr| hr.items.row_data(col as usize)).map(|p| p.id)
    })
}

fn art_url_for_logo(item: &Item) -> String {
    item.logo.as_deref().filter(|l| !l.is_empty()).map(|l| art_url(Some(l), 800, 300)).unwrap_or_default()
}

/// The billboard's Play: a film plays (resuming); a show plays its next episode.
fn play_billboard() {
    let Some(item) = with_state(|s| s.billboard.clone()) else { return };
    if !matches!(item.kind, ItemKind::Movie | ItemKind::Show | ItemKind::Episode) {
        return open_item(item.rating_key);
    }
    if item.kind != ItemKind::Show {
        return play(item);
    }
    let Some(server) = backend_for(&item.rating_key) else { return };
    rt().spawn(async move {
        let eps = server.all_episodes(&item.rating_key).await.unwrap_or_default();
        let next = eps
            .iter()
            .find(|e| e.progress().is_some())
            .or_else(|| eps.iter().find(|e| !e.is_watched()))
            .or(eps.first())
            .cloned();
        on_ui(move |_| match next {
            Some(e) => play(e),
            None => open_item(item.rating_key),
        });
    });
}

/// Recently added, one card per show (and per album): episodes of the same show collapse into the
/// show, in the order the newest one arrived. Each entry carries how many new items it stands for.
fn group_recent(items: &[Item]) -> Vec<(Item, usize)> {
    let mut out: Vec<(Item, usize, Option<Item>)> = Vec::new();
    for item in items {
        let parent = match item.kind {
            ItemKind::Episode => item.grandparent_rating_key.clone().map(|k| (k, ItemKind::Show)),
            ItemKind::Track => item.parent_rating_key.clone().map(|k| (k, ItemKind::Album)),
            _ => None,
        };
        let Some((key, kind)) = parent else {
            out.push((item.clone(), 1, None));
            continue;
        };
        if let Some(entry) = out.iter_mut().find(|(i, _, _)| i.rating_key == key) {
            entry.1 += 1;
            continue;
        }
        let group = Item {
            rating_key: key,
            kind: kind.clone(),
            title: if kind == ItemKind::Show { item.grandparent_title.clone() } else { item.parent_title.clone() }.unwrap_or_default(),
            thumb: if kind == ItemKind::Show { item.grandparent_thumb.clone() } else { item.parent_thumb.clone() }.or_else(|| item.thumb.clone()),
            art: item.grandparent_art.clone().or_else(|| item.art.clone()),
            parent_title: if kind == ItemKind::Album { item.grandparent_title.clone() } else { None },
            ..Default::default()
        };
        out.push((group, 1, Some(item.clone())));
    }
    out.into_iter()
        .map(|(mut group, n, first)| {
            // A single new episode keeps its label for the subtitle.
            if n == 1 && let Some(ep) = first.filter(|e| e.kind == ItemKind::Episode) {
                group.summary = ep.episode_label().map(|l| format!("{l} new"));
            }
            (group, n)
        })
        .collect()
}

fn recent_poster((item, count): &(Item, usize), landscape: bool) -> Poster {
    let mut p = poster(item, landscape);
    match item.kind {
        ItemKind::Show if *count > 1 => p.subtitle = format!("{count} new episodes").into(),
        ItemKind::Show => p.subtitle = item.summary.clone().unwrap_or_else(|| "New episode".into()).into(),
        ItemKind::Album if *count > 1 => p.subtitle = format!("{count} new tracks").into(),
        _ => {}
    }
    p
}

fn select_library(index: i32) {
    let ui = ui();
    ui.set_selected_library(index);
    if ui.global::<Nav>().get_zone() == "content" {
        set_focus(&ui, "content", 0, 0);
    }
    if index < 0 {
        return enter_home();
    }
    let share = with_state(|s| {
        (s.library_kinds.get(index as usize).map(String::as_str) == Some("folder"))
            .then(|| s.library_keys.get(index as usize).and_then(|k| k.strip_prefix("share:")?.parse::<usize>().ok()))
            .flatten()
    });
    if let Some(si) = share {
        return shares::browse(si, String::new());
    }
    with_state(|s| s.share_view = None);
    let (key, kind, generation) = with_state(|s| {
        s.generation += 1;
        (s.library_keys.get(index as usize).cloned(), s.library_kinds.get(index as usize).cloned(), s.generation)
    });
    let Some(key) = key else { return };
    let Some(server) = backend_for(&key) else { return };
    let kind_str = kind.clone().unwrap_or_default();
    // Music libraries have tabs: artists, albums, songs, playlists.
    let music = kind_str == "artist";
    let tab = if music { with_state(|s| s.music_tab) } else { 0 };
    let tabs: Vec<SharedString> = if music { MUSIC_TABS.iter().map(|t| SharedString::from(*t)).collect() } else { vec![] };
    ui.set_grid_tabs(ModelRc::new(VecModel::from(tabs)));
    ui.set_grid_tab(tab);
    with_state(|s| s.songs.clear());
    with_models(|m| m.songs.set_vec(Vec::<Track>::new()));
    ui.set_grid_square(music);
    let title = with_models(|m| m.libraries.row_data(index as usize).map(|l| l.title)).unwrap_or_default();
    ui.set_grid_title(title);
    ui.set_loading(true);
    with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    rt().spawn(async move {
        let mut start = 0;
        loop {
            let page = if music {
                server.music_items(&key, tab, start, PAGE).await
            } else {
                server.library_items(&key, &kind_str, start, PAGE).await
            };
            let done = match &page {
                Ok(p) => p.items.len() < PAGE as usize || p.total.is_some_and(|t| start + PAGE >= t),
                Err(_) => true,
            };
            let stale = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stale2 = stale.clone();
            on_ui(move |ui| {
                if with_state(|s| s.generation) != generation {
                    stale2.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                match page {
                    Ok(p) if music && tab == SONGS_TAB => {
                        // A list, not posters; the grid only keeps the ids, for the remote.
                        let base = with_state(|s| {
                            let n = s.songs.len();
                            s.songs.extend(p.items.iter().cloned());
                            n
                        });
                        let rows: Vec<Track> = p.items.iter().enumerate().map(|(i, t)| song_row(t, base + i)).collect();
                        let ids: Vec<Poster> = p.items.iter().map(|t| Poster { id: t.rating_key.clone().into(), ..Default::default() }).collect();
                        remember(p.items);
                        with_models(|m| {
                            rows.into_iter().for_each(|r| m.songs.push(r));
                            ids.into_iter().for_each(|x| m.grid.push(x));
                        });
                    }
                    Ok(p) => {
                        let posters: Vec<Poster> = p.items.iter().map(|i| poster(i, false)).collect();
                        remember(p.items);
                        with_models(|m| posters.into_iter().for_each(|x| m.grid.push(x)));
                    }
                    Err(e) => toast(ui, format!("Couldn't load the library: {e}")),
                }
                if done {
                    ui.set_loading(false);
                }
            });
            if done || stale.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            start += PAGE;
        }
    });
}

fn remember(items: impl IntoIterator<Item = Item>) {
    with_state(|s| {
        for i in items {
            // Keep the richer copy (with media/markers) if we already have one.
            match s.items.get(&i.rating_key) {
                Some(old) if !old.media.is_empty() && i.media.is_empty() => {}
                _ => {
                    s.items.insert(i.rating_key.clone(), i);
                }
            }
        }
    });
}

// ---------- Posters and images ----------

fn poster(item: &Item, landscape: bool) -> Poster {
    let is_ep = item.kind == ItemKind::Episode;
    let title = if is_ep { item.grandparent_title.clone().unwrap_or_default() } else { item.title.clone() };
    let square = matches!(item.kind, ItemKind::Artist | ItemKind::Album | ItemKind::Track | ItemKind::Playlist);
    let subtitle = if is_ep {
        format!("{} · {}", item.episode_label().unwrap_or_default(), item.title)
    } else if item.kind == ItemKind::Album {
        item.parent_title.clone().unwrap_or_default()
    } else if item.kind == ItemKind::Artist {
        String::new()
    } else {
        match (item.kind.clone(), item.year, item.leaf_count) {
            (ItemKind::Show, _, Some(n)) => format!("{n} episodes"),
            (ItemKind::Playlist, _, _) => match item.leaf_count.or(item.child_count) {
                Some(1) => "1 song".into(),
                Some(n) => format!("{n} songs"),
                None => "Playlist".into(),
            },
            (ItemKind::Collection, _, _) => match item.child_count {
                Some(1) => "1 title".into(),
                Some(n) => format!("{n} titles"),
                None => "Collection".into(),
            },
            (_, Some(y), _) => y.to_string(),
            _ => String::new(),
        }
    };
    let path = if landscape {
        item.art.clone().or_else(|| item.grandparent_art.clone()).or_else(|| item.thumb.clone())
    } else if is_ep {
        item.grandparent_thumb.clone().or_else(|| item.thumb.clone())
    } else {
        item.thumb.clone().or_else(|| item.composite.clone())
    };
    let (w, h) = if landscape {
        (560, 316)
    } else if square {
        (400, 400)
    } else {
        (320, 480)
    };
    let url = art_url(path.as_deref(), w, h);
    Poster {
        id: item.rating_key.clone().into(),
        title: title.into(),
        subtitle: subtitle.into(),
        image: Image::default(),
        art_url: url.into(),
        progress: item.progress().unwrap_or(0.0),
        watched: item.is_watched() && !square,
        square,
    }
}

fn art_url(path: Option<&str>, w: u32, h: u32) -> String {
    match path {
        // Jellyfin/Emby give full image URLs; ask for the size we need.
        Some(p) if p.contains("/Items/") => format!("{p}&maxWidth={w}&maxHeight={h}"),
        // Plex's own pictures on plex.tv (cast and crew photos, watchlist art) are full size,
        // up to 2000 x 2000 (16 MB each once decoded): through the server's resizer, as Plex's
        // own apps do. Without a Plex server, as they are.
        Some(p) if p.starts_with("http") && plex_hosted(p) => server().map(|s| s.image_url(p, w, h)).unwrap_or_else(|| p.to_string()),
        // Other full URLs as they are.
        Some(p) if p.starts_with("http") => p.to_string(),
        Some(p) if !p.is_empty() => server().map(|s| s.image_url(p, w, h)).unwrap_or_default(),
        _ => String::new(),
    }
}

/// A picture on plex.tv's servers (metadata-static.plex.tv and the like).
fn plex_hosted(url: &str) -> bool {
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("").to_ascii_lowercase();
    host == "plex.tv" || host.ends_with(".plex.tv")
}

/// The image if cached; otherwise start loading it and return an empty image for now.
fn want_image(url: &str) -> Image {
    if url.is_empty() {
        return Image::default();
    }
    let start = with_state(|s| {
        if let Some(i) = s.images.get(url) {
            return Err(i);
        }
        Ok(s.pending_images.insert(url.to_string()))
    });
    match start {
        Err(img) => img,
        Ok(false) => Image::default(),
        Ok(true) => {
            let url = url.to_string();
            let loader = IMAGES.get_or_init(images::Loader::new).clone();
            rt().spawn(async move {
                let buf = loader.fetch(&url).await;
                on_ui(move |ui| {
                    let img = buf.map(images::to_image).unwrap_or_default();
                    with_state(|s| {
                        s.pending_images.remove(&url);
                        s.images.insert(url.clone(), img.clone());
                    });
                    apply_image(ui, &url, &img);
                    let art = ui.global::<Art>();
                    art.set_loaded(art.get_loaded().wrapping_add(1));
                });
            });
            Image::default()
        }
    }
}

fn apply_image(ui: &AppWindow, url: &str, img: &Image) {
    // (Posters aren't here: each card asks Art for its picture while it's near the screen.)
    with_models(|m| {
        for i in 0..m.people.row_count() {
            if let Some(mut p) = m.people.row_data(i)
                && p.art_url == url
            {
                p.image = img.clone();
                m.people.set_row_data(i, p);
            }
        }
        for i in 0..m.episodes.row_count() {
            if let Some(mut e) = m.episodes.row_data(i)
                && e.art_url == url
            {
                e.image = img.clone();
                m.episodes.set_row_data(i, e);
            }
        }
    });
    let mut d = ui.get_detail();
    if d.art_url == url {
        d.image = img.clone();
        ui.set_detail(d);
    }
    if with_state(|s| s.music.as_ref().is_some_and(|m| m.art_url == url)) {
        ui.set_music_art(img.clone());
    }
    if with_state(|s| s.music.as_ref().is_some_and(|m| m.art_large_url == url)) {
        ui.set_music_art_large(img.clone());
    }
    if with_state(|s| s.detail_backdrop_url == url) {
        ui.set_detail_backdrop(img.clone());
    }
    let mut b = ui.get_billboard();
    if b.art_url == url || b.logo_url == url {
        if b.art_url == url {
            b.image = img.clone();
        }
        if b.logo_url == url {
            b.logo = img.clone();
        }
        ui.set_billboard(b);
    }
}

fn backdrop_url(item: &Item) -> String {
    art_url(item.art.as_deref().or(item.grandparent_art.as_deref()), 1920, 1080)
}

// ---------- Details ----------

fn open_item(id: String) {
    if shares::open(&id) {
        return;
    }
    if let Some(guid) = id.strip_prefix("watch:") {
        return open_watchlist_item(guid.to_string());
    }
    let Some(item) = with_state(|s| s.items.get(&id).cloned()) else {
        // Not loaded this session (a favourite after a restart): ask its server.
        let Some(server) = backend_for(&id) else { return };
        rt().spawn(async move {
            let found = server.metadata(&id).await;
            on_ui(move |ui| match found {
                Ok(Some(item)) => {
                    remember([item.clone()]);
                    show_detail(item);
                }
                Ok(None) => toast(ui, "It's no longer on the server."),
                Err(e) => toast(ui, format!("Couldn't open it: {e}")),
            });
        });
        return;
    };
    // Episodes and part-watched movies from "Continue Watching" go straight to the player.
    if item.kind == ItemKind::Episode {
        return play(item);
    }
    if item.kind == ItemKind::Track {
        if ui().get_screen() == "home" && with_state(|s| s.songs.iter().any(|t| t.rating_key == item.rating_key)) {
            return play_song(&item.rating_key);
        }
        return play_music(vec![item], 0);
    }
    if item.kind == ItemKind::Collection {
        return open_collection(item);
    }
    show_detail(item);
}

const MUSIC_TABS: [&str; 4] = ["Artists", "Albums", "Songs", "Playlists"];
const SONGS_TAB: i32 = 2;

/// A music library's tab.
fn select_tab(tab: i32) {
    with_state(|s| s.music_tab = tab);
    select_library(ui().get_selected_library());
}

/// One line of the Songs tab.
fn song_row(t: &Item, position: usize) -> Track {
    let artist = t.original_title.clone().or_else(|| t.grandparent_title.clone()).unwrap_or_default();
    let album = t.parent_title.clone().unwrap_or_default();
    let playing = with_state(|s| s.music.as_ref().and_then(|q| q.items.get(q.index)).is_some_and(|p| p.rating_key == t.rating_key));
    Track {
        id: t.rating_key.clone().into(),
        number: (position + 1).to_string().into(),
        title: t.title.clone().into(),
        artist: [artist, album].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ").into(),
        runtime: t.duration.map(|d| clock(d as f64 / 1000.0)).unwrap_or_default().into(),
        playing,
    }
}

/// Play the Songs tab from this song on (a few hundred at a time, so the queue stays quick).
fn play_song(id: &str) {
    let (songs, idx) = with_state(|s| (s.songs.clone(), s.songs.iter().position(|t| t.rating_key == id)));
    if let Some(i) = idx {
        let end = (i + 500).min(songs.len());
        play_music(songs[i..end].to_vec(), 0);
    }
}

/// The page that shows a collection: like a library page (selected library -3).
const COLLECTION_PAGE: i32 = -3;

/// A collection's titles, as a page of posters.
fn open_collection(item: Item) {
    let ui = ui();
    let Some(server) = backend_for(&item.rating_key) else { return };
    let generation = with_state(|s| {
        s.generation += 1;
        s.share_view = None;
        s.collection = Some(item.clone());
        s.generation
    });
    ui.set_screen("home".into());
    ui.set_selected_library(COLLECTION_PAGE);
    ui.set_grid_square(false);
    ui.set_grid_title(item.title.clone().into());
    ui.set_loading(true);
    with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    if ui.global::<Nav>().get_active() {
        set_focus(&ui, "content", 0, 0);
    }
    let id = item.rating_key.clone();
    rt().spawn(async move {
        let found = server.collection_items(&id).await;
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            ui.set_loading(false);
            match found {
                Ok(items) => {
                    let posters: Vec<Poster> = items.iter().map(|i| poster(i, false)).collect();
                    remember(items);
                    with_models(|m| m.grid.set_vec(posters));
                }
                Err(e) => toast(ui, format!("Couldn't load the collection: {e}")),
            }
        });
    });
}

/// Load the current library page again (a library, search results or a collection).
fn reload_page(selected: i32) {
    match selected {
        -2 => search(ui().get_search_text().to_string()),
        COLLECTION_PAGE => match with_state(|s| s.collection.clone()) {
            Some(c) => open_collection(c),
            None => enter_home(),
        },
        i => select_library(i),
    }
}

/// A watchlist title: show it from the server if the server has it.
fn open_watchlist_item(guid: String) {
    let Some(server) = server() else { return };
    rt().spawn(async move {
        let found = server.find_by_guid(&guid).await.ok().flatten();
        on_ui(move |ui| match found {
            Some(item) => {
                remember([item.clone()]);
                show_detail(item);
            }
            None => toast(ui, "Not on your server yet."),
        });
    });
}

fn show_detail(item: Item) {
    let ui = ui();
    {
        let nav = ui.global::<Nav>();
        if ui.get_screen() == "home" {
            with_state(|s| s.home_nav = Some((nav.get_zone(), nav.get_row(), nav.get_col())));
        }
        // Coming back from the player to the same title keeps the focus where it was.
        let same = with_state(|s| s.detail.as_ref().is_some_and(|d| d.rating_key == item.rating_key));
        if !(same && nav.get_zone() == "detail") {
            set_focus(&ui, "detail", 0, 0);
        }
    }
    with_state(|s| {
        s.generation += 1;
        s.detail = Some(item.clone());
        s.next_up = None;
        s.season_items.clear();
    });
    ui.set_detail(poster(&item, false));
    let backdrop = backdrop_url(&item);
    with_state(|s| s.detail_backdrop_url = backdrop.clone());
    ui.set_detail_backdrop(want_image(&backdrop));
    fill_detail_text(&ui, &item);
    ui.set_detail_is_show(item.kind == ItemKind::Show);
    ui.set_detail_kind(
        match item.kind {
            ItemKind::Show => "show",
            ItemKind::Artist => "artist",
            ItemKind::Album => "album",
            ItemKind::Playlist => "playlist",
            _ => "movie",
        }
        .into(),
    );
    ui.set_play_label(match item.kind {
        ItemKind::Show => SharedString::new(),
        ItemKind::Album | ItemKind::Artist | ItemKind::Playlist => "Play".into(),
        _ => play_label(&item).into(),
    });
    ui.set_detail_can_shuffle(matches!(item.kind, ItemKind::Album | ItemKind::Artist | ItemKind::Playlist));
    with_models(|m| {
        m.seasons.set_vec(Vec::<SharedString>::new());
        m.seasons_watched.set_vec(Vec::<bool>::new());
        m.episodes.set_vec(Vec::<Episode>::new());
        m.albums.set_vec(Vec::<Poster>::new());
        m.tracks.set_vec(Vec::<Track>::new());
        m.people.set_vec(Vec::<Person>::new());
        m.related.set_vec(Vec::<Poster>::new());
    });
    with_state(|s| {
        s.album_tracks.clear();
        s.artist_albums.clear();
        s.trailer = None;
    });
    ui.set_detail_can_trailer(false);
    ui.set_screen("detail".into());
    // Where each section starts, for the remote focus (the old page's would be stale).
    detail_rows(&ui);
    if matches!(item.kind, ItemKind::Movie | ItemKind::Show) {
        find_trailer(&item.rating_key);
    }

    let Some(server) = backend_for(&item.rating_key) else { return };
    let generation = with_state(|s| s.generation);
    let id = item.rating_key.clone();
    let is_show = item.kind == ItemKind::Show;
    let kind = item.kind.clone();
    rt().spawn(async move {
        let full = server.metadata(&id).await.ok().flatten();
        let children = match kind {
            ItemKind::Artist | ItemKind::Album => server.children(&id).await.unwrap_or_default(),
            ItemKind::Playlist => server.playlist_items(&id).await.unwrap_or_default(),
            _ => vec![],
        };
        let related = if kind == ItemKind::Playlist { vec![] } else { server.related(&id).await.unwrap_or_default() };
        let seasons = if is_show { server.children(&id).await.unwrap_or_default() } else { vec![] };
        let all_eps = if is_show { server.all_episodes(&id).await.unwrap_or_default() } else { vec![] };
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            match kind {
                ItemKind::Artist => {
                    let albums: Vec<Poster> = children.iter().map(|a| poster(a, false)).collect();
                    with_state(|s| s.artist_albums = children.clone());
                    remember(children);
                    with_models(|m| m.albums.set_vec(albums));
                }
                ItemKind::Album | ItemKind::Playlist => {
                    with_state(|s| s.album_tracks = children.clone());
                    refresh_tracks();
                    remember(children);
                }
                _ => {}
            }
            let related_posters: Vec<Poster> = related.iter().map(|i| poster(i, false)).collect();
            remember(related);
            with_models(|m| m.related.set_vec(related_posters));
            detail_rows(ui);
            if let Some(full) = full {
                fill_detail_text(ui, &full);
                let cast = people(&full);
                with_models(|m| m.people.set_vec(cast));
                if !is_show && !matches!(full.kind, ItemKind::Artist | ItemKind::Album | ItemKind::Playlist) {
                    ui.set_play_label(play_label(&full).into());
                }
                remember([full.clone()]);
                with_state(|s| s.detail = Some(full));
            }
            if is_show {
                // Next up: first episode in progress, else first unwatched, else the first.
                let next = all_eps
                    .iter()
                    .find(|e| e.progress().is_some())
                    .or_else(|| all_eps.iter().find(|e| !e.is_watched()))
                    .or(all_eps.first())
                    .cloned();
                if let Some(n) = &next {
                    let verb = if n.progress().is_some() { "Resume" } else { "Play" };
                    ui.set_play_label(format!("{verb} {}", n.episode_label().unwrap_or_default()).into());
                    ui.set_detail_can_restart(n.progress().is_some());
                }
                let season_of_next = next.as_ref().and_then(|n| n.parent_index);
                let selected = seasons.iter().position(|s| s.index == season_of_next && season_of_next.is_some()).unwrap_or(0);
                remember(all_eps);
                with_state(|s| {
                    s.next_up = next;
                    s.season_items = seasons.clone();
                });
                with_models(|m| {
                    m.seasons.set_vec(seasons.iter().map(|s| SharedString::from(s.title.as_str())).collect::<Vec<_>>());
                    m.seasons_watched.set_vec(seasons.iter().map(Item::is_watched).collect::<Vec<_>>());
                });
                select_season(selected as i32);
            }
        });
    });
}

/// Ask the item's server for a trailer; the Trailer button shows when there is one.
fn find_trailer(id: &str) {
    let Some(server) = backend_for(id) else { return };
    let generation = with_state(|s| s.generation);
    let id = id.to_string();
    rt().spawn(async move {
        let mut trailer = server.trailer(&id).await;
        // Servers rarely have anime trailers; AniList does.
        if trailer.is_none()
            && let Some(full) = server.metadata(&id).await.ok().flatten()
            && anilist::is_anime(&full)
        {
            trailer = anilist::trailer(&full).await.map(backend::Trailer::Link);
        }
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            ui.set_detail_can_trailer(trailer.is_some());
            with_state(|s| s.trailer = trailer);
        });
    });
}

fn play_trailer() {
    match with_state(|s| s.trailer.clone()) {
        Some(backend::Trailer::Play(item)) => play_fresh(*item),
        Some(backend::Trailer::Link(url)) => open_trailer_link(&url),
        None => {}
    }
}

/// An online trailer. Android: YouTube trailers play inside Encheo, in YouTube's embedded
/// player; elsewhere (and for other sites) the browser or the site's app opens it.
fn open_trailer_link(url: &str) {
    #[cfg(target_os = "android")]
    if let (Some(app), Some(id)) = (ANDROID_APP.get(), youtube_id(url)) {
        match android_trailer(app, id) {
            Ok(()) => return,
            Err(e) => eprintln!("trailer player: {e}"),
        }
    }
    open_url(url);
}

/// The video id of a YouTube address (youtube.com/watch?v=…, youtu.be/…, youtube.com/embed/…).
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
fn youtube_id(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r).trim_start_matches("www.").trim_start_matches("m.");
    let id = if let Some(r) = rest.strip_prefix("youtu.be/") {
        r
    } else if let Some(r) = rest.strip_prefix("youtube.com/embed/") {
        r
    } else {
        let query = rest.strip_prefix("youtube.com/watch")?.split_once('?')?.1;
        query.split('&').find_map(|kv| kv.strip_prefix("v="))?
    };
    let id = id.split(['?', '&', '#', '/']).next()?;
    (id.len() >= 6 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(id)
}

/// Android: start Encheo's trailer screen (TrailerActivity) for a YouTube video.
#[cfg(target_os = "android")]
fn android_trailer(app: &slint::android::AndroidApp, id: &str) -> jni::errors::Result<()> {
    use jni::objects::{JObject, JValue};
    // SAFETY: both pointers come from the running activity and stay valid while it runs.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread()?;
    let intent = env.new_object("android/content/Intent", "()V", &[])?;
    let (pkg, class) = (env.new_string("app.encheo")?, env.new_string("app.encheo.TrailerActivity")?);
    env.call_method(&intent, "setClassName", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&pkg), JValue::Object(&class)])?;
    let (key, video) = (env.new_string("video")?, env.new_string(id)?);
    env.call_method(&intent, "putExtra", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&key), JValue::Object(&video)])?;
    let started = env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]);
    if started.is_err() {
        let _ = env.exception_clear();
    }
    started.map(|_| ())
}

/// Open a web address in the system's browser (Android: whatever app handles it, e.g. YouTube).
pub(crate) fn open_url(url: &str) {
    #[cfg(target_os = "android")]
    if let Some(app) = ANDROID_APP.get() {
        if let Err(e) = android_view(app, url) {
            eprintln!("open {url}: {e}");
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = open::that(url);
}

/// Android: start an ACTION_VIEW intent for the URL.
#[cfg(target_os = "android")]
fn android_view(app: &slint::android::AndroidApp, url: &str) -> jni::errors::Result<()> {
    use jni::objects::{JObject, JValue};
    // SAFETY: both pointers come from the running activity and stay valid while it runs.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread()?;
    let jurl = env.new_string(url)?;
    let uri = env
        .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&jurl)])?
        .l()?;
    let action = env.new_string("android.intent.action.VIEW")?;
    let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
    env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?; // FLAG_ACTIVITY_NEW_TASK
    let started = env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]);
    if started.is_err() {
        // No app for it (no browser on some TVs): don't leave a Java exception pending.
        let _ = env.exception_clear();
    }
    started.map(|_| ())
}

fn fill_detail_text(ui: &AppWindow, item: &Item) {
    let mut meta = Vec::new();
    if item.kind == ItemKind::Album
        && let Some(a) = &item.parent_title
    {
        meta.push(a.clone());
    }
    if let Some(d) = item.duration.filter(|_| item.kind == ItemKind::Movie) {
        meta.push(runtime(d));
    }
    if let Some(y) = item.year {
        meta.push(y.to_string());
    }
    if let Some(n) = item.leaf_count.filter(|_| item.kind == ItemKind::Album) {
        meta.push(format!("{n} track{}", if n == 1 { "" } else { "s" }));
    }
    if let Some(n) = item.child_count.filter(|_| item.kind == ItemKind::Show) {
        meta.push(format!("{n} season{}", if n == 1 { "" } else { "s" }));
    }
    let genres: Vec<&str> = item.genres.iter().take(4).map(|g| g.tag.as_str()).collect();
    if !genres.is_empty() {
        meta.push(genres.join(", "));
    }
    ui.set_detail_meta(meta.join("  ·  ").into());
    ui.set_detail_badges(ModelRc::new(VecModel::from(badges(item).into_iter().map(SharedString::from).collect::<Vec<_>>())));

    let full = item.summary.as_deref().unwrap_or_default().trim().to_string();
    ui.set_detail_summary(short_summary(&full, 260).into());
    ui.set_detail_summary_full(full.into());

    let part = item.primary_part();
    ui.set_detail_file(
        part.and_then(|p| p.file.as_deref())
            .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f).to_string())
            .unwrap_or_default()
            .into(),
    );
    ui.set_detail_tech(tech_line(item).into());
    ui.set_scores(ModelRc::new(VecModel::from(scores(item))));

    let playable = matches!(item.kind, ItemKind::Movie | ItemKind::Episode);
    ui.set_detail_can_restart(playable && item.view_offset.is_some_and(|o| o > 0));
    ui.set_detail_can_mark_watched(matches!(item.kind, ItemKind::Movie | ItemKind::Episode | ItemKind::Show | ItemKind::Season));
    ui.set_detail_watched(item.is_watched());
    ui.set_detail_can_favourite(matches!(item.kind, ItemKind::Movie | ItemKind::Show | ItemKind::Artist | ItemKind::Album));
    ui.set_detail_favourite(with_state(|s| s.favourites.get_or_insert_with(favourites::Store::load).contains(&item.rating_key)));
}

fn toggle_favourite() {
    let Some(item) = with_state(|s| s.detail.clone()) else { return };
    let p = poster(&item, false);
    let fav = favourites::Favourite {
        id: item.rating_key.clone(),
        title: p.title.to_string(),
        subtitle: p.subtitle.to_string(),
        thumb: item.thumb.clone(),
        square: p.square,
    };
    let now = with_state(|s| s.favourites.get_or_insert_with(favourites::Store::load).toggle(fav));
    let ui = ui();
    ui.set_detail_favourite(now);
    toast(&ui, if now { "Added to Favourites" } else { "Removed from Favourites" });
}

fn favourite_posters() -> Vec<Poster> {
    let favs = with_state(|s| s.favourites.get_or_insert_with(favourites::Store::load).all().to_vec());
    favs.into_iter()
        .map(|f| {
            let (w, h) = if f.square { (400, 400) } else { (320, 480) };
            let url = art_url(f.thumb.as_deref(), w, h);
            Poster {
                id: f.id.into(),
                title: f.title.into(),
                subtitle: f.subtitle.into(),
                image: Image::default(),
                art_url: url.into(),
                progress: 0.0,
                watched: false,
                square: f.square,
            }
        })
        .collect()
}

/// "2 hrs 6 min".
fn runtime(ms: u64) -> String {
    let m = ms / 60_000;
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (1, m) => format!("1 hr {m} min"),
        (h, m) => format!("{h} hrs {m} min"),
    }
}

/// Small outlined labels after the meta line: content rating, resolution, dynamic range.
fn badges(item: &Item) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(r) = &item.content_rating {
        out.push(r.clone());
    }
    if let Some(m) = item.media.first() {
        match m.video_resolution.as_deref() {
            Some("4k") => out.push("4K".into()),
            Some(r) if r.chars().all(|c| c.is_ascii_digit()) && r.parse::<u32>().unwrap_or(0) >= 720 => out.push("HD".into()),
            _ => {}
        }
        let range = item.primary_part().and_then(|p| p.dynamic_range()).map(String::from).or_else(|| m.video_dynamic_range.clone().filter(|r| r != "SDR"));
        if let Some(r) = range {
            out.push(r);
        }
    }
    out
}

/// Footer: "32.2 GB · HEVC · 4K HDR10 · EAC3 5.1 · 34.2 Mbps · 24 fps".
fn tech_line(item: &Item) -> String {
    let Some(m) = item.media.first() else { return String::new() };
    let part = item.primary_part();
    let res = m.video_resolution.as_ref().map(|r| if r.chars().all(|c| c.is_ascii_digit()) { format!("{r}p") } else { r.to_uppercase() });
    let range = part.and_then(|p| p.dynamic_range());
    [
        part.and_then(|p| p.size).map(|b| format!("{:.1} GB", b as f64 / 1e9)),
        m.video_codec.as_ref().map(|c| c.to_uppercase()),
        match (res, range) {
            (Some(r), Some(d)) => Some(format!("{r} {d}")),
            (r, d) => r.or(d.map(String::from)),
        },
        m.audio_codec.as_ref().map(|a| {
            let ch = m.audio_channels.map(|c| format!(" {}", channels(c))).unwrap_or_default();
            format!("{}{ch}", a.to_uppercase())
        }),
        m.bitrate.map(|b| format!("{:.1} Mbps", b as f64 / 1000.0)),
        m.video_frame_rate.as_deref().map(|f| match f {
            "PAL" => "25 fps".into(),
            "NTSC" => "29.97 fps".into(),
            f => format!("{} fps", f.trim_end_matches('p')),
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ·  ")
}

/// Outside ratings Plex gathered, as small labelled chips.
fn scores(item: &Item) -> Vec<Score> {
    item.ratings
        .iter()
        .filter_map(|r| {
            let (source, value, tint) = match r.image.as_str() {
                i if i.starts_with("imdb://") => ("IMDb", format!("{:.1}", r.value), slint::Color::from_rgb_u8(0xf5, 0xc5, 0x18)),
                i if i.starts_with("rottentomatoes://image.rating.ripe") || i.starts_with("rottentomatoes://image.rating.rotten") => {
                    ("Critics", format!("{:.0}%", r.value * 10.0), slint::Color::from_rgb_u8(0xfa, 0x6b, 0x4a))
                }
                i if i.starts_with("rottentomatoes://") => ("Audience", format!("{:.0}%", r.value * 10.0), slint::Color::from_rgb_u8(0xf7, 0xb0, 0x3c)),
                i if i.starts_with("themoviedb://") => ("TMDB", format!("{:.1}", r.value), slint::Color::from_rgb_u8(0x5c, 0xd3, 0xb4)),
                _ => return None,
            };
            Some(Score { source: source.into(), value: value.into(), tint: tint.into() })
        })
        .collect()
}

/// Director first, then the cast in billing order, then writers and producers.
fn people(item: &Item) -> Vec<Person> {
    let person = |name: &str, role: &str, thumb: Option<&str>| {
        let url = art_url(thumb, 200, 200);
        Person { name: name.into(), role: role.into(), image: want_image(&url), art_url: url.into() }
    };
    let mut out: Vec<Person> = item.directors.iter().map(|d| person(&d.tag, "Director", d.thumb.as_deref())).collect();
    out.extend(item.cast.iter().take(24).map(|r| person(&r.tag, r.role.as_deref().unwrap_or(""), r.thumb.as_deref())));
    out.extend(item.writers.iter().take(3).map(|w| person(&w.tag, "Writer", w.thumb.as_deref())));
    out.extend(item.producers.iter().take(3).map(|p| person(&p.tag, "Producer", p.thumb.as_deref())));
    out
}

/// The first paragraph, cut at a word boundary near `max` characters. Album and artist
/// biographies from Plex can run to thousands of words.
fn short_summary(text: &str, max: usize) -> String {
    let first = text.split('\n').map(str::trim).find(|p| !p.is_empty()).unwrap_or("");
    if first.chars().count() <= max {
        return first.to_string();
    }
    let cut: String = first.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

fn channels(c: u32) -> String {
    match c {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}ch"),
    }
}

fn play_label(item: &Item) -> String {
    match item.view_offset.filter(|o| *o > 0) {
        Some(o) => format!("Resume from {}", clock(o as f64 / 1000.0)),
        None => "Play".into(),
    }
}

fn select_season(index: i32) {
    let ui = ui();
    ui.set_selected_season(index);
    let Some(season) = with_state(|s| s.season_items.get(index as usize).cloned()) else { return };
    let Some(server) = backend_for(&season.rating_key) else { return };
    let generation = with_state(|s| s.generation);
    rt().spawn(async move {
        let eps = server.children(&season.rating_key).await.unwrap_or_default();
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation || ui.get_selected_season() != index {
                return;
            }
            let rows: Vec<Episode> = eps
                .iter()
                .map(|e| {
                    let url = art_url(e.thumb.as_deref(), 356, 200);
                    Episode {
                        id: e.rating_key.clone().into(),
                        title: e.title.clone().into(),
                        label: format!("E{}", e.index.unwrap_or(0)).into(),
                        summary: e.summary.clone().unwrap_or_default().into(),
                        image: want_image(&url),
                        art_url: url.into(),
                        progress: e.progress().unwrap_or(0.0),
                        watched: e.is_watched(),
                        runtime: e.duration.map(|d| format!("{}m", d / 60_000)).unwrap_or_default().into(),
                    }
                })
                .collect();
            remember(eps);
            with_models(|m| m.episodes.set_vec(rows));
        });
    });
}

/// Play from 0:00 even when there's a resume point.
fn play_from_start() {
    let item = with_state(|s| s.next_up.clone().or_else(|| s.detail.clone()));
    if let Some(mut item) = item {
        item.view_offset = None;
        play_fresh(item);
    }
}

fn toggle_watched() {
    let Some(item) = with_state(|s| s.detail.clone()) else { return };
    let Some(server) = backend_for(&item.rating_key) else { return };
    let watched = item.is_watched();
    let id = item.rating_key.clone();
    ui().set_detail_watched(!watched);
    rt().spawn(async move {
        let r = server.set_watched(&id, !watched).await;
        on_ui(move |ui| match r {
            // Reload, so episode ticks, progress and the play button follow.
            Ok(()) => show_detail(item),
            Err(e) => {
                ui.set_detail_watched(watched);
                toast(ui, format!("Couldn't update the server: {e}"));
            }
        });
    });
}

fn play_detail() {
    if !play_music_detail(false) {
        let item = with_state(|s| s.next_up.clone().or_else(|| s.detail.clone()));
        if let Some(item) = item {
            play(item);
        }
    }
}

fn shuffle_detail() {
    play_music_detail(true);
}

/// Albums and artists: play every track, in order or shuffled. False for anything else.
fn play_music_detail(shuffle: bool) -> bool {
    let kind = ui().get_detail_kind();
    let order = move |tracks: Vec<Item>| if shuffle { shuffled(tracks) } else { tracks };
    match kind.as_str() {
        "album" | "playlist" => {
            let tracks = with_state(|s| s.album_tracks.clone());
            play_music(order(tracks), 0);
        }
        "artist" => {
            // An artist's tracks are its albums' tracks, album by album.
            let albums = with_state(|s| s.artist_albums.clone());
            let Some(server) = albums.first().and_then(|a| backend_for(&a.rating_key)) else {
                toast(&ui(), "Nothing to play.");
                return true;
            };
            ui().set_loading(true);
            rt().spawn(async move {
                let mut tracks = Vec::new();
                for album in &albums {
                    tracks.extend(server.children(&album.rating_key).await.unwrap_or_default());
                }
                on_ui(move |ui| {
                    ui.set_loading(false);
                    play_music(order(tracks), 0);
                });
            });
        }
        _ => return false,
    }
    true
}

/// The list in a random order (Fisher–Yates, seeded from a fresh random UUID).
fn shuffled<T>(mut v: Vec<T>) -> Vec<T> {
    let mut x = (uuid::Uuid::new_v4().as_u128() as u64) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
    v
}

// ---------- Playback ----------

fn ps(ui: &AppWindow) -> PlayerState<'_> {
    ui.global::<PlayerState>()
}

fn play(item: Item) {
    play_opts(item, false);
}

/// Play ignoring the resume point.
fn play_fresh(item: Item) {
    play_opts(item, true);
}

fn play_opts(item: Item, from_start: bool) {
    if shares::play_item(&item) {
        return;
    }
    let Some(server) = backend_for(&item.rating_key) else { return };
    let ui = ui();
    let back_to = ui.get_screen().to_string();
    if back_to != "player" {
        with_state(|s| s.return_to = back_to);
    }
    ps(&ui).set_buffering(true);
    let id = item.rating_key.clone();
    rt().spawn(async move {
        // Always refetch: list entries lack markers, and the resume point may have moved.
        let mut full = server.metadata(&id).await.ok().flatten().unwrap_or(item);
        if from_start {
            full.view_offset = None;
        }
        // Episodes get their show's episode list for previous/next, the list panel and autoplay.
        let episodes = match (&full.kind, &full.grandparent_rating_key) {
            (ItemKind::Episode, Some(show)) => server.all_episodes(show).await.unwrap_or_default(),
            _ => vec![],
        };
        let show_settings = match (&full.kind, &full.grandparent_rating_key) {
            (ItemKind::Episode, Some(show)) => server.preferences(show).await,
            _ => vec![],
        };
        on_ui(move |ui| {
            let ps = ps(ui);
            let Some(part) = full.primary_part() else {
                ps.set_buffering(false);
                return toast(ui, "Nothing to play: this item has no file.");
            };
            let Some(p) = player() else { return toast(ui, "The video player didn't start (libmpv missing?).") };
            let Some(url) = server.stream_url(&full) else { return };
            let (title, subtitle) = if full.kind == ItemKind::Episode {
                (
                    full.grandparent_title.clone().unwrap_or_default(),
                    format!("{} · {}", full.episode_label().unwrap_or_default(), full.title),
                )
            } else {
                (full.title.clone(), full.year.map(|y| y.to_string()).unwrap_or_default())
            };
            if with_state(|s| s.music.take()).is_some() {
                ui.set_music_active(false);
            }
            ps.set_title(title.into());
            ps.set_subtitle(subtitle.into());
            ps.set_position(0.0);
            ps.set_duration((full.duration.unwrap_or(0) as f64 / 1000.0) as f32);
            ps.set_skip_label(SharedString::new());
            ps.set_controls_visible(true);
            ps.set_panel(SharedString::new());
            ps.set_can_find_subs(true);
            ui.set_screen("player".into());
            p.load(&url, full.view_offset.map(|o| o as f64 / 1000.0));
            // Subtitle files next to the video only reach mpv if we hand them over.
            for st in part.streams_of(StreamKind::Subtitle).filter(|st| st.index.is_none()) {
                if let Some(sub_url) = server.subtitle_url(st) {
                    p.add_subtitle(&sub_url, &format!("plex:{}", st.id));
                }
            }
            remember([full.clone()]);
            {
                let (server, item) = (server.clone(), full.clone());
                rt().spawn(async move { server.report_start(&item).await });
            }
            let server_id = with_state(|s| s.session.server.as_ref().map(|sv| sv.client_identifier.clone())).unwrap_or_default();
            let show_key = full.grandparent_rating_key.as_ref().filter(|_| full.kind == ItemKind::Episode).map(|show| format!("{server_id}:{show}"));
            let show_languages = match &show_key {
                Some(key) => with_state(|s| s.show_store.get_or_insert_with(langs::ShowStore::load).get(key))
                    .or(langs::ShowLanguages::from_plex(&show_settings)),
                None => langs::ShowLanguages::default(),
            };
            with_state(|s| {
                s.now_playing = Some(NowPlaying {
                    item: full,
                    episodes,
                    streams_applied: false,
                    detected: HashMap::new(),
                    skipped: vec![],
                    user_picked_sub: false,
                    user_picked_audio: false,
                    show_key,
                    show_languages,
                    show_settings,
                    last_report: Instant::now(),
                    started: Instant::now(),
                });
                s.last_poke = Some(Instant::now());
            });
            update_neighbours(ui);
        });
    });
}

/// The episode before/after the playing one, if any.
fn neighbour(delta: isize) -> Option<Item> {
    with_state(|s| {
        let np = s.now_playing.as_ref()?;
        let i = np.episodes.iter().position(|e| e.rating_key == np.item.rating_key)?;
        np.episodes.get(i.checked_add_signed(delta)?).cloned()
    })
}

fn is_episode() -> bool {
    with_state(|s| s.now_playing.as_ref().is_some_and(|n| n.item.kind == ItemKind::Episode))
}

fn update_neighbours(ui: &AppWindow) {
    let ps = ps(ui);
    if is_episode() {
        ps.set_has_prev(neighbour(-1).is_some());
        ps.set_has_next(neighbour(1).is_some());
    } else {
        // Movies: previous/next step through chapters.
        let chapters = player().map(|p| p.chapters().len()).unwrap_or(0);
        ps.set_has_prev(chapters > 1);
        ps.set_has_next(chapters > 1);
    }
}

fn step(delta: isize) {
    if is_episode() {
        if let Some(ep) = neighbour(delta) {
            let pos = player().map(|p| p.status().position).unwrap_or(0.0);
            report(PlayState::Stopped, pos);
            play(ep);
        }
    } else if let Some(p) = player() {
        p.step_chapter(delta as i64);
    }
}

fn report(state: PlayState, position: f64) {
    if let Some(item) = with_state(|s| s.now_playing.as_ref().map(|n| n.item.clone())) {
        report_item(item, state, position);
    }
}

fn report_item(item: Item, state: PlayState, position: f64) {
    if shares::is_share_id(&item.rating_key) {
        return shares::record_progress(&item, position);
    }
    let Some(server) = backend_for(&item.rating_key) else { return };
    rt().spawn(async move {
        let _ = server.report_progress(&item, state, (position * 1000.0) as u64).await;
    });
}

fn stop_playback() {
    let Some(p) = player() else { return };
    let status = p.status();
    // (Nothing to report if the file never opened.)
    if !status.idle {
        report(PlayState::Stopped, status.position);
    }
    p.stop();
    #[cfg(target_os = "android")]
    android_video::stop();
    let ui = ui();
    if ps(&ui).get_pip() {
        toggle_pip();
    }
    if ps(&ui).get_fullscreen() {
        toggle_fullscreen();
    }
    let (back, detail) = with_state(|s| {
        s.now_playing = None;
        (std::mem::take(&mut s.return_to), s.detail.clone())
    });
    // Give Plex a moment to record the position, then refresh what's on screen.
    slint::Timer::single_shot(Duration::from_millis(600), move || match back.as_str() {
        "detail" => match detail {
            Some(d) => show_detail(d),
            None => enter_home(),
        },
        _ => {
            let sel = ui.get_selected_library();
            ui.set_screen("home".into());
            if !shares::reload_current() {
                if sel == -2 { refresh() } else { reload_page(sel) }
            }
        }
    });
}

/// Runs four times a second: mirrors mpv's state into the UI, reports progress, auto-hides
/// controls, offers to skip intros and credits, and moves on to the next episode at the end.
fn tick() {
    let ui = ui();
    fit_tv_scale(&ui);
    tv_hero_follow(&ui);
    #[cfg(target_os = "ios")]
    ios_video::follow_size();
    if ui.get_screen() != "player" {
        #[cfg(target_os = "android")]
        android_video::stop();
        return music_tick(&ui);
    }
    let Some(p) = player() else { return };
    let ps = ps(&ui);
    // Android: HDR straight to the TV when it takes it; Encheo then draws the subtitles.
    #[cfg(target_os = "android")]
    {
        let line = android_video::follow(with_state(|s| s.session.prefs.direct));
        ps.set_direct(line.is_some());
        ps.set_direct_sub(line.unwrap_or_default().into());
    }
    let st = p.status();
    ps.set_position(st.position as f32);
    if st.duration > 0.0 {
        ps.set_duration(st.duration as f32);
    }
    let duration = ps.get_duration() as f64;
    ps.set_position_text(clock(st.position).into());
    ps.set_remaining_text(format!("-{}", clock((duration - st.position).max(0.0))).into());
    ps.set_paused(st.paused);
    ps.set_buffering(st.buffering);
    ps.set_volume(p.volume() as f32);
    ps.set_muted(p.muted());
    ps.set_speed(p.speed() as f32);
    ps.set_sub_delay(p.sub_delay() as f32);
    ps.set_audio_delay(p.audio_delay() as f32);
    if !is_episode() {
        update_neighbours(&ui); // chapters appear once the file is open
    }
    apply_plex_streams(&p);

    // mpv went idle without playing: the file couldn't be opened (refused, missing, unreadable).
    // Say so instead of showing the spinner for ever.
    let failed = st.idle && with_state(|s| s.now_playing.as_ref().is_some_and(|np| np.started.elapsed() > Duration::from_secs(5)));
    if failed {
        toast(&ui, "Couldn't play this: the server didn't send the video.");
        return stop_playback();
    }
    let (report_now, marker, hide, finished) = with_state(|s| {
        let Some(np) = s.now_playing.as_mut() else { return (false, None, false, false) };
        let report_now = np.last_report.elapsed() >= Duration::from_secs(10) && !st.buffering;
        if report_now {
            np.last_report = Instant::now();
        }
        let ms = (st.position * 1000.0) as u64;
        let marker = np
            .item
            .markers
            .iter()
            .find(|m| matches!(m.kind, MarkerKind::Intro | MarkerKind::Credits) && ms >= m.start_time_offset && ms + 1500 < m.end_time_offset)
            .cloned();
        let hide = !st.paused && s.last_poke.is_some_and(|t| t.elapsed() > Duration::from_secs(3));
        let finished = st.ended && np.started.elapsed() > Duration::from_secs(3);
        (report_now, marker, hide, finished)
    });
    if report_now {
        report(if st.paused { PlayState::Paused } else { PlayState::Playing }, st.position);
    }
    ps.set_skip_label(match &marker {
        Some(m) if m.kind == MarkerKind::Intro => "Skip Intro".into(),
        Some(_) => "Skip Credits".into(),
        None => SharedString::new(),
    });
    // Auto-skip: jump once per marker. Credits only when there's more to watch after them (a
    // post-credits scene); otherwise the episode just ends and the next one starts.
    if let Some(m) = marker.filter(|_| ps.get_auto_skip()) {
        let first_time = with_state(|s| {
            s.now_playing.as_mut().is_some_and(|np| {
                let key = m.start_time_offset;
                if np.skipped.contains(&key) { false } else { np.skipped.push(key); true }
            })
        });
        if first_time {
            p.seek_to(m.end_time_offset as f64 / 1000.0);
            toast(&ui, if m.kind == MarkerKind::Intro { "Skipped the intro" } else { "Skipped the credits" });
        }
    }
    if hide && ps.get_controls_visible() && ps.get_panel().is_empty() {
        ps.set_controls_visible(false);
    }
    if finished {
        match neighbour(1).filter(|_| is_episode()) {
            Some(next) => {
                report(PlayState::Stopped, st.position);
                play(next);
            }
            None => stop_playback(),
        }
    }
}

fn skip_marker() {
    let Some(p) = player() else { return };
    let pos_ms = (p.status().position * 1000.0) as u64;
    let end = with_state(|s| {
        s.now_playing.as_ref().and_then(|np| {
            np.item.markers.iter().find(|m| pos_ms >= m.start_time_offset && pos_ms < m.end_time_offset).map(|m| m.end_time_offset)
        })
    });
    if let Some(end) = end {
        p.seek_to(end as f64 / 1000.0);
    }
}

/// Open the settings or list panel (or close with ""), filling it with fresh data from mpv.
fn open_panel(which: &str) {
    let ui = ui();
    let ps = ps(&ui);
    let Some(p) = player() else { return };
    match which {
        "subsearch" => {
            let mine = with_state(|s| s.subtitle_language.clone()).unwrap_or_else(|| "eng".into());
            let mut langs = vec![(mine.clone(), language_label(&mine))];
            if mine != "eng" {
                langs.push(("eng".into(), "English".into()));
            }
            ps.set_sub_langs(ModelRc::new(VecModel::from(
                langs.into_iter().map(|(id, title)| ListEntry { id: id.into(), title: title.into(), detail: SharedString::new(), current: false }).collect::<Vec<_>>(),
            )));
            ps.set_panel(which.into());
            return find_subtitles(mine);
        }
        "settings" => fill_tracks(&ui),
        "list" => {
            let entries: Vec<ListEntry> = if is_episode() {
                ps.set_list_title("EPISODES".into());
                with_state(|s| {
                    let np = s.now_playing.as_ref();
                    np.map(|np| {
                        np.episodes
                            .iter()
                            .map(|e| ListEntry {
                                id: e.rating_key.clone().into(),
                                title: format!("{}  {}", e.episode_label().unwrap_or_default(), e.title).into(),
                                detail: e.duration.map(|d| clock(d as f64 / 1000.0)).unwrap_or_default().into(),
                                current: e.rating_key == np.item.rating_key,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
                })
            } else {
                ps.set_list_title("CHAPTERS".into());
                let current = p.current_chapter();
                p.chapters()
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| ListEntry {
                        id: format!("chapter:{}", c.start).into(),
                        title: c.title.into(),
                        detail: clock(c.start).into(),
                        current: Some(i) == current,
                    })
                    .collect()
            };
            // TV remote: start on what's playing.
            ps.set_panel_row(entries.iter().position(|e| e.current).unwrap_or(0) as i32);
            ps.set_list_items(ModelRc::new(VecModel::from(entries)));
        }
        _ => {}
    }
    ps.set_panel(which.into());
}

/// Plex streams of the playing file.
fn plex_streams() -> Vec<Stream> {
    with_state(|s| s.now_playing.as_ref().and_then(|np| np.item.primary_part().map(|p| p.streams.clone())).unwrap_or_default())
}

/// The Plex stream an mpv track is: embedded tracks by their index in the file, files we added by
/// the `plex:<id>` title we gave them.
fn plex_stream_for<'a>(track: &player::Track, kind: StreamKind, streams: &'a [Stream]) -> Option<&'a Stream> {
    let ours = track.title.as_deref().and_then(|t| t.strip_prefix("plex:")).and_then(|id| id.parse::<u64>().ok());
    streams.iter().filter(|s| s.stream_type == kind as u8).find(|s| match ours {
        Some(id) => s.id == id,
        None => track.ff_index.is_some() && s.index.map(i64::from) == track.ff_index,
    })
}

/// Once the file's tracks are known, switch to the audio and subtitles the user last chose in any
/// Plex app. Runs once per file.
fn apply_plex_streams(p: &Player) {
    if with_state(|s| s.now_playing.as_ref().is_none_or(|np| np.streams_applied)) {
        return;
    }
    let audio = p.tracks("audio");
    if audio.is_empty() {
        return; // not loaded yet
    }
    let streams = plex_streams();
    for (kind, mpv_kind, tracks) in [(StreamKind::Audio, "audio", audio), (StreamKind::Subtitle, "sub", p.tracks("sub"))] {
        let wanted = streams.iter().find(|s| s.stream_type == kind as u8 && s.selected);
        if let Some(w) = wanted
            && let Some(t) = tracks.iter().find(|t| plex_stream_for(t, kind, &streams).is_some_and(|s| s.id == w.id))
        {
            p.select_track(mpv_kind, Some(t.id));
        }
    }
    with_state(|s| {
        if let Some(np) = s.now_playing.as_mut() {
            np.streams_applied = true;
        }
    });
    apply_language_preferences();
    detect_subtitle_languages(p);
}

/// Untagged embedded subtitle tracks get their language detected from their text: from the cache
/// if this file was scanned before, otherwise by reading a stretch of the file in the background.
fn detect_subtitle_languages(p: &Player) {
    let untagged: Vec<i64> = p.tracks("sub").iter().filter(|t| t.lang.is_none()).filter_map(|t| t.ff_index).collect();
    if untagged.is_empty() {
        return;
    }
    let Some((part_id, _part_key, item_id)) = with_state(|s| {
        let np = s.now_playing.as_ref()?;
        let part = np.item.primary_part()?;
        Some((part.id, part.clone(), np.item.rating_key.clone()))
    }) else {
        return;
    };
    let cached = with_state(|s| s.language_cache.get_or_insert_with(langs::Cache::load).get(part_id).cloned());
    if let Some(found) = cached {
        return use_detected(&item_id, found);
    }
    let share_item = with_state(|s| s.now_playing.as_ref().map(|n| n.item.clone())).filter(|i| shares::is_share_id(&i.rating_key));
    let url = match share_item {
        Some(_) => None,
        None => {
            let item = with_state(|s| s.now_playing.as_ref().map(|n| n.item.clone()));
            match item.and_then(|i| backend_for(&i.rating_key).and_then(|b| b.stream_url(&i))) {
                Some(u) => Some(u),
                None => return,
            }
        }
    };
    rt().spawn_blocking(move || {
        let started = Instant::now();
        let reader: Box<dyn ReadSeekSend> = match (&url, &share_item) {
            (Some(url), _) => match encheo_sublang::HttpReader::new(url) {
                Ok(r) => Box::new(r),
                Err(e) => return eprintln!("subtitle scan: {e}"),
            },
            (None, Some(item)) => match shares::open_reader(item) {
                Some(r) => Box::new(r),
                None => return,
            },
            _ => return,
        };
        let texts = match encheo_sublang::untagged_subtitle_text(reader, &encheo_sublang::Budget::default()) {
            Ok(t) => t,
            Err(e) => return eprintln!("subtitle scan: {e}"),
        };
        let found: HashMap<i64, langs::Detected> = texts
            .iter()
            .filter_map(|(i, text)| {
                let lang = encheo_sublang::detect(text)?;
                Some((*i as i64, langs::Detected { code: lang.code.into(), name: lang.name.into() }))
            })
            .collect();
        eprintln!("subtitle scan: {} of {} tracks identified in {:.1}s", found.len(), texts.len(), started.elapsed().as_secs_f64());
        on_ui(move |_| {
            with_state(|s| s.language_cache.get_or_insert_with(langs::Cache::load).insert(part_id, found.clone()));
            use_detected(&item_id, found);
        });
    });
}

/// What the subtitle scanner reads from: a server URL or a file source.
trait ReadSeekSend: std::io::Read + std::io::Seek + Send {}
impl<T: std::io::Read + std::io::Seek + Send + ?Sized> ReadSeekSend for T {}

fn use_detected(item_id: &str, found: HashMap<i64, langs::Detected>) {
    let still_playing = with_state(|s| {
        let Some(np) = s.now_playing.as_mut().filter(|np| np.item.rating_key == item_id) else { return false };
        np.detected = found;
        true
    });
    if !still_playing {
        return;
    }
    apply_language_preferences();
    let ui = ui();
    if ps(&ui).get_panel() == "settings" {
        fill_tracks(&ui);
    }
}

/// The language a track really is: its tag if it has one, else what we detected.
fn track_language(t: &player::Track, detected: &HashMap<i64, langs::Detected>) -> Option<String> {
    t.lang.as_deref().map(langs::normalize).or_else(|| t.ff_index.and_then(|i| detected.get(&i)).map(|d| langs::normalize(&d.code)))
}

/// Pick audio and subtitles by language. The show's remembered languages come first (so a show
/// you watch in Japanese stays Japanese from episode to episode); without those, an untagged
/// subtitle track that turns out not to be in the account's language is swapped for one that is.
/// Runs when a file's tracks appear and again when detection finishes; never overrides a choice
/// the user made during this file.
fn apply_language_preferences() {
    let Some(p) = player() else { return };
    let Some((prefs, detected, picked_audio, picked_sub, account_sub)) = with_state(|s| {
        let np = s.now_playing.as_ref()?;
        Some((np.show_languages.clone(), np.detected.clone(), np.user_picked_audio, np.user_picked_sub, s.subtitle_language.clone()))
    }) else {
        return;
    };

    if let (Some(want), false) = (&prefs.audio, picked_audio) {
        let audio = p.tracks("audio");
        let current_ok = audio.iter().find(|t| t.selected).is_some_and(|t| track_language(t, &detected).as_deref() == Some(want.as_str()));
        if !current_ok
            && let Some(t) = audio.iter().find(|t| track_language(t, &detected).as_deref() == Some(want.as_str()))
        {
            p.select_track("audio", Some(t.id));
        }
    }

    if picked_sub {
        return;
    }
    let subs = p.tracks("sub");
    let current = subs.iter().find(|t| t.selected);
    let pick_language = |want: &str| {
        subs.iter().filter(|t| track_language(t, &detected).as_deref() == Some(want)).min_by_key(|t| t.forced).map(|t| t.id)
    };
    match &prefs.subtitles {
        Some(langs::SubtitlePref::Off) => {
            if current.is_some() {
                p.select_track("sub", None);
            }
        }
        Some(langs::SubtitlePref::Language(want)) => {
            let ok = current.is_some_and(|t| track_language(t, &detected).as_deref() == Some(want.as_str()) && !t.forced);
            if !ok && let Some(id) = pick_language(want) {
                p.select_track("sub", Some(id));
            }
        }
        None => {
            // No show preference: only fix untagged tracks that were picked under a wrong label.
            let (Some(want), Some(cur)) = (account_sub, current) else { return };
            if cur.lang.is_some() || track_language(cur, &detected).is_none_or(|l| l == want) {
                return;
            }
            if let Some(id) = pick_language(&want) {
                eprintln!("subtitles: switching from track {} to {id} ({want})", cur.id);
                choose_track_inner(StreamKind::Subtitle, Some(id));
            }
        }
    }
}

/// Remember the language of a track the user picked for the whole show, locally and (if allowed)
/// in the show's Plex settings so other Plex apps follow.
fn remember_show_language(kind: StreamKind, mpv_id: Option<i64>) {
    let Some(p) = player() else { return };
    let Some((key, detected, settings, mut langs_now)) = with_state(|s| {
        let np = s.now_playing.as_ref()?;
        Some((np.show_key.clone()?, np.detected.clone(), np.show_settings.clone(), np.show_languages.clone()))
    }) else {
        return;
    };
    let mpv_kind = if kind == StreamKind::Audio { "audio" } else { "sub" };
    let lang = mpv_id.and_then(|id| p.tracks(mpv_kind).into_iter().find(|t| t.id == id)).and_then(|t| track_language(&t, &detected));
    match (kind, mpv_id, lang) {
        (StreamKind::Audio, _, Some(l)) => langs_now.audio = Some(l),
        (StreamKind::Subtitle, None, _) => langs_now.subtitles = Some(langs::SubtitlePref::Off),
        (StreamKind::Subtitle, Some(_), Some(l)) => langs_now.subtitles = Some(langs::SubtitlePref::Language(l)),
        _ => return, // a track of unknown language: nothing to carry over
    }
    with_state(|s| {
        s.show_store.get_or_insert_with(langs::ShowStore::load).set(key.clone(), langs_now.clone());
        if let Some(np) = s.now_playing.as_mut() {
            np.show_languages = langs_now.clone();
        }
    });
    let values = langs_now.to_plex(&settings);
    let show = key.split_once(':').map(|(_, show)| show.to_string()).unwrap_or_default();
    if let (Some(server), false) = (backend_for(&show), values.is_empty()) {
        rt().spawn(async move {
            let pairs: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
            if let Err(e) = server.set_preferences(&show, &pairs).await {
                eprintln!("saving show languages in Plex (kept locally): {e}");
            }
        });
    }
}

/// The user picked a track: switch mpv and tell Plex, so every Plex app remembers it.
fn choose_track(kind: StreamKind, mpv_id: Option<i64>) {
    with_state(|s| {
        if let Some(np) = s.now_playing.as_mut() {
            match kind {
                StreamKind::Subtitle => np.user_picked_sub = true,
                _ => np.user_picked_audio = true,
            }
        }
    });
    choose_track_inner(kind, mpv_id);
    remember_show_language(kind, mpv_id);
}

fn choose_track_inner(kind: StreamKind, mpv_id: Option<i64>) {
    let Some(p) = player() else { return };
    let mpv_kind = if kind == StreamKind::Audio { "audio" } else { "sub" };
    p.select_track(mpv_kind, mpv_id);
    let streams = plex_streams();
    let plex_id = match mpv_id {
        None => Some(0), // subtitles off
        Some(id) => p.tracks(mpv_kind).iter().find(|t| t.id == id).and_then(|t| plex_stream_for(t, kind, &streams)).map(|s| s.id),
    };
    let part = with_state(|s| {
        s.now_playing.as_ref().filter(|np| !shares::is_share_id(&np.item.rating_key)).and_then(|np| np.item.primary_part().map(|p| p.id))
    });
    let owner = with_state(|s| s.now_playing.as_ref().map(|n| n.item.rating_key.clone())).unwrap_or_default();
    if let (Some(server), Some(part), Some(plex_id)) = (backend_for(&owner), part, plex_id) {
        rt().spawn(async move {
            let r = if kind == StreamKind::Audio {
                server.set_streams(part, Some(plex_id), None).await
            } else {
                server.set_streams(part, None, Some(plex_id)).await
            };
            if let Err(e) = r {
                eprintln!("saving track choice: {e}");
            }
        });
    }
    slint::Timer::single_shot(Duration::from_millis(150), || fill_tracks(&crate::ui()));
}

fn fill_tracks(ui: &AppWindow) {
    let Some(p) = player() else { return };
    let streams = plex_streams();
    let detected = with_state(|s| s.now_playing.as_ref().map(|n| n.detected.clone()).unwrap_or_default());
    let to_options = |tracks: Vec<player::Track>, kind: StreamKind| -> Vec<TrackOption> {
        tracks
            .into_iter()
            .map(|t| {
                // Untagged in the file: our detection beats Plex's guess (which is "English").
                let found = t.lang.is_none().then(|| t.ff_index.and_then(|i| detected.get(&i))).flatten();
                let label = match found {
                    Some(d) => {
                        let forced = if t.forced { " Forced" } else { "" };
                        let codec = t.codec.as_deref().map(|c| format!(" ({c})")).unwrap_or_default();
                        format!("{}{forced}{codec} · detected", d.name)
                    }
                    // Otherwise Plex's names are nicer than the raw tags.
                    None => plex_stream_for(&t, kind, &streams)
                        .and_then(|s| s.extended_display_title.clone().or_else(|| s.display_title.clone()))
                        .unwrap_or(t.label),
                };
                TrackOption { id: t.id as i32, label: label.into(), selected: t.selected }
            })
            .collect()
    };
    let subs = p.tracks("sub");
    let ps = ps(ui);
    ps.set_sub_off(!subs.iter().any(|t| t.selected));
    ps.set_audio_tracks(ModelRc::new(VecModel::from(to_options(p.tracks("audio"), StreamKind::Audio))));
    ps.set_sub_tracks(ModelRc::new(VecModel::from(to_options(subs, StreamKind::Subtitle))));
}

fn language_label(code: &str) -> String {
    match langs::normalize(code).as_str() {
        "eng" => "English", "nld" => "Dutch", "fra" => "French", "deu" => "German", "spa" => "Spanish",
        "ita" => "Italian", "por" => "Portuguese", "jpn" => "Japanese", "kor" => "Korean", "cmn" => "Chinese",
        "swe" => "Swedish", "dan" => "Danish", "nob" => "Norwegian", "fin" => "Finnish", "pol" => "Polish",
        "tur" => "Turkish", "rus" => "Russian", "arb" => "Arabic",
        other => return other.to_uppercase(),
    }
    .into()
}

/// Ask the playing item's server for subtitles in a language.
fn find_subtitles(lang: String) {
    let ui = ui();
    let ps = ps(&ui);
    let Some(item) = with_state(|s| s.now_playing.as_ref().map(|n| n.item.clone())) else { return };
    let Some(server) = backend_for(&item.rating_key) else { return };
    let langs: Vec<ListEntry> = ps.get_sub_langs().iter().map(|mut l| { l.current = l.id == lang.as_str(); l }).collect();
    ps.set_sub_langs(ModelRc::new(VecModel::from(langs)));
    ps.set_sub_results(ModelRc::new(VecModel::from(Vec::<ListEntry>::new())));
    ps.set_sub_search_status(format!("Searching {}…", language_label(&lang)).into());
    rt().spawn(async move {
        let found = server.search_subtitles(&item.rating_key, &lang).await;
        on_ui(move |ui| {
            let ps = crate::ps(ui);
            match found {
                Ok(list) if list.is_empty() => ps.set_sub_search_status("Nothing found.".into()),
                Ok(list) => {
                    ps.set_sub_search_status(SharedString::new());
                    let rows: Vec<ListEntry> = list
                        .into_iter()
                        .map(|(key, title, detail)| ListEntry { id: format!("sub:{key}").into(), title: title.into(), detail: detail.into(), current: false })
                        .collect();
                    ps.set_sub_results(ModelRc::new(VecModel::from(rows)));
                }
                Err(e) => ps.set_sub_search_status(format!("Couldn't search: {e}").into()),
            }
        });
    });
}

/// Save the chosen online subtitle on the server and switch to it.
fn pick_online_subtitle(key: String) {
    let ui = ui();
    let ps = ps(&ui);
    let Some(item) = with_state(|s| s.now_playing.as_ref().map(|n| n.item.clone())) else { return };
    let Some(server) = backend_for(&item.rating_key) else { return };
    ps.set_sub_search_status("Downloading…".into());
    rt().spawn(async move {
        let result = server.pick_subtitle(&item.rating_key, &key).await;
        on_ui(move |ui| {
            let ps = crate::ps(ui);
            match result {
                Ok(Some(url)) => {
                    if let Some(p) = player() {
                        p.add_subtitle_selected(&url, "Downloaded");
                    }
                    with_state(|s| {
                        if let Some(np) = s.now_playing.as_mut() {
                            np.user_picked_sub = true;
                        }
                    });
                    ps.set_sub_search_status(SharedString::new());
                    ps.set_panel(SharedString::new());
                }
                Ok(None) => ps.set_sub_search_status("Saved on the server; it shows up next time you play this.".into()),
                Err(e) => ps.set_sub_search_status(format!("Couldn't download it: {e}").into()),
            }
        });
    });
}

fn pick(id: String) {
    if let Some(key) = id.strip_prefix("sub:") {
        return pick_online_subtitle(key.to_string());
    }
    if let Some(start) = id.strip_prefix("chapter:") {
        if let (Some(p), Ok(t)) = (player(), start.parse::<f64>()) {
            p.seek_to(t);
        }
        return crate::ps(&crate::ui()).set_panel(SharedString::new());
    }
    let ep = with_state(|s| s.now_playing.as_ref().and_then(|np| np.episodes.iter().find(|e| e.rating_key == id).cloned()));
    if let Some(ep) = ep {
        let pos = player().map(|p| p.status().position).unwrap_or(0.0);
        report(PlayState::Stopped, pos);
        play(ep);
    }
}

fn toggle_fullscreen() {
    let ui = ui();
    let ps = ps(&ui);
    let on = !ui.window().is_fullscreen();
    ui.window().set_fullscreen(on);
    ps.set_fullscreen(on);
}

/// Android: the app is always full screen and system PiP needs the Java activity; not yet.
#[cfg(target_os = "android")]
fn toggle_pip() {}

/// Picture in Picture: a small borderless always-on-top window, back to normal on the second press.
#[cfg(not(target_os = "android"))]
fn toggle_pip() {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::{dpi, window::WindowLevel};
    let ui = ui();
    let ps = ps(&ui);
    let entering = !ps.get_pip();
    if entering && ui.window().is_fullscreen() {
        toggle_fullscreen();
    }
    if entering {
        ps.set_panel(SharedString::new()); // no room for panels in the small window
    }
    let restore = with_state(|s| s.pip_restore.take());
    ui.window().with_winit_window(|w| {
        if entering {
            with_state(|s| s.pip_restore = Some((w.inner_size(), w.outer_position().ok())));
            w.set_window_level(WindowLevel::AlwaysOnTop);
            let _ = w.request_inner_size(dpi::LogicalSize::new(480.0, 270.0));
            if let Some(m) = w.current_monitor() {
                let scale = m.scale_factor();
                let size = m.size().to_logical::<f64>(scale);
                let pos = m.position().to_logical::<f64>(scale);
                w.set_outer_position(dpi::LogicalPosition::new(pos.x + size.width - 480.0 - 24.0, pos.y + size.height - 270.0 - 80.0));
            }
        } else {
            w.set_window_level(WindowLevel::Normal);
            if let Some((size, pos)) = restore {
                let _ = w.request_inner_size(size);
                if let Some(pos) = pos {
                    w.set_outer_position(pos);
                }
            }
        }
    });
    ps.set_pip(entering);
}

// ---------- Network discovery and speed test ----------

/// Look for servers and shares on the local network for the Add Source form.
fn scan_network() {
    let ui = ui();
    if ui.get_scanning() {
        return;
    }
    ui.set_scanning(true);
    rt().spawn_blocking(|| {
        let found = encheo_discovery::discover(Duration::from_secs(3));
        on_ui(move |ui| {
            ui.set_scanning(false);
            let rows: Vec<FoundSource> = found
                .into_iter()
                .filter(|f| f.kind != encheo_discovery::Kind::Plex) // Plex servers come with the account
                .map(|f| {
                    let (kind, label) = match f.kind {
                        encheo_discovery::Kind::Smb => (1, "SMB"),
                        encheo_discovery::Kind::WebDav => (2, "WebDAV"),
                        encheo_discovery::Kind::Emby => (3, "Emby"),
                        encheo_discovery::Kind::Dlna => (7, "DLNA"),
                        _ => (3, "Jellyfin"),
                    };
                    FoundSource { kind, label: label.into(), name: f.name.into(), address: f.address.into() }
                })
                .collect();
            ui.set_found_sources(ModelRc::new(VecModel::from(rows)));
        });
    });
}

/// How fast the source or server on screen delivers a real video, in Mbit/s.
fn speed_test() {
    let ui = ui();
    toast(&ui, "Testing speed…");
    // A file source: the first video in the folder on screen.
    if let Some((_, _)) = with_state(|s| s.share_view.clone()) {
        let item = with_state(|s| {
            let ids: Vec<String> = with_models(|m| m.grid.iter().map(|p| p.id.to_string()).collect());
            ids.iter().filter_map(|id| s.items.get(id)).find(|i| matches!(i.kind, ItemKind::Movie | ItemKind::Episode)).cloned()
        });
        let Some(item) = item else { return toast(&ui, "Open a folder with a video in it to test the speed.") };
        rt().spawn_blocking(move || {
            let result = shares::open_reader(&item)
                .ok_or_else(|| "couldn't open the file".to_string())
                .and_then(|mut r| encheo_discovery::measure(&mut r, 96 << 20, Duration::from_secs(8)).map_err(|e| e.to_string()));
            on_ui(move |ui| report_speed(ui, result));
        });
        return;
    }
    // A media server library: the first title with a file (a show's first episode).
    let first = with_models(|m| m.grid.row_data(0)).map(|p| p.id.to_string());
    let Some(id) = first else { return toast(&ui, "Nothing here to test with.") };
    let Some(server) = backend_for(&id) else { return };
    rt().spawn(async move {
        let mut item = server.metadata(&id).await.ok().flatten();
        if item.as_ref().is_some_and(|i| i.kind == ItemKind::Show) {
            let ep = server.all_episodes(&id).await.ok().and_then(|e| e.into_iter().next());
            item = match ep {
                Some(e) => server.metadata(&e.rating_key).await.ok().flatten(),
                None => None,
            };
        }
        let url = item.and_then(|i| server.stream_url(&i));
        let result = tokio::task::spawn_blocking(move || {
            let url = url.ok_or_else(|| "nothing playable found".to_string())?;
            let mut r = encheo_sublang::HttpReader::new(&url).map_err(|e| e.to_string())?;
            encheo_discovery::measure(&mut r, 96 << 20, Duration::from_secs(8)).map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        on_ui(move |ui| report_speed(ui, result));
    });
}

fn report_speed(ui: &AppWindow, result: Result<f64, String>) {
    match result {
        Ok(mbps) => toast(ui, format!("{:.0} Mbit/s — {}", mbps, speed_verdict(mbps))),
        Err(e) => toast(ui, format!("Speed test failed: {e}")),
    }
}

/// What a speed is good for. 4K Blu-ray remuxes peak around 80–100 Mbit/s.
fn speed_verdict(mbps: f64) -> &'static str {
    match mbps {
        m if m >= 120.0 => "enough for anything, including 4K remuxes",
        m if m >= 50.0 => "fine for 4K; the biggest remuxes may buffer",
        m if m >= 20.0 => "fine for 1080p; 4K may buffer",
        m if m >= 8.0 => "fine for smaller 1080p files",
        _ => "too slow for HD; expect buffering",
    }
}

// ---------- Search ----------

/// Called on every keystroke; waits for a short pause in typing before asking the server.
fn search(query: String) {
    let generation = with_state(|s| {
        s.search_generation += 1;
        s.search_generation
    });
    let query = query.trim().to_string();
    if query.is_empty() {
        let ui = ui();
        if ui.get_selected_library() == -2 {
            if ui.global::<Theme>().get_tv() {
                ui.set_grid_title("Search".into());
                ui.set_loading(false);
                with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
            } else {
                enter_home();
            }
        }
        return;
    }
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        if with_state(|s| s.search_generation) != generation {
            return; // typed more since
        }
        let backends = all_backends();
        let ui = ui();
        ui.set_screen("home".into());
        ui.set_selected_library(-2);
        ui.set_grid_title(format!("Results for “{query}”").into());
        ui.set_grid_square(false);
        ui.set_loading(true);
        with_state(|s| s.generation += 1);
        rt().spawn(async move {
            let mut found = Vec::new();
            let mut errors = Vec::new();
            for b in &backends {
                match b.search(&query).await {
                    Ok(items) => found.extend(items),
                    Err(e) => errors.push(e),
                }
            }
            let q = query.clone();
            let files = tokio::task::spawn_blocking(move || shares::search_files(&q)).await.unwrap_or_default();
            let results: Result<Vec<Item>, String> =
                if found.is_empty() && files.is_empty() && !errors.is_empty() { Err(errors.join("; ")) } else { Ok(found) };
            on_ui(move |ui| {
                if with_state(|s| s.search_generation) != generation {
                    return;
                }
                ui.set_loading(false);
                match results {
                    Ok(items) => {
                        // Servers also list a show's episodes and seasons; the show is enough.
                        let items: Vec<Item> =
                            items.into_iter().filter(|i| !matches!(i.kind, ItemKind::Episode | ItemKind::Season)).collect();
                        let mut posters: Vec<Poster> = items.iter().map(|i| poster(i, false)).collect();
                        remember(items);
                        let progress = with_state(|s| s.progress.get_or_insert_with(progress::Store::load).clone());
                        posters.extend(files.iter().map(|(item, name)| shares::file_poster(item, name, &progress)));
                        remember(files.into_iter().map(|(item, _)| item).collect::<Vec<_>>());
                        // An empty grid says "No results" itself; no toast on every letter.
                        with_models(|m| m.grid.set_vec(posters));
                    }
                    Err(e) => toast(ui, format!("Search failed: {e}")),
                }
            });
        });
    });
}

/// TV: the search page, with a keyboard for the remote on the left and results on the right.
fn open_tv_search() {
    let ui = ui();
    ui.set_screen("home".into());
    ui.set_selected_library(-2);
    with_state(|s| s.generation += 1);
    let text = ui.get_search_text().to_string();
    if text.trim().is_empty() {
        ui.set_grid_title("Search".into());
        ui.set_loading(false);
        with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    } else {
        search(text);
    }
    ui.global::<Nav>().set_active(true);
    set_focus(&ui, "keys", 0, 0);
}

/// The TV search keyboard's keys, row by row (6 wide); the bottom row is Space, Delete, Clear.
const TV_KEYS: &str = "abcdefghijklmnopqrstuvwxyz1234567890";

fn tv_key(key: &str) {
    let ui = ui();
    let mut text = ui.get_search_text().to_string();
    match key {
        "space" if !text.is_empty() && !text.ends_with(' ') => text.push(' '),
        "space" => {}
        "delete" => {
            text.pop();
        }
        "clear" => text.clear(),
        k => text.push_str(k),
    }
    ui.set_search_text(text.clone().into());
    search(text);
}

/// Remote focus on the TV search keyboard (zone "keys").
fn keys_nav(ui: &AppWindow, row: i32, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let to_results = |ui: &AppWindow| {
        if with_models(|m| m.grid.row_count()) > 0 {
            set_focus(ui, "content", 0, 0);
        }
    };
    let bottom = 6;
    if enter {
        let k = if row < bottom {
            TV_KEYS.get((row * 6 + col) as usize..(row * 6 + col + 1) as usize).unwrap_or("")
        } else {
            ["space", "delete", "clear"][(col / 2).clamp(0, 2) as usize]
        };
        if !k.is_empty() {
            tv_key(k);
        }
    } else if is(Key::UpArrow) {
        set_focus(ui, "keys", (row - 1).max(0), col);
    } else if is(Key::DownArrow) {
        let r = (row + 1).min(bottom);
        set_focus(ui, "keys", r, if r == bottom { col / 2 * 2 } else { col });
    } else if is(Key::LeftArrow) {
        let step = if row == bottom { 2 } else { 1 };
        if col >= step {
            set_focus(ui, "keys", row, col - step);
        } else {
            set_focus(ui, "side", 0, 0);
        }
    } else if is(Key::RightArrow) {
        let last = if row == bottom { 4 } else { 5 };
        if col < last {
            set_focus(ui, "keys", row, if row == bottom { col / 2 * 2 + 2 } else { col + 1 });
        } else {
            to_results(ui);
        }
    }
    true
}

fn tv_search_page(ui: &AppWindow) -> bool {
    ui.global::<Theme>().get_tv() && ui.get_screen() == "home" && ui.get_selected_library() == -2
}

fn refresh() {
    if shares::reload_current() {
        return;
    }
    let ui = ui();
    let selected = ui.get_selected_library();
    if selected == -2 {
        shares::forget_index();
    }
    reload_page(selected);
}

// ---------- Music ----------

fn play_music(items: Vec<Item>, start: usize) {
    let Some(p) = player() else { return toast(&ui(), "The player didn't start (libmpv missing?).") };
    let items: Vec<Item> = items.into_iter().filter(|i| i.primary_part().is_some()).collect();
    if items.is_empty() {
        return toast(&ui(), "Nothing to play.");
    }
    let start = start.min(items.len() - 1);
    for (n, item) in items.iter().enumerate() {
        let url = match shares::music_url(item).or_else(|| backend_for(&item.rating_key)?.stream_url(item)) {
            Some(u) => u,
            None => continue,
        };
        if n == 0 {
            p.load(&url, None);
        } else {
            p.append(&url);
        }
    }
    p.set_playlist_pos(start);
    p.set_repeat(with_state(|s| s.music_repeat));
    with_state(|s| s.music = Some(MusicQueue { items, index: usize::MAX, last_report: Instant::now(), art_url: String::new(), art_large_url: String::new(), lyrics: HashMap::new(), lyrics_loading: None, lyrics_shown: String::new() }));
    ui().set_music_active(true);
    music_tick(&ui());
}

fn music_tick(ui: &AppWindow) {
    let Some(p) = player() else { return };
    if !with_state(|s| s.music.is_some()) {
        return;
    }
    let st = p.status();
    let pos = p.playlist_pos();
    let (changed, current, report_now) = with_state(|s| {
        let q = s.music.as_mut().expect("checked");
        let changed = pos.is_some_and(|i| i != q.index && i < q.items.len());
        if changed {
            q.index = pos.unwrap_or(0);
            q.last_report = Instant::now();
        }
        let report_now = q.last_report.elapsed() >= Duration::from_secs(10);
        if report_now {
            q.last_report = Instant::now();
        }
        (changed, q.items.get(q.index).cloned(), report_now)
    });
    let Some(track) = current else { return };
    if changed {
        let url = art_url(track.parent_thumb.as_deref().or(track.thumb.as_deref()), 160, 160);
        with_state(|s| {
            if let Some(q) = s.music.as_mut() {
                q.art_url = url.clone();
            }
        });
        ui.set_music_title(track.title.clone().into());
        ui.set_music_artist(
            [track.artist().map(str::to_string), track.parent_title.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" — ")
                .into(),
        );
        ui.set_music_art(want_image(&url));
        let large = art_url(track.parent_thumb.as_deref().or(track.thumb.as_deref()), 800, 800);
        with_state(|s| {
            if let Some(q) = s.music.as_mut() {
                q.art_large_url = large.clone();
            }
        });
        ui.set_music_art_large(want_image(&large));
        refresh_tracks();
        report_item(track.clone(), PlayState::Playing, 0.0);
    }
    ui.set_music_paused(st.paused);
    ui.set_music_position(st.position as f32);
    ui.set_music_duration(if st.duration > 0.0 { st.duration as f32 } else { track.duration.unwrap_or(0) as f32 / 1000.0 });
    ui.set_music_position_text(clock(st.position).into());
    ui.set_music_duration_text(clock(ui.get_music_duration() as f64).into());
    lyrics_tick(ui, &track, st.position);
    #[cfg(target_os = "android")]
    android_media::sync(
        &track.rating_key,
        &track.title,
        track.artist().unwrap_or_default(),
        track.parent_title.as_deref().unwrap_or_default(),
        &with_state(|s| s.music.as_ref().map(|q| q.art_large_url.clone()).unwrap_or_default()),
        ui.get_music_duration() as f64,
        st.position,
        !st.paused,
    );
    if report_now {
        report_item(track, if st.paused { PlayState::Paused } else { PlayState::Playing }, st.position);
    }
}

/// Android: a music button on the lock screen, in the notification or on headphones.
#[cfg(target_os = "android")]
fn media_command(cmd: &str) {
    let ui = ui();
    let Some(p) = player() else { return };
    if !ui.get_music_active() {
        return android_media::stop();
    }
    let paused = p.status().paused;
    match cmd {
        "play" if paused => ui.invoke_music_toggle(),
        "pause" if !paused => ui.invoke_music_toggle(),
        "next" => ui.invoke_music_next(),
        "prev" => ui.invoke_music_prev(),
        "stop" => music_close(),
        _ => {
            if let Some(ms) = cmd.strip_prefix("seek:").and_then(|m| m.parse::<f64>().ok()) {
                p.seek_to(ms / 1000.0);
            }
        }
    }
}

/// Lyrics in Now Playing, while they're open: fetches the song's words once, puts them on screen
/// when the song changes, and lights the line being sung.
fn lyrics_tick(ui: &AppWindow, track: &Item, position: f64) {
    if !ui.get_lyrics_open() {
        return;
    }
    let id = &track.rating_key;
    enum Step {
        Fetch,
        Wait,
        Show(Option<encheo_plex::Lyrics>),
        Follow(i32),
    }
    let step = with_state(|s| {
        let q = s.music.as_mut()?;
        let fresh = q.lyrics_shown != *id;
        Some(match q.lyrics.get(id) {
            Some(l) if fresh => {
                q.lyrics_shown = id.clone();
                Step::Show(l.clone())
            }
            Some(l) => Step::Follow(
                l.as_ref().filter(|l| l.synced).and_then(|l| lyrics::current(&l.lines, position)).map_or(-1, |i| i as i32),
            ),
            None if q.lyrics_loading.as_deref() == Some(id) => Step::Wait,
            None => {
                q.lyrics_loading = Some(id.clone());
                q.lyrics_shown = id.clone();
                Step::Fetch
            }
        })
    });
    match step {
        None | Some(Step::Wait) => {}
        Some(Step::Fetch) => {
            ui.set_music_lyrics(ModelRc::default());
            ui.set_music_lyric_index(-1);
            ui.set_music_lyrics_credit(SharedString::new());
            ui.set_music_lyrics_state("loading".into());
            let Some(server) = backend_for(id) else { return };
            let track = track.clone();
            rt().spawn(async move {
                // Run apart, so a failure inside still ends in "No lyrics" rather than a spinner.
                let t = track.clone();
                let found = rt().spawn(async move { server.lyrics(&t).await }).await.ok().flatten();
                on_ui(move |ui| {
                    with_state(|s| {
                        if let Some(q) = s.music.as_mut() {
                            let id = track.rating_key.clone();
                            // (The song may have changed meanwhile: leave the next one's fetch be.)
                            if q.lyrics_loading.as_ref() == Some(&id) {
                                q.lyrics_loading = None;
                            }
                            if q.lyrics_shown == id {
                                q.lyrics_shown.clear(); // shown by the tick below
                            }
                            q.lyrics.insert(id, found);
                        }
                    });
                    music_tick(ui);
                });
            });
        }
        Some(Step::Show(l)) => {
            let (state, lines, credit) = match l {
                None => ("none", Vec::new(), String::new()),
                Some(l) => (
                    if l.synced { "synced" } else { "plain" },
                    l.lines.into_iter().map(|l| LyricLine { text: l.text.into(), time: l.start as f32 }).collect(),
                    l.credit.unwrap_or_default(),
                ),
            };
            ui.set_music_lyric_index(-1);
            ui.set_music_lyrics(ModelRc::new(VecModel::from(lines)));
            ui.set_music_lyrics_credit(credit.into());
            ui.set_music_lyrics_state(state.into());
            lyrics_tick(ui, track, position);
        }
        Some(Step::Follow(i)) => ui.set_music_lyric_index(i),
    }
}

/// Rebuild the album's track list, marking the one that's playing.
fn refresh_tracks() {
    let (tracks, playing) = with_state(|s| {
        (s.album_tracks.clone(), s.music.as_ref().and_then(|q| q.items.get(q.index)).map(|i| i.rating_key.clone()))
    });
    let album_artist = with_state(|s| s.detail.as_ref().and_then(|d| d.parent_title.clone()));
    let playlist = ui().get_detail_kind() == "playlist";
    let queue: Vec<Track> = with_state(|s| {
        s.music
            .as_ref()
            .map(|q| {
                q.items
                    .iter()
                    .enumerate()
                    .map(|(i, t)| Track {
                        id: t.rating_key.clone().into(),
                        number: (i + 1).to_string().into(),
                        title: t.title.clone().into(),
                        artist: t.original_title.clone().or_else(|| t.grandparent_title.clone()).unwrap_or_default().into(),
                        runtime: t.duration.map(|d| clock(d as f64 / 1000.0)).unwrap_or_default().into(),
                        playing: i == q.index,
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    with_models(|m| m.queue.set_vec(queue));
    ui().set_music_queue_index(with_state(|s| s.music.as_ref().map_or(-1, |q| q.index as i32)));
    with_models(|m| {
        for i in 0..m.songs.row_count() {
            if let Some(mut r) = m.songs.row_data(i) {
                let now = playing.as_deref() == Some(r.id.as_str());
                if r.playing != now {
                    r.playing = now;
                    m.songs.set_row_data(i, r);
                }
            }
        }
    });
    let rows: Vec<Track> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| Track {
            id: t.rating_key.clone().into(),
            number: if playlist { (i + 1).to_string() } else { t.index.map(|n| n.to_string()).unwrap_or_default() }.into(),
            title: t.title.clone().into(),
            // Only show the artist when it differs from the album's (features, compilations);
            // a playlist's songs are by anyone.
            artist: if playlist {
                t.original_title.clone().or_else(|| t.grandparent_title.clone()).unwrap_or_default().into()
            } else {
                t.original_title.clone().filter(|a| Some(a) != album_artist.as_ref()).unwrap_or_default().into()
            },
            runtime: t.duration.map(|d| clock(d as f64 / 1000.0)).unwrap_or_default().into(),
            playing: playing.as_deref() == Some(t.rating_key.as_str()),
        })
        .collect();
    with_models(|m| m.tracks.set_vec(rows));
}

fn music_close() {
    let Some(p) = player() else { return };
    let st = p.status();
    if let Some(track) = with_state(|s| s.music.take().and_then(|q| q.items.get(q.index).cloned())) {
        report_item(track, PlayState::Stopped, st.position);
    }
    p.stop();
    ui().set_music_active(false);
    refresh_tracks();
    leave_music_bar();
    #[cfg(target_os = "android")]
    android_media::stop();
    if ui().get_now_playing_open() {
        now_playing(false);
    }
}

/// Open or close Now Playing. The remote focus goes to Play/Pause, and back where it was after.
fn now_playing(open: bool) {
    let ui = ui();
    let nav = ui.global::<Nav>();
    if open == ui.get_now_playing_open() {
        return;
    }
    if open {
        if !ui.get_music_active() {
            return;
        }
        if nav.get_zone() != "music" && nav.get_zone() != "np" {
            with_state(|s| s.music_return = Some((nav.get_zone(), nav.get_row(), nav.get_col())));
        }
        ui.set_now_playing_open(true);
        if nav.get_active() {
            set_focus(&ui, "np", 0, 1);
        }
    } else {
        ui.set_now_playing_open(false);
        if nav.get_zone() == "np" {
            match with_state(|s| s.music_return.take()) {
                Some((zone, row, col)) => set_focus(&ui, &zone, row, col),
                None => set_focus(&ui, "content", 0, 0),
            }
        }
    }
}

/// Now Playing: start this queued song.
fn music_jump(id: &str) {
    let i = with_state(|s| s.music.as_ref().and_then(|q| q.items.iter().position(|t| t.rating_key == id)));
    if let (Some(i), Some(p)) = (i, player()) {
        p.set_playlist_pos(i);
        if p.status().paused {
            p.toggle_pause();
        }
    }
}

/// Remote focus on Now Playing: row 0 the controls, then the queue.
fn np_nav(ui: &AppWindow, row: i32, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    // Below the controls: the queue, or the lines of synced lyrics (plain ones can't be picked).
    let lyrics = ui.get_lyrics_open();
    let rows = if !lyrics {
        with_models(|m| m.queue.row_count()) as i32
    } else if ui.get_music_lyrics_state() == "synced" {
        ui.get_music_lyrics().row_count() as i32
    } else {
        0
    };
    if enter {
        if row == 0 {
            match col {
                0 => ui.invoke_music_prev(),
                1 => ui.invoke_music_toggle(),
                2 => ui.invoke_music_next(),
                3 => cycle_repeat(),
                _ => {
                    ui.set_lyrics_open(!lyrics);
                    music_tick(ui);
                }
            }
        } else if lyrics {
            // Sing from this line.
            if let Some(l) = ui.get_music_lyrics().row_data(row as usize - 1) {
                ui.invoke_music_seek_to(l.time);
                set_focus(ui, "np", 0, 1);
            }
        } else if let Some(t) = with_models(|m| m.queue.row_data(row as usize - 1)) {
            music_jump(&t.id);
        }
    } else if row == 0 && is(Key::LeftArrow) {
        set_focus(ui, "np", 0, (col - 1).max(0));
    } else if row == 0 && is(Key::RightArrow) {
        set_focus(ui, "np", 0, (col + 1).min(4));
    } else if is(Key::DownArrow) && row < rows {
        // Into the queue at the song that's playing; into the lyrics at the line being sung.
        let here = if lyrics { ui.get_music_lyric_index().max(0) } else { with_state(|s| s.music.as_ref().map_or(0, |q| q.index as i32)) };
        let start = if row == 0 { here + 1 } else { row + 1 };
        set_focus(ui, "np", start.min(rows), 0);
    } else if is(Key::UpArrow) && row > 0 {
        set_focus(ui, "np", row - 1, if row == 1 { 1 } else { 0 });
    }
    true
}

/// Repeat: off → the whole queue → this track → off.
fn cycle_repeat() {
    let mode = with_state(|s| {
        s.music_repeat = (s.music_repeat + 1) % 3;
        s.music_repeat
    });
    ui().set_music_repeat(mode as i32);
    if let Some(p) = player() {
        p.set_repeat(mode);
    }
}

/// Remote: Down from the last row of a page moves to the music bar (when music plays). Returns
/// whether it did.
fn enter_music_bar(ui: &AppWindow, zone: &str, row: i32, col: i32) -> bool {
    if !ui.get_music_active() || ui.get_screen() == "player" {
        return false;
    }
    with_state(|s| s.music_return = Some((zone.into(), row, col)));
    set_focus(ui, "music", 0, 2);
    true
}

/// The remote focus goes back to where it was before the music bar.
fn leave_music_bar() {
    let ui = ui();
    if ui.global::<Nav>().get_zone() != "music" {
        return;
    }
    match with_state(|s| s.music_return.take()) {
        Some((zone, row, col)) => set_focus(&ui, &zone, row, col),
        None => set_focus(&ui, "content", 0, 0),
    }
}

/// Remote focus on the music bar: the song (opens Now Playing), Previous, Play/Pause, Next,
/// Repeat, Close.
fn music_nav(ui: &AppWindow, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    if !ui.get_music_active() {
        leave_music_bar();
        return true;
    }
    if enter {
        match col {
            0 => now_playing(true),
            1 => ui.invoke_music_prev(),
            2 => ui.invoke_music_toggle(),
            3 => ui.invoke_music_next(),
            4 => cycle_repeat(),
            _ => music_close(),
        }
    } else if is(Key::LeftArrow) {
        set_focus(ui, "music", 0, (col - 1).max(0));
    } else if is(Key::RightArrow) {
        set_focus(ui, "music", 0, (col + 1).min(5));
    } else if is(Key::UpArrow) {
        leave_music_bar();
    }
    true
}

fn poke() {
    with_state(|s| s.last_poke = Some(Instant::now()));
    let ui = ui();
    if !ps(&ui).get_controls_visible() {
        ps(&ui).set_controls_visible(true);
    }
}

// ---------- Keyboard / remote navigation ----------

#[derive(Clone, Copy, PartialEq)]
enum DetailRow {
    Actions,
    Seasons,
    Episodes,
    Albums,
    Tracks,
    Related,
}

/// The focusable rows of the details page, in screen order: (kind, items in the row). Episodes and
/// tracks are one row each per item; their first row index is what the UI needs.
fn detail_rows(ui: &AppWindow) -> Vec<(DetailRow, usize)> {
    let actions = usize::from(!ui.get_play_label().is_empty())
        + usize::from(ui.get_detail_can_shuffle())
        + usize::from(ui.get_detail_can_trailer())
        + usize::from(ui.get_detail_can_restart())
        + usize::from(ui.get_detail_can_mark_watched())
        + usize::from(ui.get_detail_can_favourite());
    let kind = ui.get_detail_kind();
    let (seasons, episodes, albums, tracks, related) = with_models(|m| {
        (m.seasons.row_count(), m.episodes.row_count(), m.albums.row_count(), m.tracks.row_count(), m.related.row_count())
    });
    let mut rows = vec![(DetailRow::Actions, actions)];
    if ui.get_detail_is_show() {
        rows.push((DetailRow::Seasons, seasons));
        rows.extend(std::iter::repeat_n((DetailRow::Episodes, 1), episodes));
    }
    if kind == "artist" {
        rows.push((DetailRow::Albums, albums));
    }
    if kind == "album" || kind == "playlist" {
        rows.extend(std::iter::repeat_n((DetailRow::Tracks, 1), tracks));
    }
    rows.push((DetailRow::Related, related));
    // Tell the UI which row index each section starts at.
    let nav = ui.global::<Nav>();
    let first = |k: DetailRow| rows.iter().position(|(r, _)| *r == k).map_or(-1, |i| i as i32);
    nav.set_d_seasons(first(DetailRow::Seasons));
    nav.set_d_episodes(first(DetailRow::Episodes));
    nav.set_d_albums(first(DetailRow::Albums));
    nav.set_d_tracks(first(DetailRow::Tracks));
    nav.set_d_related(first(DetailRow::Related));
    rows
}

/// Arrow keys and Enter on the home and details screens. Returns whether the key was used.
fn nav_key(key: &str) -> bool {
    use slint::platform::Key;
    let k = |key_: Key| SharedString::from(key_);
    let (up, down, left, right) = (k(Key::UpArrow), k(Key::DownArrow), k(Key::LeftArrow), k(Key::RightArrow));
    let enter = key == "\n" || key == "\r";
    let arrow = [&up, &down, &left, &right].iter().any(|a| a.as_str() == key);
    let ui = ui();
    if !arrow && !enter && tv_search_page(&ui) && !ui.get_sources_open() && !ui.get_about_open() {
        if key == SharedString::from(Key::Backspace).as_str() {
            tv_key("delete");
            return true;
        }
        let mut chars = key.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && (c.is_alphanumeric() || c == ' ')
        {
            tv_key(if c == ' ' { "space".to_string() } else { c.to_lowercase().to_string() }.as_str());
            return true;
        }
    }
    if !arrow && !enter {
        return false;
    }
    let nav = ui.global::<Nav>();
    // Windows over the page: About closes with OK; the add/edit form has text fields, which take
    // the keys themselves; Settings has its own focus.
    if ui.get_about_open() {
        let col = ui.get_about_col();
        if enter && col == 0 {
            open_notices();
        } else if enter {
            ui.set_about_open(false);
        } else if key == left.as_str() || key == right.as_str() {
            ui.set_about_col(if key == left.as_str() { 0 } else { 1 });
        }
        return true;
    }
    if ui.get_add_source_open() {
        return false;
    }
    if ui.get_sources_open() {
        nav.set_active(true);
        let (zone, row, col) = (nav.get_zone(), nav.get_row(), nav.get_col());
        return settings_nav(&ui, &zone, row, col, key, enter);
    }
    if !nav.get_active() {
        // First key press only reveals the focus, on the content (never straight into search).
        nav.set_active(true);
        if tv_search_page(&ui) {
            set_focus(&ui, "keys", 0, 0);
        } else if ui.get_screen() == "home" && nav.get_zone() != "content" {
            set_focus(&ui, "content", 0, 0);
        }
        return true;
    }
    let (zone, row, col) = (nav.get_zone(), nav.get_row(), nav.get_col());
    if zone == "music" {
        return music_nav(&ui, col, key, enter);
    }
    if ui.get_now_playing_open() {
        if zone != "np" {
            set_focus(&ui, "np", 0, 1);
            return true;
        }
        return np_nav(&ui, row, col, key, enter);
    }
    match ui.get_screen().as_str() {
        "home" => home_nav(&ui, &zone, row, col, key, enter),
        "detail" => detail_nav(&ui, row, col, key, enter),
        _ => false,
    }
}

fn set_focus(ui: &AppWindow, zone: &str, row: i32, col: i32) {
    let nav = ui.global::<Nav>();
    nav.set_zone(zone.into());
    nav.set_row(row);
    nav.set_col(col);
}

fn home_nav(ui: &AppWindow, zone: &str, row: i32, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let libraries = with_models(|m| m.libraries.row_count()) as i32;
    let selected = ui.get_selected_library();
    let compact = ui.window().size().to_logical(ui.window().scale_factor()).width < 720.0;

    if zone == "side" {
        let last = libraries + 2; // 0 search, 1 Home, 2.. libraries, then Sources
        if enter {
            match row {
                0 if ui.global::<Theme>().get_tv() => open_tv_search(),
                0 => ui.global::<Nav>().set_search_request(ui.global::<Nav>().get_search_request() + 1),
                1 => select_library(-1),
                r if r == last => shares::open_sources(),
                r => select_library(r - 2),
            }
            if row > 0 && row != last {
                set_focus(ui, "side", row, 0);
            }
        } else if is(Key::UpArrow) {
            set_focus(ui, "side", (row - 1).max(0), 0);
        } else if is(Key::DownArrow) {
            set_focus(ui, "side", (row + 1).min(last), 0);
        } else if is(Key::RightArrow) {
            if tv_search_page(ui) {
                set_focus(ui, "keys", 0, 0);
            } else {
                set_focus(ui, "content", 0, 0);
            }
        }
        return true;
    }
    if zone == "keys" {
        return keys_nav(ui, row, col, key, enter);
    }

    let to_side = |ui: &AppWindow| {
        if tv_search_page(ui) {
            set_focus(ui, "keys", 0, 5);
        } else if !compact {
            set_focus(ui, "side", if selected >= 0 { selected + 2 } else { 1 }, 0);
        }
    };

    if selected == -1 {
        // The billboard (Play, More Info), continue watching (if any), then one row per library.
        let billboard = ui.get_billboard().id;
        let lens: Vec<(i32, Vec<SharedString>)> = with_models(|m| {
            let mut v = Vec::new();
            if !billboard.is_empty() {
                v.push((2, vec![]));
            }
            if m.continue_watching.row_count() > 0 {
                v.push((m.continue_watching.row_count() as i32, m.continue_watching.iter().map(|p| p.id).collect()));
            }
            for r in m.home_rows.iter() {
                v.push((r.items.row_count() as i32, r.items.iter().map(|p| p.id).collect()));
            }
            v
        });
        if lens.is_empty() {
            return false;
        }
        let row = row.clamp(0, lens.len() as i32 - 1);
        let col = col.clamp(0, (lens[row as usize].0 - 1).max(0));
        if enter && row == 0 && !billboard.is_empty() {
            if col == 0 { play_billboard() } else { open_item(billboard.to_string()) }
        } else if enter {
            if let Some(id) = lens[row as usize].1.get(col as usize) {
                open_item(id.to_string());
            }
        } else if is(Key::UpArrow) {
            let r = (row - 1).max(0);
            set_focus(ui, "content", r, col.min(lens[r as usize].0 - 1));
        } else if is(Key::DownArrow) && row + 1 >= lens.len() as i32 {
            enter_music_bar(ui, "content", row, col);
        } else if is(Key::DownArrow) {
            let r = (row + 1).min(lens.len() as i32 - 1);
            set_focus(ui, "content", r, col.min(lens[r as usize].0 - 1));
        } else if is(Key::LeftArrow) {
            if col == 0 { to_side(ui) } else { set_focus(ui, "content", row, col - 1) }
        } else if is(Key::RightArrow) {
            let len = lens[row as usize].0;
            let bb = i32::from(!billboard.is_empty());
            let cw = i32::from(with_models(|m| m.continue_watching.row_count() > 0));
            if col + 1 < len {
                set_focus(ui, "content", row, col + 1);
            } else if row >= bb + cw && row_has_more((row - bb - cw) as usize) {
                // The next titles are on their way; they extend the row.
                more_in_row((row - bb - cw) as usize);
            } else if row >= bb && len > 1 {
                // The end of a row: back to its start, as on Netflix.
                set_focus(ui, "content", row, 0);
            }
        }
        return true;
    }

    // A library grid or search results.
    let has_tabs = selected >= 0 && ui.get_grid_tabs().row_count() > 0;
    if zone == "tabs" {
        let n = ui.get_grid_tabs().row_count() as i32;
        if enter {
            select_tab(col);
        } else if is(Key::LeftArrow) {
            if col == 0 { to_side(ui) } else { set_focus(ui, "tabs", 0, col - 1) }
        } else if is(Key::RightArrow) {
            set_focus(ui, "tabs", 0, (col + 1).min(n - 1));
        } else if is(Key::DownArrow) && with_models(|m| m.grid.row_count()) > 0 {
            set_focus(ui, "content", 0, 0);
        }
        return true;
    }
    let ids: Vec<SharedString> = with_models(|m| m.grid.iter().map(|p| p.id).collect());
    if ids.is_empty() {
        if is(Key::LeftArrow) {
            to_side(ui);
        } else if is(Key::UpArrow) && has_tabs {
            set_focus(ui, "tabs", 0, ui.get_grid_tab());
        }
        return true;
    }
    let cols = ui.global::<Nav>().get_grid_cols().max(1);
    let last = ids.len() as i32 - 1;
    let idx = (row * cols + col).clamp(0, last);
    let go = |ui: &AppWindow, i: i32| set_focus(ui, "content", i / cols, i % cols);
    if enter {
        open_item(ids[idx as usize].to_string());
    } else if is(Key::UpArrow) && idx < cols && has_tabs {
        set_focus(ui, "tabs", 0, ui.get_grid_tab());
    } else if is(Key::UpArrow) {
        go(ui, if idx >= cols { idx - cols } else { idx });
    } else if is(Key::DownArrow) && idx / cols == last / cols {
        enter_music_bar(ui, "content", row, col);
    } else if is(Key::DownArrow) {
        go(ui, (idx + cols).min(last));
    } else if is(Key::LeftArrow) {
        if idx % cols == 0 { to_side(ui) } else { go(ui, idx - 1) }
    } else if is(Key::RightArrow) {
        go(ui, (idx + 1).min(last));
    }
    true
}

/// Remote focus in Settings: one row per source (Edit/Remove, or Sign In/Sign Out for Plex, or
/// Cancel/Remove while it asks), then Refresh, About, Add Source.
fn settings_nav(ui: &AppWindow, zone: &str, row: i32, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    if zone != "dialog" {
        with_state(|s| s.dialog_return = Some((zone.into(), row, col)));
        set_focus(ui, "dialog", 0, 0);
        return true;
    }
    let sources: Vec<SourceRow> = ui.get_sources().iter().collect();
    let confirm = ui.get_sources_confirm();
    // The sources, the Animations switch, (Android) the HDR switch, then the buttons.
    let switch_row = sources.len() as i32;
    let direct_row = if ui.get_direct_info().is_empty() { -1 } else { switch_row + 1 };
    let width = |r: i32| -> i32 {
        match sources.get(r as usize) {
            Some(_) if r == confirm => 2,
            Some(s) if s.kind == "plex" && s.signed_in && ui.get_can_switch_profile() => 2,
            Some(s) if s.kind == "plex" => 1,
            Some(_) => 2,
            None if r == switch_row || r == direct_row => 1,
            None if ui.get_can_tv_mode() => 4,
            None => 3,
        }
    };
    let last = sources.len() as i32 + if direct_row < 0 { 1 } else { 2 };
    let row = row.clamp(0, last);
    let col = col.clamp(0, width(row) - 1);
    if enter {
        match sources.get(row as usize) {
            Some(src) if row == confirm => {
                ui.set_sources_confirm(-1);
                if col == 1 {
                    shares::remove_source(&src.kind, src.index as usize);
                    set_focus(ui, "dialog", 0, 0);
                }
            }
            Some(src) if src.kind == "plex" && !src.signed_in => {
                ui.set_sources_open(false);
                begin_signin();
            }
            Some(src) if src.kind != "plex" && col == 0 => shares::edit_source(&src.kind, src.index as usize),
            Some(src) if src.kind == "plex" && col == 0 && ui.get_can_switch_profile() => {
                settings_closed();
                profiles::open();
            }
            // Remove / Sign Out ask first; the focus starts on Cancel.
            Some(_) => {
                ui.set_sources_confirm(row);
                set_focus(ui, "dialog", row, 0);
            }
            None if row == switch_row => toggle_animations(),
            None if row == direct_row => toggle_direct(),
            None => match col {
                0 => {
                    ui.set_sources_open(false);
                    ui.invoke_refresh();
                }
                1 => ui.set_about_open(true),
                2 if ui.get_can_tv_mode() => toggle_tv_mode(),
                _ => shares::new_source(),
            },
        }
    } else if is(Key::UpArrow) || is(Key::DownArrow) {
        let r = (row + if is(Key::UpArrow) { -1 } else { 1 }).clamp(0, last);
        if r != row && confirm >= 0 {
            ui.set_sources_confirm(-1);
        }
        set_focus(ui, "dialog", r, col.min(width(r) - 1));
    } else if is(Key::LeftArrow) {
        set_focus(ui, "dialog", row, (col - 1).max(0));
    } else if is(Key::RightArrow) {
        set_focus(ui, "dialog", row, (col + 1).min(width(row) - 1));
    }
    true
}

fn detail_nav(ui: &AppWindow, row: i32, col: i32, key: &str, enter: bool) -> bool {
    use slint::platform::Key;
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let rows = detail_rows(ui);
    let usable = |r: i32| rows.get(r as usize).is_some_and(|(_, n)| *n > 0);
    // Start on the first row that has something in it.
    let mut row = row.clamp(0, rows.len() as i32 - 1);
    if !usable(row) {
        row = (0..rows.len() as i32).find(|r| usable(*r)).unwrap_or(0);
    }
    let len = rows[row as usize].1 as i32;
    let col = col.clamp(0, (len - 1).max(0));
    // Wide layouts put Play above the other actions (Trailer, watched, …): Down and Up move
    // between them before leaving the row.
    let wide = ui.window().size().to_logical(ui.window().scale_factor()).width >= 720.0;
    // (Albums and artists have their buttons in one row.)
    let music = matches!(ui.get_detail_kind().as_str(), "album" | "artist" | "playlist");
    let play_on_top = wide && !music && rows[row as usize].0 == DetailRow::Actions && !ui.get_play_label().is_empty() && len > 1;
    if enter {
        detail_activate(ui, rows[row as usize].0, row, col, &rows);
    } else if play_on_top && is(Key::DownArrow) && col == 0 {
        set_focus(ui, "detail", row, 1);
    } else if play_on_top && is(Key::UpArrow) && col > 0 {
        set_focus(ui, "detail", row, 0);
    } else if is(Key::UpArrow) || is(Key::DownArrow) {
        let step = if is(Key::UpArrow) { -1 } else { 1 };
        let mut r = row + step;
        while r >= 0 && (r as usize) < rows.len() && !usable(r) {
            r += step;
        }
        if r >= 0 && (r as usize) < rows.len() {
            set_focus(ui, "detail", r, col.min(rows[r as usize].1 as i32 - 1));
        } else if step == 1 {
            enter_music_bar(ui, "detail", row, col);
        }
    } else if is(Key::LeftArrow) {
        set_focus(ui, "detail", row, (col - 1).max(0));
    } else if is(Key::RightArrow) {
        set_focus(ui, "detail", row, (col + 1).min(len - 1));
    }
    true
}

fn detail_activate(ui: &AppWindow, kind: DetailRow, row: i32, col: i32, rows: &[(DetailRow, usize)]) {
    let first_of = |k: DetailRow| rows.iter().position(|(r, _)| *r == k).unwrap_or(0) as i32;
    match kind {
        DetailRow::Actions => {
            let mut actions: Vec<fn()> = Vec::new();
            if !ui.get_play_label().is_empty() {
                actions.push(play_detail);
            }
            if ui.get_detail_can_shuffle() {
                actions.push(shuffle_detail);
            }
            if ui.get_detail_can_trailer() {
                actions.push(play_trailer);
            }
            if ui.get_detail_can_restart() {
                actions.push(play_from_start);
            }
            if ui.get_detail_can_mark_watched() {
                actions.push(toggle_watched);
            }
            if ui.get_detail_can_favourite() {
                actions.push(toggle_favourite);
            }
            if let Some(f) = actions.get(col as usize) {
                f();
            }
        }
        DetailRow::Seasons => select_season(col),
        DetailRow::Episodes => {
            let id = with_models(|m| m.episodes.row_data((row - first_of(DetailRow::Episodes)) as usize).map(|e| e.id));
            if let Some(item) = id.and_then(|id| with_state(|s| s.items.get(id.as_str()).cloned())) {
                play(item);
            }
        }
        DetailRow::Tracks => {
            let id = with_models(|m| m.tracks.row_data((row - first_of(DetailRow::Tracks)) as usize).map(|t| t.id));
            if let Some(id) = id {
                ui.invoke_play_track(id);
            }
        }
        DetailRow::Albums => {
            if let Some(p) = with_models(|m| m.albums.row_data(col as usize)) {
                open_item(p.id.to_string());
            }
        }
        DetailRow::Related => {
            if let Some(p) = with_models(|m| m.related.row_data(col as usize)) {
                open_item(p.id.to_string());
            }
        }
    }
}

// ---------- Keys handled below Slint ----------

/// Sees winit's events before Slint does. Escape is handled here because on macOS AppKit swallows
/// its key-down: only the key-up reaches the window. So we act on the press when it arrives (other
/// platforms) and on the release when it didn't (macOS).
#[cfg(not(target_os = "android"))]
#[derive(Default)]
struct KeyTap {
    escape_down: bool,
}

#[cfg(not(target_os = "android"))]
impl slint::winit_030::CustomApplicationHandler for KeyTap {
    fn window_event(
        &mut self,
        _event_loop: &slint::winit_030::winit::event_loop::ActiveEventLoop,
        _window_id: slint::winit_030::winit::window::WindowId,
        _winit_window: Option<&slint::winit_030::winit::window::Window>,
        _slint_window: Option<&slint::Window>,
        event: &slint::winit_030::winit::event::WindowEvent,
    ) -> slint::winit_030::EventResult {
        use slint::winit_030::winit::{event::{ElementState, WindowEvent}, keyboard::{Key, NamedKey}};
        if let WindowEvent::KeyboardInput { event, .. } = event {
            if std::env::var("ENCHEO_DEBUG_KEYS").is_ok_and(|v| !v.is_empty()) {
                eprintln!("winit key: {:?} {:?}", event.logical_key, event.state);
            }
            if event.logical_key == Key::Named(NamedKey::Escape) {
                match event.state {
                    ElementState::Pressed if !event.repeat => {
                        self.escape_down = true;
                        escape();
                    }
                    ElementState::Released if !std::mem::take(&mut self.escape_down) => escape(),
                    _ => {}
                }
                return slint::winit_030::EventResult::PreventDefault;
            }
        }
        slint::winit_030::EventResult::Propagate
    }
}

/// The third-party notices (scripts/notices.sh), in pieces a list can show without laying out
/// 650 KB of text at once. Built only while they're open.
const NOTICES: &str = include_str!("../notices.txt");

fn open_notices() {
    let ui = ui();
    ui.set_notices(ModelRc::new(VecModel::from(notice_pieces(NOTICES))));
    ui.set_notices_open(true);
}

fn notice_pieces(text: &str) -> Vec<SharedString> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for paragraph in text.split("\n\n") {
        if !piece.is_empty() && piece.len() + paragraph.len() > 2000 {
            pieces.push(SharedString::from(std::mem::take(&mut piece)));
        }
        if !piece.is_empty() {
            piece.push_str("\n\n");
        }
        piece.push_str(paragraph);
    }
    if !piece.is_empty() {
        pieces.push(piece.into());
    }
    pieces
}

/// Settings opened: with the remote in use, the focus moves straight into it.
pub(crate) fn focus_settings() {
    let ui = ui();
    let nav = ui.global::<Nav>();
    if nav.get_active() && nav.get_zone() != "dialog" {
        with_state(|s| s.dialog_return = Some((nav.get_zone(), nav.get_row(), nav.get_col())));
        set_focus(&ui, "dialog", 0, 0);
    }
}

/// Settings closed: the focus goes back to where it was.
fn settings_closed() {
    let ui = ui();
    if ui.global::<Nav>().get_zone() == "dialog" {
        let (zone, row, col) = with_state(|s| s.dialog_return.take()).unwrap_or(("content".into(), 0, 0));
        set_focus(&ui, &zone, row, col);
    }
}

/// Back on Home: windows first (About, the form, Settings with a pending "Remove?" first), then
/// search results and library pages go back to Home. False when there's nothing to go back to.
fn go_back() -> bool {
    if profiles::back() {
        return true;
    }
    let ui = ui();
    if ui.get_notices_open() {
        ui.set_notices_open(false);
        ui.set_notices(ModelRc::default());
        return true;
    }
    if ui.global::<Nav>().get_zone() == "music" {
        leave_music_bar();
        return true;
    }
    if ui.get_now_playing_open() {
        now_playing(false);
        return true;
    }
    if ui.get_about_open() {
        ui.set_about_open(false);
    } else if ui.get_add_source_open() {
        ui.set_add_source_open(false);
    } else if ui.get_sources_open() {
        if ui.get_sources_confirm() >= 0 {
            ui.set_sources_confirm(-1);
        } else {
            ui.set_sources_open(false);
        }
    } else if tv_search_page(&ui) && ui.global::<Nav>().get_zone() == "content" {
        set_focus(&ui, "keys", 0, 0);
    } else if ui.get_screen() == "home" && ui.get_selected_library() == -2 {
        ui.set_search_text(SharedString::new());
        enter_home();
    } else if ui.get_screen() == "home" && (ui.get_selected_library() >= 0 || ui.get_selected_library() == COLLECTION_PAGE) {
        select_library(-1);
    } else if ui.get_screen() == "home" && ui.global::<Nav>().get_active() {
        // Like TV apps: Back goes to the top of Home, then to the menu, and only then leaves.
        let nav = ui.global::<Nav>();
        match (nav.get_zone().as_str(), nav.get_row(), nav.get_col()) {
            ("content", 0, 0) if ui.window().size().to_logical(ui.window().scale_factor()).width >= 720.0 => {
                set_focus(&ui, "side", 1, 0)
            }
            ("content", _, _) if (nav.get_row(), nav.get_col()) != (0, 0) => set_focus(&ui, "content", 0, 0),
            _ => return false,
        }
    } else {
        return false;
    }
    true
}

#[cfg(not(target_os = "android"))]
/// Escape backs out one step: windows → panel → fullscreen → player → details → search.
fn escape() {
    if go_back() {
        return;
    }
    let ui = ui();
    let ps = ps(&ui);
    match ui.get_screen().as_str() {
        "player" => {
            if !ps.get_panel().is_empty() {
                ps.set_panel(SharedString::new());
            } else if ps.get_fullscreen() && !ui.global::<Theme>().get_tv() {
                // TV mode stays full screen; Escape leaves the film instead.
                toggle_fullscreen();
            } else {
                stop_playback();
            }
        }
        "detail" => ui.set_screen("home".into()),
        "home" if ui.get_selected_library() == -2 => {
            ui.set_search_text(SharedString::new());
            enter_home();
        }
        _ => {}
    }
}

// ---------- Wiring ----------

/// Android entry point: hand the activity to Slint, tell libmpv's decoders about the Java VM
/// (needed for MediaCodec hardware decoding), keep data in the app's private folder, then run.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    unsafe extern "C" {
        fn av_jni_set_java_vm(vm: *mut std::ffi::c_void, log_ctx: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    // SAFETY: the VM pointer comes from the running activity and outlives the process.
    unsafe {
        av_jni_set_java_vm(app.vm_as_ptr(), std::ptr::null_mut());
    }
    if let Some(dir) = app.internal_data_path() {
        session::set_data_dir(dir);
    }
    let _ = ANDROID_APP.set(app.clone());
    if let Err(e) = slint::android::init(app) {
        eprintln!("slint init: {e}");
        return;
    }
    if let Err(e) = run() {
        eprintln!("encheo: {e}");
    }
    // The app closed: its music stopped with it.
    android_media::stop();
}

#[cfg(target_os = "android")]
static ANDROID_APP: OnceLock<slint::android::AndroidApp> = OnceLock::new();

/// Close the on-screen keyboard. Android TV opens it on start (Slint's text-input view is the
/// first focusable view) and when a text field loses focus it can linger.
fn hide_keyboard() {
    #[cfg(target_os = "android")]
    if let Some(app) = ANDROID_APP.get() {
        app.hide_soft_input(false);
    }
}

/// Android TV: density-based scaling makes a 1080p TV only 960 points wide, which is phone-sized
/// UI from across the room is huge. TVs get a scale that makes the screen 1280 points wide,
/// whatever its resolution. Slint resets the scale when the window is recreated, so this is
/// checked regularly.
fn fit_tv_scale(ui: &AppWindow) {
    #[cfg(target_os = "android")]
    {
        let Some(app) = ANDROID_APP.get() else { return };
        if app.config().ui_mode_type() != slint::android::android_activity::ndk::configuration::UiModeType::Television {
            return;
        }
        let width = ui.window().size().width as f32;
        if width <= 0.0 {
            return;
        }
        if !ui.global::<Theme>().get_tv() {
            ui.global::<Theme>().set_tv(true);
            // Home may have loaded before the TV was recognised: TV rows have wide cards.
            if ui.get_screen() == "home" && ui.get_selected_library() == -1 && !ui.get_loading() {
                enter_home();
            }
        }
        let want = (width / 1280.0).max(1.0);
        if (ui.window().scale_factor() - want).abs() > 0.01 {
            set_scale(ui, want);
        }
    }
    // Desktop TV mode: the same 1280-point-wide layout, so it reads from the sofa too.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let native = NATIVE_SCALE.get();
        if ui.global::<Theme>().get_tv() {
            let width = ui.window().size().width as f32;
            if width <= 0.0 {
                return;
            }
            let native = native.unwrap_or_else(|| {
                let n = ui.window().scale_factor();
                NATIVE_SCALE.set(Some(n));
                n
            });
            let want = (width / 1280.0).max(native);
            if (ui.window().scale_factor() - want).abs() > 0.01 {
                set_scale(ui, want);
            }
        } else if let Some(n) = native {
            NATIVE_SCALE.set(None);
            if (ui.window().scale_factor() - n).abs() > 0.01 {
                set_scale(ui, n);
            }
        }
    }
    #[cfg(target_os = "ios")]
    let _ = ui;
}

/// A new scale, and the window's size in points to go with it (the layout follows the size;
/// without it a layout made at the old scale can stay, drawn too small).
#[cfg(not(target_os = "ios"))]
fn set_scale(ui: &AppWindow, scale: f32) {
    use slint::platform::WindowEvent;
    ui.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    ui.window().dispatch_event(WindowEvent::Resized { size: ui.window().size().to_logical(scale) });
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
thread_local! {
    /// The screen's own scale, from before TV mode enlarged it.
    static NATIVE_SCALE: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// Desktop: TV mode on or off. TV mode is full screen; it's remembered for the next start.
/// The decoder draws HDR video onto Android's video layer: mpv doesn't draw into Slint's frame.
fn direct_video() -> bool {
    #[cfg(target_os = "android")]
    return android_video::direct();
    #[cfg(not(target_os = "android"))]
    false
}

/// Android: HDR and surround passthrough on or off (Settings). Applies from the next moment of
/// playback: the video's path is chosen again four times a second.
fn toggle_direct() {
    let ui = ui();
    let on = !ui.get_direct_on();
    ui.set_direct_on(on);
    with_state(|s| {
        s.session.prefs.direct = on;
        s.session.save();
    });
    #[cfg(target_os = "android")]
    if let Some(p) = player() {
        p.set_spdif(&android_video::spdif(on));
    }
}

fn toggle_animations() {
    let ui = ui();
    let on = !ui.global::<Theme>().get_animate();
    ui.global::<Theme>().set_animate(on);
    with_state(|s| {
        s.session.prefs.animations = on;
        s.session.save();
    });
}

fn toggle_tv_mode() {
    let ui = ui();
    let on = !ui.global::<Theme>().get_tv();
    ui.global::<Theme>().set_tv(on);
    ui.window().set_fullscreen(on);
    ps(&ui).set_fullscreen(on);
    ui.set_sources_open(false);
    with_state(|s| {
        s.session.prefs.tv_mode = on;
        s.session.save();
    });
    if on {
        ui.global::<Nav>().set_active(true);
        set_focus(&ui, "content", 0, 0);
    }
    if ui.get_screen() == "home" && ui.get_selected_library() == -1 {
        enter_home(); // TV rows have wide cards
    }
}

/// Start the app and run until its window closes.
pub fn run() -> Result<(), slint::PlatformError> {
    // The video is drawn with OpenGL under the UI, so the UI must render with OpenGL too.
    #[cfg(target_os = "android")]
    slint::BackendSelector::new().require_opengl_es().select()?;
    #[cfg(not(target_os = "android"))]
    let selector = slint::BackendSelector::new();
    // iOS: Slint draws with Metal there (no OpenGL), so no requirement.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let selector = selector.require_opengl();
    #[cfg(target_os = "ios")]
    if let Some(home) = std::env::var_os("HOME") {
        session::set_data_dir(std::path::PathBuf::from(home).join("Library/Application Support/Encheo"));
    }
    #[cfg(not(target_os = "android"))]
    selector
        .with_winit_window_attributes_hook(|attrs| {
            // macOS look: content runs under a transparent titlebar (the window buttons sit in
            // the sidebar) and the transparent parts of the window get the system blur. Other
            // platforms keep their normal, opaque window frame.
            #[cfg(target_os = "macos")]
            let attrs = {
                use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
                attrs
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true)
                    .with_transparent(true)
                    .with_blur(true)
            };
            attrs
        })
        .with_winit_custom_application_handler(KeyTap::default())
        .select()?;
    // Development: keep everything in another folder (the screenshot script's demo session).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Some(dir) = std::env::var_os("ENCHEO_DATA_DIR") {
        session::set_data_dir(dir.into());
    }
    let ui = AppWindow::new()?;
    // Development: the TV look on a desktop window (ENCHEO_TV=1).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if std::env::var("ENCHEO_TV").is_ok_and(|v| !v.is_empty()) {
        ui.global::<Theme>().set_tv(true);
    }
    ui.global::<Theme>().set_buttons_in_content(cfg!(target_os = "macos"));
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    ui.global::<PlayerState>().set_window_modes(cfg!(not(any(target_os = "android", target_os = "ios"))));
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
    with_state(|s| s.session = Session::load());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        ui.set_can_tv_mode(true);
        if with_state(|s| s.session.prefs.tv_mode) {
            ui.global::<Theme>().set_tv(true);
            ui.window().set_fullscreen(true);
            ui.global::<PlayerState>().set_fullscreen(true);
        }
    }
    shares::sync_configs();
    backend::load_jf_servers();

    let models = Models {
        libraries: Rc::new(VecModel::default()),
        continue_watching: Rc::new(VecModel::default()),
        home_rows: Rc::new(VecModel::default()),
        row_models: RefCell::new(Vec::new()),
        row_more: RefCell::new(Vec::new()),
        grid: Rc::new(VecModel::default()),
        albums: Rc::new(VecModel::default()),
        tracks: Rc::new(VecModel::default()),
        songs: Rc::new(VecModel::default()),
        queue: Rc::new(VecModel::default()),
        people: Rc::new(VecModel::default()),
        related: Rc::new(VecModel::default()),
        seasons: Rc::new(VecModel::default()),
        seasons_watched: Rc::new(VecModel::default()),
        episodes: Rc::new(VecModel::default()),
    };
    ui.set_libraries(ModelRc::from(models.libraries.clone()));
    ui.set_continue_watching(ModelRc::from(models.continue_watching.clone()));
    ui.set_home_rows(ModelRc::from(models.home_rows.clone()));
    ui.set_songs(ModelRc::from(models.songs.clone()));
    ui.set_music_queue(ModelRc::from(models.queue.clone()));
    ui.set_grid(ModelRc::from(models.grid.clone()));
    ui.set_albums(ModelRc::from(models.albums.clone()));
    ui.set_tracks(ModelRc::from(models.tracks.clone()));
    ui.set_people(ModelRc::from(models.people.clone()));
    ui.set_related(ModelRc::from(models.related.clone()));
    ui.set_seasons(ModelRc::from(models.seasons.clone()));
    ui.set_seasons_watched(ModelRc::from(models.seasons_watched.clone()));
    ui.set_episodes(ModelRc::from(models.episodes.clone()));
    MODELS.with(|m| *m.borrow_mut() = Some(models));

    match Player::new() {
        Ok(p) => {
            let prefs = with_state(|s| s.session.prefs.clone());
            p.set_sub_scale(prefs.sub_scale);
            let ps = ui.global::<PlayerState>();
            ps.set_sub_scale(prefs.sub_scale as f32);
            ps.set_auto_skip(prefs.auto_skip);
            // Android: what the TV and sound output take as is, checked once per launch.
            #[cfg(target_os = "android")]
            {
                android_video::init(p.mpv());
                p.set_spdif(&android_video::spdif(prefs.direct));
                ui.set_direct_info(android_video::summary().into());
                ui.set_direct_on(prefs.direct);
            }
            PLAYER.with(|slot| *slot.borrow_mut() = Some(Rc::new(p)));
        }
        Err(e) => eprintln!("mpv failed to start: {e}"),
    }

    let weak = ui.as_weak();
    ui.window()
        .set_rendering_notifier(move |state, api| {
            let Some(p) = player() else { return };
            match state {
                slint::RenderingState::RenderingSetup => {

                    if let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api {
                        let w = weak.clone();
                        p.setup_gl(*get_proc_address, move || {
                            let _ = w.upgrade_in_event_loop(|ui| ui.window().request_redraw());
                        });
                    }
                }
                slint::RenderingState::BeforeRendering => {
                    if let Some(ui) = weak.upgrade()
                        && ui.get_screen() == "player"
                        && !direct_video()
                    {
                        let size = ui.window().size();
                        p.draw(size.width, size.height);
                    }
                }
                slint::RenderingState::RenderingTeardown => p.teardown(),
                _ => {}
            }
        })
        .map_err(|e| slint::PlatformError::Other(format!("rendering notifier: {e:?}")))?;

    ui.on_open_link(|| open_url("https://plex.tv/link"));
    ui.on_billboard_play(play_billboard);
    ui.on_play_trailer(play_trailer);
    ui.on_new_code(begin_signin);
    ui.on_select_library(select_library);
    ui.on_open_item(|id| open_item(id.to_string()));
    ui.on_skip_plex(|| {
        with_state(|s| {
            s.session.skip_plex = true;
            s.session.save();
            s.signin_generation += 1; // stop waiting for the code
        });
        enter_home();
    });
    ui.on_back(|| {
        let ui = crate::ui();
        ui.set_screen("home".into());
        if let Some((zone, row, col)) = with_state(|s| s.home_nav.take()) {
            // The home page is rebuilt when shown; set the focus once it has laid out, so the
            // grid's scroll-into-view sees the change.
            set_focus(&ui, &zone, -1, -1);
            slint::Timer::single_shot(Duration::from_millis(50), move || set_focus(&crate::ui(), &zone, row, col));
        }
    });
    ui.on_play_detail(play_detail);
    ui.on_play_from_start(play_from_start);
    ui.on_toggle_watched(toggle_watched);
    ui.on_toggle_favourite(toggle_favourite);
    ui.on_select_season(select_season);
    ui.on_play_episode(|id| {
        if let Some(item) = with_state(|s| s.items.get(id.as_str()).cloned()) {
            play(item);
        }
    });
    {
        let ps = ui.global::<PlayerState>();
        ps.on_toggle_pause(|| {
            if let Some(p) = player() {
                p.toggle_pause();
            }
        });
        ps.on_seek_to(|t| {
            if let Some(p) = player() {
                p.seek_to(t as f64);
            }
        });
        ps.on_scrub_to(|t| {
            if let Some(p) = player() {
                p.seek_fast(t as f64);
            }
        });
        ps.on_seek_by(|d| {
            if let Some(p) = player() {
                p.seek_by(d as f64);
            }
        });
        ps.on_stop(stop_playback);
        ps.on_skip_marker(skip_marker);
        ps.on_poke(poke);
        ps.on_toggle_controls(|| {
            let ui = crate::ui();
            let ps = crate::ps(&ui);
            if ps.get_controls_visible() {
                ps.set_controls_visible(false);
            } else {
                poke();
            }
        });
        ps.on_set_volume(|v| {
            if let Some(p) = player() {
                p.set_volume(v as f64);
                crate::ps(&crate::ui()).set_volume(v);
                crate::ps(&crate::ui()).set_muted(false);
            }
        });
        ps.on_toggle_mute(|| {
            if let Some(p) = player() {
                p.toggle_mute();
            }
        });
        ps.on_prev(|| step(-1));
        ps.on_next(|| step(1));
        ps.on_toggle_fullscreen(toggle_fullscreen);
        ps.on_toggle_pip(toggle_pip);
        ps.on_open_panel(|which| open_panel(which.as_str()));
        ps.on_select_audio(|id| choose_track(StreamKind::Audio, Some(id as i64)));
        ps.on_select_sub(|id| choose_track(StreamKind::Subtitle, (id >= 0).then_some(id as i64)));
        ps.on_set_speed(|sp| {
            if let Some(p) = player() {
                p.set_speed(sp as f64);
                crate::ps(&crate::ui()).set_speed(sp);
            }
        });
        ps.on_pick(|id| pick(id.to_string()));
        ps.on_find_subs(|lang| find_subtitles(lang.to_string()));
        ps.on_set_sub_scale(|v| {
            if let Some(p) = player() {
                p.set_sub_scale(v as f64);
            }
            crate::ps(&crate::ui()).set_sub_scale(v);
            with_state(|s| {
                s.session.prefs.sub_scale = v as f64;
                s.session.save();
            });
        });
        ps.on_nudge_sub_delay(|d| {
            if let Some(p) = player() {
                let v = ((p.sub_delay() + d as f64) * 10.0).round() / 10.0;
                p.set_sub_delay(v);
                crate::ps(&crate::ui()).set_sub_delay(v as f32);
            }
        });
        ps.on_nudge_audio_delay(|d| {
            if let Some(p) = player() {
                let v = ((p.audio_delay() + d as f64) * 10.0).round() / 10.0;
                p.set_audio_delay(v);
                crate::ps(&crate::ui()).set_audio_delay(v as f32);
            }
        });
        ps.on_set_auto_skip(|on| {
            crate::ps(&crate::ui()).set_auto_skip(on);
            with_state(|s| {
                s.session.prefs.auto_skip = on;
                s.session.save();
            });
        });
    }
    ui.on_play_track(|id| {
        let (tracks, idx) = with_state(|s| {
            let i = s.album_tracks.iter().position(|t| t.rating_key == id.as_str());
            (s.album_tracks.clone(), i)
        });
        if let Some(i) = idx {
            play_music(tracks, i);
        }
    });
    ui.on_music_toggle(|| {
        if let Some(p) = player() {
            p.toggle_pause();
        }
    });
    ui.on_music_next(|| {
        if let Some(p) = player() {
            p.next();
        }
    });
    ui.on_music_prev(|| {
        if let Some(p) = player() {
            // Like every music player: back to the start first, previous track if already there.
            if p.status().position > 3.0 { p.seek_to(0.0) } else { p.prev() }
        }
    });
    ui.on_music_close(music_close);
    ui.on_music_repeat_cycle(cycle_repeat);
    ui.on_shuffle_detail(shuffle_detail);
    ui.on_select_tab(select_tab);
    ui.on_now_playing(now_playing);
    ui.on_lyrics_toggled(|| music_tick(&self::ui()));
    ui.on_music_jump(|id| music_jump(&id));
    ui.on_music_seek_to(|t| {
        if let Some(p) = player() {
            p.seek_to(t as f64);
        }
    });
    ui.on_search(|q| search(q.to_string()));
    ui.on_add_source(|kind, name, location, user, password| {
        shares::add_source(kind, name.to_string(), location.to_string(), user.to_string(), password.to_string())
    });
    ui.on_open_sources(shares::open_sources);
    ui.on_new_source(shares::new_source);
    ui.on_edit_source(|kind, index| shares::edit_source(&kind, index as usize));
    ui.on_remove_source(|kind, index| shares::remove_source(&kind, index as usize));
    ui.on_scan(scan_network);
    ui.on_speed_test(speed_test);
    ui.set_can_speed_test(true);
    ui.global::<Nav>().on_key(|k| nav_key(k.as_str()));
    ui.global::<Nav>().on_back(go_back);
    ui.global::<Nav>().on_dialog_closed(settings_closed);
    ui.global::<Nav>().on_tv_search(open_tv_search);
    ui.global::<Nav>().on_tv_key(|k| tv_key(k.as_str()));
    ui.global::<Nav>().on_leave_search(|| {
        hide_keyboard();
        let ui = crate::ui();
        let nav = ui.global::<Nav>();
        nav.set_refocus_request(nav.get_refocus_request() + 1);
        if nav.get_active() {
            set_focus(&ui, "side", 1, 0);
        }
    });
    ui.on_refresh(refresh);
    ui.on_tv_mode(toggle_tv_mode);
    ui.on_animations_toggled(toggle_animations);
    ui.on_direct_toggled(toggle_direct);
    ui.on_open_notices(open_notices);
    ui.on_notices_closed(|| crate::ui().set_notices(ModelRc::default()));
    ui.on_profile_pick(|i| profiles::pick(i.max(0) as usize));
    ui.on_pin_key(|k| profiles::pin_key(&k));
    ui.on_profile_key(|k| profiles::key(&k));
    ui.on_profiles_back(|| {
        profiles::back();
    });
    ui.on_switch_profile(|| {
        settings_closed();
        profiles::open();
    });
    ui.on_row_more(|ri| more_in_row(ri.max(0) as usize));
    ui.global::<Theme>().set_animate(with_state(|s| s.session.prefs.animations));
    // Posters ask for their pictures while on or near the screen.
    ui.global::<Art>().on_image(|url, _| if url.starts_with("http") { want_image(&url) } else { Image::default() });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), tick);

    resume_session();
    // Android TV pops the keyboard up as the window gets focus; close it once that has happened.
    for ms in [400, 1500] {
        slint::Timer::single_shot(Duration::from_millis(ms), hide_keyboard);
    }
    // Development: ENCHEO_PLAY=<url> plays a URL right after start (simulators and emulators,
    // where nothing can be clicked from scripts).
    if let Ok(url) = std::env::var("ENCHEO_PLAY") {
        slint::Timer::single_shot(Duration::from_secs(2), move || {
            if let Some(p) = player() {
                crate::ui().set_screen("player".into());
                p.load(&url, None);
            }
        });
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Some(dir) = std::env::var_os("ENCHEO_SCREENSHOTS") {
        screenshots::start(dir.into());
    }
    // iOS: give mpv its video layer once the window (and its view) exists.
    #[cfg(target_os = "ios")]
    {
        let timer = Box::leak(Box::new(slint::Timer::default()));
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), || {
            if ios_video::attached() {
                return;
            }
            if let (Some(p), Some(wid)) = (player(), ios_video::attach(crate::ui().window())) {
                p.set_wid(wid);
            }
        });
    }
    ui.run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn notices_split_into_pieces_without_losing_text() {
        let pieces = super::notice_pieces(super::NOTICES);
        assert!(pieces.len() > 50);
        assert_eq!(pieces.iter().map(|p| p.as_str()).collect::<Vec<_>>().join("\n\n"), super::NOTICES);
        assert!(super::NOTICES.contains("FFmpeg") && super::NOTICES.contains("GNU LESSER GENERAL PUBLIC LICENSE"));
    }

    #[test]
    fn plex_hosted_pictures() {
        use super::plex_hosted;
        assert!(plex_hosted("https://metadata-static.plex.tv/people/5d7768.jpg"));
        assert!(plex_hosted("https://plex.tv/users/abc/avatar?c=1"));
        assert!(!plex_hosted("https://evilplex.tv/x.jpg"));
        assert!(!plex_hosted("https://plex.tv.example.com/x.jpg"));
        assert!(!plex_hosted("http://192.168.1.10:32400/library/metadata/1/thumb"));
        assert!(!plex_hosted("/library/metadata/1/thumb"));
    }

    #[test]
    fn youtube_ids() {
        use super::youtube_id;
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.youtube.com/watch?feature=x&v=abc_DEF-12&t=3"), Some("abc_DEF-12"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ?t=10"), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.youtube.com/embed/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.dailymotion.com/video/x8abc"), None);
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=<script>"), None);
    }

    use super::{runtime, short_summary};

    #[test]
    fn recent_episodes_collapse_into_shows() {
        use encheo_plex::{Item, ItemKind};
        let ep = |n: u32, show: &str| Item {
            rating_key: format!("{show}-{n}"),
            kind: ItemKind::Episode,
            title: format!("Ep {n}"),
            index: Some(n),
            parent_index: Some(1),
            grandparent_rating_key: Some(show.into()),
            grandparent_title: Some(format!("Show {show}")),
            ..Default::default()
        };
        let movie = Item { rating_key: "m".into(), kind: ItemKind::Movie, title: "Film".into(), ..Default::default() };
        let items = vec![ep(10, "a"), ep(9, "a"), movie, ep(3, "b"), ep(8, "a")];
        let g = super::group_recent(&items);
        let summary: Vec<(String, usize)> = g.iter().map(|(i, n)| (i.title.clone(), *n)).collect();
        assert_eq!(summary, [("Show a".to_string(), 3), ("Film".to_string(), 1), ("Show b".to_string(), 1)]);
        assert_eq!(g[0].0.kind, ItemKind::Show);
        assert_eq!(g[0].0.rating_key, "a");
        assert_eq!(g[2].0.summary.as_deref(), Some("S1 · E3 new"));
    }

    #[test]
    fn speed_verdicts() {
        assert!(super::speed_verdict(150.0).contains("4K remuxes"));
        assert!(super::speed_verdict(30.0).contains("1080p"));
        assert!(super::speed_verdict(3.0).contains("too slow"));
    }

    #[test]
    fn runtimes() {
        assert_eq!(runtime(45 * 60_000), "45 min");
        assert_eq!(runtime(66 * 60_000), "1 hr 6 min");
        assert_eq!(runtime(126 * 60_000), "2 hrs 6 min");
    }

    #[test]
    fn summaries_are_shortened_at_words() {
        assert_eq!(short_summary("Short.", 420), "Short.");
        assert_eq!(short_summary("\nFirst para.\nSecond para.", 420), "First para.");
        assert_eq!(short_summary("one two three four", 10), "one two…");
        assert_eq!(short_summary("one, two three", 9), "one, two…");
        assert_eq!(short_summary("one, two three", 5), "one…");
    }
}
