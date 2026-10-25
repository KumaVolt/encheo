//! Encheo desktop app: Plex sign-in, browsing, details and playback.
//!
//! Threads: the UI thread owns all Slint objects and the app state (thread-locals below). Network
//! and decoding run on a tokio runtime; results come back through `on_ui`.

mod backend;
mod images;
mod langs;
mod progress;
mod shares;
mod player;
mod session;

use encheo_plex::{ClientInfo, Item, ItemKind, MarkerKind, PlayState, Plex, Server, Stream, StreamKind};
use player::{Player, clock};
use backend::{Backend, all_backends, backend_for};
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
    grid: Rc<VecModel<Poster>>,
    albums: Rc<VecModel<Poster>>,
    tracks: Rc<VecModel<Track>>,
    people: Rc<VecModel<Person>>,
    related: Rc<VecModel<Poster>>,
    seasons: Rc<VecModel<SharedString>>,
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
    /// Everything we've seen, by rating key, so clicks can find the full item.
    items: HashMap<String, Item>,
    images: HashMap<String, Image>,
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
    season_items: Vec<Item>,
    next_up: Option<Item>,
    now_playing: Option<NowPlaying>,
    /// Tracks of the album on the detail screen.
    album_tracks: Vec<Item>,
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
}

struct NowPlaying {
    item: Item,
    /// The show's episodes, in order, when playing an episode.
    episodes: Vec<Item>,
    /// Whether the user's Plex track choices have been applied to this file yet.
    streams_applied: bool,
    /// Languages detected from the text of untagged subtitle tracks, by stream index.
    detected: HashMap<i64, langs::Detected>,
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
    Plex::new(ClientInfo::new(session.client_id.clone(), format!("Encheo ({})", std::env::consts::OS)))
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
    ui.set_grid_can_remove(false);
    let backends = all_backends();
    ui.set_server_name(backends.iter().map(Backend::label).collect::<Vec<_>>().join(", ").into());
    ui.set_server_kind(match backends.as_slice() {
        [] => SharedString::new(),
        [b] => b.kind_label().into(),
        _ => "Media servers".into(),
    });
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
        let mut on_deck = Vec::new();
        let mut errors = Vec::new();
        for b in &backends {
            match b.libraries().await {
                Ok(list) => {
                    for l in list {
                        let recent = b.recently_added(&l.key, 24).await.unwrap_or_default();
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
            let mut rows = Vec::new();
            let mut row_models = Vec::new();
            for (index, (group, l, items)) in libs.iter().enumerate() {
                if items.is_empty() {
                    continue;
                }
                let model = Rc::new(VecModel::from(items.iter().map(|i| poster(i, false)).collect::<Vec<_>>()));
                let title = if several { format!("Recently Added in {} · {group}", l.title) } else { format!("Recently Added in {}", l.title) };
                rows.push(HomeRow {
                    title: title.into(),
                    library: index as i32,
                    landscape: false,
                    square: l.kind == "artist",
                    items: ModelRc::from(model.clone()),
                });
                row_models.push(model);
            }
            remember(on_deck.into_iter().chain(libs.into_iter().flat_map(|(_, _, items)| items)));
            with_models(|m| {
                m.continue_watching.set_vec(cw);
                *m.row_models.borrow_mut() = row_models;
                m.home_rows.set_vec(rows);
            });
        });
    });
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
    // Jellyfin/Emby servers can be removed from any of their libraries.
    ui.set_grid_can_remove(matches!(server, Backend::Jf(_)));
    let kind_str = kind.clone().unwrap_or_default();
    ui.set_grid_square(kind.as_deref() == Some("artist"));
    let title = with_models(|m| m.libraries.row_data(index as usize).map(|l| l.title)).unwrap_or_default();
    ui.set_grid_title(title);
    ui.set_loading(true);
    with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    rt().spawn(async move {
        let mut start = 0;
        loop {
            let page = server.library_items(&key, &kind_str, start, PAGE).await;
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
    let square = matches!(item.kind, ItemKind::Artist | ItemKind::Album | ItemKind::Track);
    let subtitle = if is_ep {
        format!("{} · {}", item.episode_label().unwrap_or_default(), item.title)
    } else if item.kind == ItemKind::Album {
        item.parent_title.clone().unwrap_or_default()
    } else if item.kind == ItemKind::Artist {
        String::new()
    } else {
        match (item.kind.clone(), item.year, item.leaf_count) {
            (ItemKind::Show, _, Some(n)) => format!("{n} episodes"),
            (_, Some(y), _) => y.to_string(),
            _ => String::new(),
        }
    };
    let path = if landscape {
        item.art.clone().or_else(|| item.grandparent_art.clone()).or_else(|| item.thumb.clone())
    } else if is_ep {
        item.grandparent_thumb.clone().or_else(|| item.thumb.clone())
    } else {
        item.thumb.clone()
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
        image: want_image(&url),
        art_url: url.into(),
        progress: item.progress().unwrap_or(0.0),
        watched: item.is_watched() && !square,
        square,
    }
}

fn art_url(path: Option<&str>, w: u32, h: u32) -> String {
    match path {
        // Jellyfin/Emby give full image URLs; ask for the size we need.
        Some(p) if p.starts_with("http") => format!("{p}&maxWidth={w}&maxHeight={h}"),
        Some(p) if !p.is_empty() => server().map(|s| s.image_url(p, w, h)).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The image if cached; otherwise start loading it and return an empty image for now.
fn want_image(url: &str) -> Image {
    if url.is_empty() {
        return Image::default();
    }
    let start = with_state(|s| {
        if let Some(i) = s.images.get(url) {
            return Err(i.clone());
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
                });
            });
            Image::default()
        }
    }
}

fn apply_image(ui: &AppWindow, url: &str, img: &Image) {
    fn update_posters(model: &VecModel<Poster>, url: &str, img: &Image) {
        for i in 0..model.row_count() {
            if let Some(mut p) = model.row_data(i)
                && p.art_url == url
            {
                p.image = img.clone();
                model.set_row_data(i, p);
            }
        }
    }
    with_models(|m| {
        update_posters(&m.continue_watching, url, img);
        update_posters(&m.grid, url, img);
        for row in m.row_models.borrow().iter() {
            update_posters(row, url, img);
        }
        update_posters(&m.albums, url, img);
        update_posters(&m.related, url, img);
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
    if with_state(|s| s.detail_backdrop_url == url) {
        ui.set_detail_backdrop(img.clone());
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
    let Some(item) = with_state(|s| s.items.get(&id).cloned()) else { return };
    // Episodes and part-watched movies from "Continue Watching" go straight to the player.
    if item.kind == ItemKind::Episode {
        return play(item);
    }
    if item.kind == ItemKind::Track {
        return play_music(vec![item], 0);
    }
    show_detail(item);
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
            _ => "movie",
        }
        .into(),
    );
    ui.set_play_label(match item.kind {
        ItemKind::Show | ItemKind::Artist => SharedString::new(),
        ItemKind::Album => "Play album".into(),
        _ => play_label(&item).into(),
    });
    with_models(|m| {
        m.seasons.set_vec(Vec::<SharedString>::new());
        m.episodes.set_vec(Vec::<Episode>::new());
        m.albums.set_vec(Vec::<Poster>::new());
        m.tracks.set_vec(Vec::<Track>::new());
        m.people.set_vec(Vec::<Person>::new());
        m.related.set_vec(Vec::<Poster>::new());
    });
    with_state(|s| s.album_tracks.clear());
    ui.set_screen("detail".into());

    let Some(server) = backend_for(&item.rating_key) else { return };
    let generation = with_state(|s| s.generation);
    let id = item.rating_key.clone();
    let is_show = item.kind == ItemKind::Show;
    let kind = item.kind.clone();
    rt().spawn(async move {
        let full = server.metadata(&id).await.ok().flatten();
        let children = if matches!(kind, ItemKind::Artist | ItemKind::Album) {
            server.children(&id).await.unwrap_or_default()
        } else {
            vec![]
        };
        let related = server.related(&id).await.unwrap_or_default();
        let seasons = if is_show { server.children(&id).await.unwrap_or_default() } else { vec![] };
        let all_eps = if is_show { server.all_episodes(&id).await.unwrap_or_default() } else { vec![] };
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            match kind {
                ItemKind::Artist => {
                    let albums: Vec<Poster> = children.iter().map(|a| poster(a, false)).collect();
                    remember(children);
                    with_models(|m| m.albums.set_vec(albums));
                }
                ItemKind::Album => {
                    with_state(|s| s.album_tracks = children.clone());
                    refresh_tracks();
                    remember(children);
                }
                _ => {}
            }
            let related_posters: Vec<Poster> = related.iter().map(|i| poster(i, false)).collect();
            remember(related);
            with_models(|m| m.related.set_vec(related_posters));
            if let Some(full) = full {
                fill_detail_text(ui, &full);
                let cast = people(&full);
                with_models(|m| m.people.set_vec(cast));
                if !is_show && !matches!(full.kind, ItemKind::Artist | ItemKind::Album) {
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
                with_models(|m| m.seasons.set_vec(seasons.iter().map(|s| SharedString::from(s.title.as_str())).collect::<Vec<_>>()));
                select_season(selected as i32);
            }
        });
    });
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
    let album = with_state(|s| {
        (s.detail.as_ref().map(|d| d.kind.clone()) == Some(ItemKind::Album)).then(|| s.album_tracks.clone())
    });
    if let Some(tracks) = album {
        return play_music(tracks, 0);
    }
    let item = with_state(|s| s.next_up.clone().or_else(|| s.detail.clone()));
    if let Some(item) = item {
        play(item);
    }
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
    report(PlayState::Stopped, status.position);
    p.stop();
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
                if sel == -2 { refresh() } else { select_library(sel) }
            }
        }
    });
}

