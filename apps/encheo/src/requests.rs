//! Requests through Seerr (Overseerr, Jellyseerr): a title that isn't in the library gets a
//! Request button on its page instead of Play, and once asked for, its state there: waiting for
//! approval, downloading (how far, how long left), or available.
//!
//! Seerr is added like a source (Settings → Add Source → Seerr). With a Plex account the user
//! signs in with it and nothing else; Jellyfin and Emby users give their server's name and
//! password, which Seerr checks and Encheo doesn't keep.
//!
//! Without a Plex account, search asks Seerr for what isn't in the library (`search`).

use super::*;
use crate::session::SavedSeerr;
use encheo_seerr::{Kind, Progress, SeasonState, Seasons, Seerr, State as Status, Title, User};
use std::cell::RefCell;
use std::sync::Mutex;

/// The connection, kept so its HTTP client is reused, and the signed-in user (for what they may
/// request). Built from the saved session on first use.
static CONN: Mutex<Option<(Seerr, Option<User>)>> = Mutex::new(None);

/// How often a page with something on its way asks again (Seerr itself checks Radarr and
/// Sonarr every minute).
const REFRESH: Duration = Duration::from_secs(20);

/// The title whose page is open, as Seerr knows it.
struct Page {
    rating_key: String,
    kind: Kind,
    tmdb_id: u64,
    title: Option<Title>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
    static TIMER: slint::Timer = slint::Timer::default();
}

/// The Seerr to use, when one is set up. Any thread; filled from the session on the UI thread.
pub(crate) fn connection() -> Option<Seerr> {
    CONN.lock().ok()?.as_ref().map(|(s, _)| s.clone())
}

/// Pick up the saved sign-in (start, or after it changed). UI thread.
pub(crate) fn load() {
    let saved = with_state(|s| s.session.seerr.clone());
    if let Ok(mut c) = CONN.lock() {
        *c = saved.map(|s| (Seerr::resume(&s.address, &s.session), None));
    }
}

fn user() -> Option<User> {
    CONN.lock().ok()?.as_ref().and_then(|(_, u)| u.clone())
}

fn set_user(user: User) {
    if let Ok(mut c) = CONN.lock()
        && let Some((_, u)) = c.as_mut()
    {
        *u = Some(user);
    }
}

/// The Plex token to renew a Plex sign-in with; None when the user signed in another way.
fn plex_renewal() -> Option<String> {
    with_state(|s| s.session.seerr.as_ref().filter(|c| c.login.is_empty()).and(s.session.token.clone()))
}

/// Run `call`; when Seerr's session ran out and the user signed in with Plex, sign in again
/// and run it once more. The new session is saved.
async fn renewing<T, F, Fut>(seerr: &Seerr, plex_token: Option<String>, call: F) -> encheo_seerr::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = encheo_seerr::Result<T>>,
{
    match call().await {
        Err(encheo_seerr::Error::SignedOut) => {
            let Some(token) = plex_token else { return Err(encheo_seerr::Error::SignedOut) };
            let user = seerr.sign_in_plex(&token).await?;
            let session = seerr.session();
            on_ui(move |_| {
                with_state(|s| {
                    if let Some(c) = s.session.seerr.as_mut() {
                        c.session = session;
                        s.session.save();
                    }
                });
            });
            set_user(user);
            call().await
        }
        r => r,
    }
}

// ---------- Settings ----------