/// Runs four times a second: mirrors mpv's state into the UI, reports progress, auto-hides
/// controls, offers to skip intros and credits, and moves on to the next episode at the end.
fn tick() {
    let ui = ui();
    fit_tv_scale(&ui);
    if ui.get_screen() != "player" {
        return music_tick(&ui);
    }
    let Some(p) = player() else { return };
    let ps = ps(&ui);
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
    if !is_episode() {
        update_neighbours(&ui); // chapters appear once the file is open
    }
    apply_plex_streams(&p);

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

fn pick(id: String) {
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

// ---------- Search ----------

/// Called on every keystroke; waits for a short pause in typing before asking the server.
fn search(query: String) {
    let generation = with_state(|s| {
        s.search_generation += 1;
        s.search_generation
    });
    let query = query.trim().to_string();
    if query.is_empty() {
        if ui().get_selected_library() == -2 {
            enter_home();
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
            let results: Result<Vec<Item>, String> =
                if found.is_empty() && !errors.is_empty() { Err(errors.join("; ")) } else { Ok(found) };
            on_ui(move |ui| {
                if with_state(|s| s.search_generation) != generation {
                    return;
                }
                ui.set_loading(false);
                match results {
                    Ok(items) => {
                        let posters: Vec<Poster> = items.iter().map(|i| poster(i, false)).collect();
                        remember(items);
                        with_models(|m| m.grid.set_vec(posters));
                    }
                    Err(e) => toast(ui, format!("Search failed: {e}")),
                }
            });
        });
    });
}