/// Sign in from the Add Source form. A blank user name means "with my Plex account".
pub(crate) fn sign_in(location: String, login: String, password: String) {
    let ui = ui();
    let plex_token = with_state(|s| s.session.token.clone());
    let login = login.trim().to_string();
    if login.is_empty() && plex_token.is_none() {
        return ui.set_add_source_status("Enter your user name and password.".into());
    }
    ui.set_add_source_busy(true);
    ui.set_add_source_status("Connecting…".into());
    rt().spawn(async move {
        let result = async {
            let seerr = Seerr::connect(&location).await?;
            let settings = seerr.public_settings().await?;
            let user = if login.is_empty() {
                if !settings.uses_plex() {
                    return Err(Failure::Text("This Seerr uses Jellyfin or Emby: enter your user name and password.".into()));
                }
                seerr.sign_in_plex(plex_token.as_deref().unwrap_or_default()).await?
            } else if settings.uses_jellyfin() && settings.media_server_login {
                match seerr.sign_in_jellyfin(&login, &password).await {
                    // Seerr's own accounts sign in with an email address.
                    Err(encheo_seerr::Error::Refused { .. }) if login.contains('@') && settings.local_login => seerr.sign_in_local(&login, &password).await?,
                    r => r?,
                }
            } else {
                seerr.sign_in_local(&login, &password).await?
            };
            Ok((seerr, user))
        }
        .await;
        on_ui(move |ui| {
            ui.set_add_source_busy(false);
            match result {
                Ok((seerr, user)) => {
                    let saved = SavedSeerr { address: seerr.address().to_string(), session: seerr.session(), user_name: user.display_name.clone(), login };
                    with_state(|s| {
                        s.session.seerr = Some(saved);
                        s.session.save();
                        s.editing = None;
                    });
                    if let Ok(mut c) = CONN.lock() {
                        *c = Some((seerr, Some(user.clone())));
                    }
                    ui.set_add_source_status(SharedString::new());
                    ui.set_add_source_open(false);
                    shares::fill_sources();
                    toast(ui, format!("Signed in to Seerr as {}", user.display_name));
                    refresh_page();
                }
                Err(e) => ui.set_add_source_status(e.text().into()),
            }
        });
    });
}

enum Failure {
    Seerr(encheo_seerr::Error),
    Text(String),
}

impl From<encheo_seerr::Error> for Failure {
    fn from(e: encheo_seerr::Error) -> Self {
        Failure::Seerr(e)
    }
}

impl Failure {
    fn text(&self) -> String {
        use encheo_seerr::Error;
        match self {
            Failure::Text(t) => t.clone(),
            Failure::Seerr(Error::Refused { message, .. }) if message.contains("INVALID_CREDENTIALS") || message == "Access denied." => {
                "Seerr didn't let you in: check the user name and password, and that your account may use Seerr.".into()
            }
            Failure::Seerr(Error::Refused { message, .. }) => format!("Seerr said: {message}"),
            Failure::Seerr(e) => format!("Couldn't connect: {e}"),
        }
    }
}

/// The Settings row, when Seerr is set up.
pub(crate) fn source_row() -> Option<SourceRow> {
    with_state(|s| {
        s.session.seerr.as_ref().map(|c| SourceRow {
            kind: "seerr".into(),
            index: 0,
            title: "Seerr".into(),
            label: "Requests".into(),
            detail: format!("{} · {}", c.user_name, c.address).into(),
            signed_in: true,
        })
    })
}

/// What the edit form starts with: the address and what was typed to sign in.
pub(crate) fn form() -> Option<(String, String)> {
    with_state(|s| s.session.seerr.as_ref().map(|c| (c.address.clone(), c.login.clone())))
}

/// Remove Seerr (the user confirmed): sign out there too.
pub(crate) fn remove() {
    if let Some(seerr) = connection() {
        rt().spawn(async move {
            let _ = seerr.sign_out().await;
        });
    }
    with_state(|s| {
        s.session.seerr = None;
        s.session.save();
    });
    load();
    refresh_page();
}

// ---------- A title's page ----------

/// A title's page opened: forget the last one's state.
pub(crate) fn page_opened() {
    PAGE.with(|p| *p.borrow_mut() = None);
    TIMER.with(|t| t.stop());
    let ui = ui();
    ui.set_detail_play_icon("play".into());
    ui.set_detail_play_enabled(true);
    ui.set_detail_play_progress(-1.0);
    ui.set_detail_play_note(SharedString::new());
}

/// A catalogue title's page has its details: ask Seerr about it, if Seerr is set up.
pub(crate) fn catalogue_page(item: &Item) {
    let kind = match item.kind {
        ItemKind::Movie => Kind::Movie,
        ItemKind::Show => Kind::Tv,
        _ => return,
    };
    let Some(tmdb_id) = item.external_id("tmdb").and_then(|t| t.parse().ok()) else { return };
    if connection().is_none() {
        return;
    }
    PAGE.with(|p| *p.borrow_mut() = Some(Page { rating_key: item.rating_key.clone(), kind, tmdb_id, title: None }));
    refresh_page();
}

/// A title found through Seerr's search: its page is all Seerr's.
pub(crate) fn seerr_page(ui: &AppWindow, item: &Item) {
    let Some((kind, tmdb_id)) = parse_id(&item.rating_key) else { return };
    ui.set_play_label(SharedString::new());
    seerr_buttons(ui);
    PAGE.with(|p| *p.borrow_mut() = Some(Page { rating_key: item.rating_key.clone(), kind, tmdb_id, title: None }));
    refresh_page();
}

/// `seerr:movie:438631` → (film, 438631).
fn parse_id(rating_key: &str) -> Option<(Kind, u64)> {
    let rest = rating_key.strip_prefix("seerr:")?;
    let (kind, id) = rest.split_once(':')?;
    let kind = match kind {
        "movie" => Kind::Movie,
        "tv" => Kind::Tv,
        _ => return None,
    };
    Some((kind, id.parse().ok()?))
}

/// Whether the open page's main button is a request (Play then asks Seerr instead).
pub(crate) fn is_request_page() -> bool {
    let key = with_state(|s| s.detail.as_ref().map(|d| d.rating_key.clone()));
    PAGE.with(|p| p.borrow().as_ref().is_some_and(|p| Some(&p.rating_key) == key.as_ref()))
}

/// Ask Seerr about the open page's title again and show what it says.
fn refresh_page() {
    let Some((key, kind, tmdb_id)) = PAGE.with(|p| p.borrow().as_ref().map(|p| (p.rating_key.clone(), p.kind, p.tmdb_id))) else { return };
    let Some(seerr) = connection() else {
        // Seerr was removed: the page goes back to having no main button.
        PAGE.with(|p| *p.borrow_mut() = None);
        let ui = ui();
        ui.set_play_label(SharedString::new());
        ui.set_detail_play_note(SharedString::new());
        ui.set_detail_play_progress(-1.0);
        return;
    };
    let renewal = plex_renewal();
    let known = user();
    rt().spawn(async move {
        let title = renewing(&seerr, renewal.clone(), || seerr.title(kind, tmdb_id)).await;
        let user = match known {
            Some(u) => Some(u),
            None => renewing(&seerr, renewal, || seerr.me()).await.ok(),
        };
        if let Some(u) = &user {
            set_user(u.clone());
        }
        on_ui(move |ui| {
            let still = with_state(|s| s.detail.as_ref().is_some_and(|d| d.rating_key == key));
            if !still || ui.get_screen() != "detail" {
                return;
            }
            match title {
                Ok(title) => {
                    let seerr_only = key.starts_with("seerr:");
                    if seerr_only {
                        fill_from_seerr(ui, &title);
                    }
                    show_state(ui, &title, user.as_ref());
                    PAGE.with(|p| {
                        if let Some(p) = p.borrow_mut().as_mut() {
                            p.title = Some(title);
                        }
                    });
                }
                Err(encheo_seerr::Error::SignedOut) => {
                    ui.set_play_label("Request".into());
                    ui.set_detail_play_icon("plus".into());
                    ui.set_detail_play_enabled(false);
                    ui.set_detail_play_note("Signed out of Seerr: sign in again under Settings.".into());
                }
                // Seerr can't be reached: the page simply has no request button.
                Err(_) => {}
            }
        });
    });
}

/// Nothing to play, mark or keep: it isn't yours yet. A badge says so.
fn seerr_buttons(ui: &AppWindow) {
    ui.set_detail_can_restart(false);
    ui.set_detail_can_mark_watched(false);
    ui.set_detail_can_shuffle(false);
    ui.set_detail_can_favourite(false);
    ui.set_detail_badges(ModelRc::new(VecModel::from(vec![SharedString::from("Not in your library")])));
}

/// A Seerr-only page: the details come from Seerr (TMDB).
fn fill_from_seerr(ui: &AppWindow, title: &Title) {
    let mut item = to_item(title);
    item.summary = Some(title.overview.clone()).filter(|s| !s.is_empty());
    let backdrop = title.backdrop_path.as_deref().map(|p| encheo_seerr::image_url(p, 1280)).unwrap_or_default();
    with_state(|s| {
        s.detail = Some(item.clone());
        s.detail_backdrop_url = backdrop.clone();
    });
    ui.set_detail(poster(&item, false));
    ui.set_detail_backdrop(want_image(&backdrop));
    fill_detail_text(ui, &item);
    seerr_buttons(ui);
}