fn refresh() {
    if shares::reload_current() {
        return;
    }
    let ui = ui();
    match ui.get_selected_library() {
        -2 => search(ui.get_search_text().to_string()),
        i => select_library(i),
    }
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
    with_state(|s| s.music = Some(MusicQueue { items, index: usize::MAX, last_report: Instant::now(), art_url: String::new() }));
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
        refresh_tracks();
        report_item(track.clone(), PlayState::Playing, 0.0);
    }
    ui.set_music_paused(st.paused);
    ui.set_music_position(st.position as f32);
    ui.set_music_duration(if st.duration > 0.0 { st.duration as f32 } else { track.duration.unwrap_or(0) as f32 / 1000.0 });
    ui.set_music_position_text(clock(st.position).into());
    ui.set_music_duration_text(clock(ui.get_music_duration() as f64).into());
    if report_now {
        report_item(track, if st.paused { PlayState::Paused } else { PlayState::Playing }, st.position);
    }
}

/// Rebuild the album's track list, marking the one that's playing.
fn refresh_tracks() {
    let (tracks, playing) = with_state(|s| {
        (s.album_tracks.clone(), s.music.as_ref().and_then(|q| q.items.get(q.index)).map(|i| i.rating_key.clone()))
    });
    let album_artist = with_state(|s| s.detail.as_ref().and_then(|d| d.parent_title.clone()));
    let rows: Vec<Track> = tracks
        .iter()
        .map(|t| Track {
            id: t.rating_key.clone().into(),
            number: t.index.map(|n| n.to_string()).unwrap_or_default().into(),
            title: t.title.clone().into(),
            // Only show the artist when it differs from the album's (features, compilations).
            artist: t.original_title.clone().filter(|a| Some(a) != album_artist.as_ref()).unwrap_or_default().into(),
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
        + usize::from(ui.get_detail_can_restart())
        + usize::from(ui.get_detail_can_mark_watched());
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
    if kind == "album" {
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
    if !arrow && !enter {
        return false;
    }
    let ui = ui();
    let nav = ui.global::<Nav>();
    if !nav.get_active() {
        // First key press only reveals the focus, on the content (never straight into search).
        nav.set_active(true);
        if ui.get_screen() == "home" && nav.get_zone() != "content" {
            set_focus(&ui, "content", 0, 0);
        }
        return true;
    }
    let (zone, row, col) = (nav.get_zone(), nav.get_row(), nav.get_col());
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
        let last = libraries + 2; // 0 search, 1 Home, 2.. libraries, then Add Source
        if enter {
            match row {
                0 => ui.global::<Nav>().set_search_request(ui.global::<Nav>().get_search_request() + 1),
                1 => select_library(-1),
                r if r == last => {
                    ui.set_add_source_status(SharedString::new());
                    ui.set_add_source_open(true);
                }
                r => select_library(r - 2),
            }
            if row > 0 {
                set_focus(ui, "side", row, 0);
            }
        } else if is(Key::UpArrow) {
            set_focus(ui, "side", (row - 1).max(0), 0);
        } else if is(Key::DownArrow) {
            set_focus(ui, "side", (row + 1).min(last), 0);
        } else if is(Key::RightArrow) {
            set_focus(ui, "content", 0, 0);
        }
        return true;
    }

    let to_side = |ui: &AppWindow| {
        if !compact {
            set_focus(ui, "side", if selected >= 0 { selected + 2 } else { 1 }, 0);
        }
    };

    if selected == -1 {
        // Shelves: continue watching (if any), then one row per library.
        let lens: Vec<(i32, Vec<SharedString>)> = with_models(|m| {
            let mut v = Vec::new();
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
        if enter {
            if let Some(id) = lens[row as usize].1.get(col as usize) {
                open_item(id.to_string());
            }
        } else if is(Key::UpArrow) {
            let r = (row - 1).max(0);
            set_focus(ui, "content", r, col.min(lens[r as usize].0 - 1));
        } else if is(Key::DownArrow) {
            let r = (row + 1).min(lens.len() as i32 - 1);
            set_focus(ui, "content", r, col.min(lens[r as usize].0 - 1));
        } else if is(Key::LeftArrow) {
            if col == 0 { to_side(ui) } else { set_focus(ui, "content", row, col - 1) }
        } else if is(Key::RightArrow) {
            set_focus(ui, "content", row, (col + 1).min(lens[row as usize].0 - 1));
        }
        return true;
    }

    // A library grid or search results.
    let ids: Vec<SharedString> = with_models(|m| m.grid.iter().map(|p| p.id).collect());
    if ids.is_empty() {
        if is(Key::LeftArrow) {
            to_side(ui);
        }
        return true;
    }
    let cols = ui.global::<Nav>().get_grid_cols().max(1);
    let last = ids.len() as i32 - 1;
    let idx = (row * cols + col).clamp(0, last);
    let go = |ui: &AppWindow, i: i32| set_focus(ui, "content", i / cols, i % cols);
    if enter {
        open_item(ids[idx as usize].to_string());
    } else if is(Key::UpArrow) {
        go(ui, if idx >= cols { idx - cols } else { idx });
    } else if is(Key::DownArrow) {
        go(ui, (idx + cols).min(last));
    } else if is(Key::LeftArrow) {
        if idx % cols == 0 { to_side(ui) } else { go(ui, idx - 1) }
    } else if is(Key::RightArrow) {
        go(ui, (idx + 1).min(last));
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
    if enter {
        detail_activate(ui, rows[row as usize].0, row, col, &rows);
    } else if is(Key::UpArrow) || is(Key::DownArrow) {
        let step = if is(Key::UpArrow) { -1 } else { 1 };
        let mut r = row + step;
        while r >= 0 && (r as usize) < rows.len() && !usable(r) {
            r += step;
        }
        if r >= 0 && (r as usize) < rows.len() {
            set_focus(ui, "detail", r, col.min(rows[r as usize].1 as i32 - 1));
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
            if ui.get_detail_can_restart() {
                actions.push(play_from_start);
            }
            if ui.get_detail_can_mark_watched() {
                actions.push(toggle_watched);
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

#[cfg(not(target_os = "android"))]
/// Escape backs out one step: panel → fullscreen → player → details → search.
fn escape() {
    let ui = ui();
    let ps = ps(&ui);
    match ui.get_screen().as_str() {
        "player" => {
            if !ps.get_panel().is_empty() {
                ps.set_panel(SharedString::new());
            } else if ps.get_fullscreen() {
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
/// UI from across the room is huge. TVs get a scale that makes the screen 1440 points wide,
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
        let want = (width / 1440.0).max(1.0);
        if (ui.window().scale_factor() - want).abs() > 0.01 {
            ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: want });
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = ui;
}

/// Start the app and run until its window closes.
pub fn run() -> Result<(), slint::PlatformError> {
    // The video is drawn with OpenGL under the UI, so the UI must render with OpenGL too.
    #[cfg(target_os = "android")]
    slint::BackendSelector::new().require_opengl_es().select()?;
    #[cfg(not(target_os = "android"))]
    slint::BackendSelector::new()
        .require_opengl()
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
    let ui = AppWindow::new()?;
    ui.global::<Theme>().set_buttons_in_content(cfg!(target_os = "macos"));
    ui.global::<PlayerState>().set_window_modes(cfg!(not(target_os = "android")));
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
    with_state(|s| s.session = Session::load());
    shares::sync_configs();
    backend::load_jf_servers();

    let models = Models {
        libraries: Rc::new(VecModel::default()),
        continue_watching: Rc::new(VecModel::default()),
        home_rows: Rc::new(VecModel::default()),
        row_models: RefCell::new(Vec::new()),
        grid: Rc::new(VecModel::default()),
        albums: Rc::new(VecModel::default()),
        tracks: Rc::new(VecModel::default()),
        people: Rc::new(VecModel::default()),
        related: Rc::new(VecModel::default()),
        seasons: Rc::new(VecModel::default()),
        episodes: Rc::new(VecModel::default()),
    };
    ui.set_libraries(ModelRc::from(models.libraries.clone()));
    ui.set_continue_watching(ModelRc::from(models.continue_watching.clone()));
    ui.set_home_rows(ModelRc::from(models.home_rows.clone()));
    ui.set_grid(ModelRc::from(models.grid.clone()));
    ui.set_albums(ModelRc::from(models.albums.clone()));
    ui.set_tracks(ModelRc::from(models.tracks.clone()));
    ui.set_people(ModelRc::from(models.people.clone()));
    ui.set_related(ModelRc::from(models.related.clone()));
    ui.set_seasons(ModelRc::from(models.seasons.clone()));
    ui.set_episodes(ModelRc::from(models.episodes.clone()));
    MODELS.with(|m| *m.borrow_mut() = Some(models));

    match Player::new() {
        Ok(p) => PLAYER.with(|slot| *slot.borrow_mut() = Some(Rc::new(p))),
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

    ui.on_open_link(|| {
        #[cfg(not(target_os = "android"))]
        let _ = open::that("https://plex.tv/link");
    });
    ui.on_new_code(begin_signin);
    ui.on_select_library(select_library);
    ui.on_open_item(|id| open_item(id.to_string()));
    ui.on_sign_out(|| {
        with_state(|s| {
            s.session.sign_out();
            s.server = None;
            s.items.clear();
        });
        begin_signin();
    });
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
    ui.on_music_seek_to(|t| {
        if let Some(p) = player() {
            p.seek_to(t as f64);
        }
    });
    ui.on_search(|q| search(q.to_string()));
    ui.on_add_source(|kind, name, location, user, password| {
        shares::add_source(kind, name.to_string(), location.to_string(), user.to_string(), password.to_string())
    });
    ui.on_remove_source(shares::remove_current);
    ui.global::<Nav>().on_key(|k| nav_key(k.as_str()));
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

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), tick);

    resume_session();
    // Android TV pops the keyboard up as the window gets focus; close it once that has happened.
    for ms in [400, 1500] {
        slint::Timer::single_shot(Duration::from_millis(ms), hide_keyboard);
    }
    ui.run()
}

#[cfg(test)]
mod tests {
    use super::{runtime, short_summary};

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