/// The button, its note and progress bar for the title's state.
fn show_state(ui: &AppWindow, title: &Title, user: Option<&User>) {
    let can = user.is_none_or(|u| u.can_request(title.kind, false));
    let state = title.state(false);
    let missing = title.kind == Kind::Tv && !title.missing_seasons(false).is_empty();
    // Seasons asked for and not there yet (a show partly on the server).
    let asked = title.seasons.iter().any(|s| title.season_state(s.number, false) == SeasonState::Requested);
    let (label, icon, enabled, progress, note): (&str, &str, bool, Option<&Progress>, String) = match &state {
        Status::NotRequested => ("Request", "plus", can, None, if can { String::new() } else { "Your Seerr account can't request this.".into() }),
        Status::Requested => ("Requested", "check", false, None, "Waiting for approval".into()),
        Status::Processing => ("Requested", "check", false, None, "Approved · waiting for a download to start".into()),
        Status::Downloading(p) => ("Downloading", "check", false, Some(p), progress_text(p)),
        Status::PartlyAvailable(p) if missing && can => ("Request More", "plus", true, p.as_ref(), partly_text(p.as_ref(), asked)),
        Status::PartlyAvailable(p) => ("Partly Available", "check", false, p.as_ref(), partly_text(p.as_ref(), asked)),
        Status::Available => ("Available", "check", false, None, "On your server: it shows up in your library after the next scan.".into()),
        Status::Blocked => ("Can't Be Requested", "close", false, None, String::new()),
    };
    ui.set_play_label(label.into());
    ui.set_detail_play_icon(icon.into());
    ui.set_detail_play_enabled(enabled);
    ui.set_detail_play_progress(progress.and_then(Progress::fraction).unwrap_or(-1.0));
    ui.set_detail_play_note(note.into());
    detail_rows(ui);
    // Something on its way: keep the page up to date while it's open.
    let moving = matches!(state, Status::Requested | Status::Processing | Status::Downloading(_) | Status::PartlyAvailable(Some(_))) || asked;
    TIMER.with(|t| {
        if moving {
            if !t.running() {
                t.start(slint::TimerMode::Repeated, REFRESH, refresh_page);
            }
        } else {
            t.stop();
        }
    });
}

fn partly_text(p: Option<&Progress>, asked: bool) -> String {
    match p {
        Some(p) => format!("Some of it is on your server · downloading more: {}", progress_text(p).to_lowercase()),
        None if asked => "Some of it is on your server · more is requested".into(),
        None => "Some of it is on your server".into(),
    }
}

/// "42% · about 12 min left", "3 files · 42% · about 2 h 5 min left", "Paused or queued".
fn progress_text(p: &Progress) -> String {
    let mut parts = Vec::new();
    if p.files > 1 {
        parts.push(format!("{} files", p.files));
    }
    if p.stalled {
        parts.push("paused or queued".into());
    } else {
        match p.fraction() {
            Some(f) => parts.push(format!("{}%", (f * 100.0).floor() as u32)),
            None => parts.push("starting".into()),
        }
        if let Some(t) = p.time_left.filter(|t| t.as_secs() > 0) {
            parts.push(format!("about {} left", duration_text(t)));
        }
    }
    let mut text = parts.join(" · ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

fn duration_text(t: Duration) -> String {
    let mins = t.as_secs().div_ceil(60);
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The page's main button: a film is requested straight away; a show offers its seasons.
pub(crate) fn request() {
    let Some((title, user)) = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.title.clone())).map(|t| (t, user())) else { return };
    if !ui().get_detail_play_enabled() {
        return;
    }
    if title.kind == Kind::Movie {
        return send(title, Seasons::All, user);
    }
    let missing = title.missing_seasons(false);
    if missing.len() <= 1 {
        return send(title, Seasons::Only(missing), user);
    }
    // The choice panel the library filters use, listing the seasons.
    let mut options = vec![format!("All {} missing seasons", missing.len())];
    for s in &title.seasons {
        let state = match title.season_state(s.number, false) {
            SeasonState::Missing => "",
            SeasonState::Requested => " · requested",
            SeasonState::Partial => " · partly there",
            SeasonState::Available => " · on your server",
        };
        let name = if s.name.is_empty() { format!("Season {}", s.number) } else { s.name.clone() };
        options.push(format!("{name}{state}"));
    }
    filters::open_choices(&format!("Request {}", title.title), options, filters::SEASONS);
}

/// A choice in the seasons panel: 0 is every missing season, then the seasons in order.
pub(crate) fn pick_season(index: usize) {
    let Some(title) = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.title.clone())) else { return };
    let seasons = if index == 0 {
        title.missing_seasons(false)
    } else {
        let Some(s) = title.seasons.get(index - 1) else { return };
        if title.season_state(s.number, false) != SeasonState::Missing {
            return toast(&ui(), format!("{} is already on its way or on your server", s.name));
        }
        vec![s.number]
    };
    send(title, Seasons::Only(seasons), user());
}

fn send(title: Title, seasons: Seasons, user: Option<User>) {
    let Some(seerr) = connection() else { return };
    let renewal = plex_renewal();
    let ui = ui();
    ui.set_detail_play_enabled(false);
    ui.set_detail_play_note("Asking Seerr…".into());
    rt().spawn(async move {
        let result = renewing(&seerr, renewal, || seerr.request(title.kind, title.tmdb_id, seasons.clone(), false)).await;
        on_ui(move |ui| {
            match result {
                Ok(r) => {
                    let waits = r.status == encheo_seerr::RequestStatus::Pending && !user.is_some_and(|u| u.auto_approved(title.kind));
                    toast(ui, if waits { format!("Requested “{}”: an admin will approve it", title.title) } else { format!("Requested “{}”", title.title) });
                }
                Err(encheo_seerr::Error::Refused { message, .. }) => toast(ui, format!("Seerr didn't take the request: {message}")),
                Err(encheo_seerr::Error::SignedOut) => toast(ui, "Signed out of Seerr: sign in again under Settings"),
                Err(e) => toast(ui, format!("Couldn't reach Seerr: {e}")),
            }
            refresh_page();
        });
    });
}

// ---------- Search ----------

/// Films and shows matching `query` on Seerr, as items whose page is Seerr's (`seerr:` ids).
pub(crate) async fn search(seerr: &Seerr, query: &str) -> Vec<Item> {
    seerr.search(query, 1).await.map(|list| list.iter().take(12).map(to_item).collect()).unwrap_or_default()
}

fn to_item(t: &Title) -> Item {
    Item {
        rating_key: format!("seerr:{}:{}", t.kind.as_str(), t.tmdb_id),
        kind: if t.kind == Kind::Movie { ItemKind::Movie } else { ItemKind::Show },
        title: t.title.clone(),
        year: t.year,
        thumb: t.poster_path.as_deref().map(|p| encheo_seerr::image_url(p, 342)),
        art: t.backdrop_path.as_deref().map(|p| encheo_seerr::image_url(p, 780)),
        guids: vec![encheo_plex::ExternalId { id: format!("tmdb://{}", t.tmdb_id) }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reads_well() {
        let p = |size, left, secs: Option<u64>, files, stalled| Progress {
            size,
            size_left: left,
            files,
            time_left: secs.map(Duration::from_secs),
            estimated_completion: None,
            stalled,
        };
        assert_eq!(progress_text(&p(100, 58, Some(700), 1, false)), "42% · about 12 min left");
        assert_eq!(progress_text(&p(100, 0, Some(0), 1, false)), "100%");
        assert_eq!(progress_text(&p(0, 0, None, 1, false)), "Starting");
        assert_eq!(progress_text(&p(400, 200, Some(7500), 3, false)), "3 files · 50% · about 2 h 5 min left");
        assert_eq!(progress_text(&p(100, 100, None, 2, true)), "2 files · paused or queued");
        assert_eq!(progress_text(&p(100, 100, None, 1, true)), "Paused or queued");
        assert_eq!(duration_text(Duration::from_secs(3600)), "1 h");
    }

    #[test]
    fn seerr_ids() {
        assert_eq!(parse_id("seerr:movie:438631"), Some((Kind::Movie, 438631)));
        assert_eq!(parse_id("seerr:tv:1399"), Some((Kind::Tv, 1399)));
        assert_eq!(parse_id("seerr:person:1"), None);
        assert_eq!(parse_id("cat:5eb8"), None);
    }
}
