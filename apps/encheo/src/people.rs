//! A person's page: tap someone under Cast and Crew to see who they are and what else they've
//! been in or made.
//!
//! - Plex: their biography and full filmography from Plex's catalogue (needs the Plex account),
//!   one row per kind of credit (Actor, Director, …). Titles you have open your copy, the rest
//!   their catalogue page; "In your library" gathers yours first.
//! - Jellyfin/Emby: their biography and the titles of theirs on that server.
//!
//! The page is a library-style page (selected library [`PERSON_PAGE`]). Back returns to the
//! title you came from, and from there to the page before it, so a chain of titles and people
//! unwinds the way it was walked.

use super::*;
use std::cell::RefCell;

/// The selected library while a person's page shows.
pub(crate) const PERSON_PAGE: i32 = -4;

/// How many titles a row of credits shows ("Appearances" can run to hundreds of talk shows).
const ROW_MAX: usize = 60;

/// The page that was showing before a person's page opened, to put back on Back.
struct Frame {
    selected: i32,
    grid: Vec<Poster>,
    grid_title: SharedString,
    grid_square: bool,
    rows: ModelRc<HomeRow>,
    person: PersonHeader,
    person_id: Option<String>,
    /// The title page the person was opened from.
    detail: Option<Item>,
}

thread_local! {
    static STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// The person on the page now (for Refresh).
    static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Their page, from a cast or crew card (`id` from `Role::person_id`).
pub(crate) fn open(id: String) {
    if id.is_empty() {
        return;
    }
    let ui = ui();
    let detail = if ui.get_screen() == "detail" { with_state(|s| s.detail.clone()) } else { None };
    let frame = Frame {
        selected: ui.get_selected_library(),
        grid: with_models(|m| m.grid.iter().collect()),
        grid_title: ui.get_grid_title(),
        grid_square: ui.get_grid_square(),
        rows: ui.get_search_rows(),
        person: ui.get_person(),
        person_id: if ui.get_selected_library() == PERSON_PAGE { CURRENT.with_borrow(|c| c.clone()) } else { None },
        detail,
    };
    STACK.with_borrow_mut(|s| s.push(frame));
    // The name and photo are on the card already: show them while the rest loads.
    let card = with_models(|m| m.people.iter().find(|p| p.id == id.as_str()));
    let header = PersonHeader {
        name: card.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
        image: card.as_ref().map(|c| c.image.clone()).unwrap_or_default(),
        art_url: card.map(|c| c.art_url).unwrap_or_default(),
        ..Default::default()
    };
    load(&ui, id, header);
}

/// Load the person on the page again.
pub(crate) fn reload() {
    if let Some(id) = CURRENT.with_borrow(|c| c.clone()) {
        let ui = ui();
        load(&ui, id, ui.get_person());
    }
}

/// Somewhere else was picked (a library, Home): the way back no longer applies.
pub(crate) fn forget() {
    STACK.with_borrow_mut(Vec::clear);
    CURRENT.with_borrow_mut(|c| *c = None);
}

/// Back from a person's page: the title it was opened from, else the page before. False when
/// there's nothing to go back to.
pub(crate) fn back() -> bool {
    let ui = ui();
    if ui.get_screen() != "home" || ui.get_selected_library() != PERSON_PAGE {
        return false;
    }
    let Some(f) = STACK.with_borrow_mut(Vec::pop) else {
        CURRENT.with_borrow_mut(|c| *c = None);
        return false;
    };
    with_state(|s| s.generation += 1); // drop a load still under way
    ui.set_selected_library(f.selected);
    with_models(|m| m.grid.set_vec(f.grid));
    ui.set_grid_title(f.grid_title);
    ui.set_grid_square(f.grid_square);
    ui.set_search_rows(f.rows);
    ui.set_person(f.person);
    ui.set_loading(false);
    CURRENT.with_borrow_mut(|c| *c = f.person_id);
    match f.detail {
        Some(item) => show_detail(item),
        None if ui.global::<Nav>().get_active() => set_focus(&ui, "content", 0, 0),
        None => {}
    }
    true
}

fn load(ui: &AppWindow, id: String, header: PersonHeader) {
    CURRENT.with_borrow_mut(|c| *c = Some(id.clone()));
    let generation = with_state(|s| {
        s.generation += 1;
        s.share_view = None;
        s.generation
    });
    ui.set_screen("home".into());
    ui.set_selected_library(PERSON_PAGE);
    ui.set_grid_square(false);
    ui.set_grid_title(header.name.clone());
    ui.set_person(header);
    ui.set_loading(true);
    with_models(|m| m.grid.set_vec(Vec::<Poster>::new()));
    ui.set_search_rows(ModelRc::default());
    if ui.global::<Nav>().get_active() {
        set_focus(ui, "content", 0, 0);
    }
    let jellyfin = backend::jf_number(&id).is_some();
    let server = if jellyfin { backend_for(&id) } else { None };
    let account = suggest::account();
    let libraries = suggest::libraries();
    rt().spawn(async move {
        let found: Result<Option<encheo_plex::Person>, String> = match (server, account) {
            (Some(backend::Backend::Jf(s)), _) => s.person(&id).await.map_err(|e| e.to_string()),
            (_, Some((plex, token))) if !jellyfin => plex.catalog_person(&token, &id).await.map_err(|e| e.to_string()),
            _ => Err("their page needs a Plex account".into()),
        };
        // Which of their titles you have (Plex's catalogue lists them all).
        let index = if jellyfin { Default::default() } else { suggest::index(libraries).await };
        on_ui(move |ui| {
            if with_state(|s| s.generation) != generation {
                return;
            }
            ui.set_loading(false);
            match found {
                Ok(Some(p)) => show(ui, p, &index),
                Ok(None) => toast(ui, "Couldn't find this person."),
                Err(e) => toast(ui, format!("Couldn't load their page: {e}")),
            }
        });
    });
}

fn show(ui: &AppWindow, p: encheo_plex::Person, index: &[Item]) {
    let url = art_url(p.thumb.as_deref(), 300, 375);
    ui.set_grid_title(p.name.clone().into());
    ui.set_person(PersonHeader {
        name: p.name.clone().into(),
        about: about(p.born.as_deref(), p.died.as_deref(), p.birth_place.as_deref()).into(),
        summary: p.summary.as_deref().map(|s| short_bio(s, 700)).unwrap_or_default().into(),
        image: want_image(&url),
        art_url: url.into(),
    });
    let row = |title: String, items: &[Item]| HomeRow {
        title: title.into(),
        library: -1,
        landscape: false,
        square: false,
        items: ModelRc::new(VecModel::from(items.iter().map(|i| poster(i, false)).collect::<Vec<_>>())),
    };
    let mut rows = Vec::new();
    let mut all = Vec::new();
    // Titles you have first, each once.
    let mut yours: Vec<Item> = Vec::new();
    let mut groups: Vec<(String, Vec<Item>)> = Vec::new();
    // Their own work first; talk shows, documentaries and the odd extra credit last.
    let mut credits = p.credits;
    credits.sort_by_key(|g| matches!(g.title.as_str(), "Appearances" | "Additional Credits"));
    for g in credits {
        let items: Vec<Item> = g
            .items
            .into_iter()
            .take(ROW_MAX)
            .map(|c| match suggest::own_copy(index, &c) {
                Some(own) => {
                    if !yours.iter().any(|y| y.rating_key == own.rating_key) {
                        yours.push(own.clone());
                    }
                    own.clone()
                }
                // (Jellyfin's are the server's own already.)
                None if backend::jf_number(&c.rating_key).is_some() => c,
                None => suggest::catalog_id(c),
            })
            .collect();
        groups.push((g.title, items));
    }
    let only_yours = groups.len() == 1 && groups[0].0 == "In your library";
    if !yours.is_empty() && !only_yours {
        rows.push(row("In your library".into(), &yours));
    }
    for (title, items) in groups {
        rows.push(row(title, &items));
        all.extend(items);
    }
    if rows.is_empty() {
        toast(ui, format!("No films or shows found for {}.", p.name));
    }
    all.extend(yours);
    remember(all);
    ui.set_search_rows(ModelRc::new(VecModel::from(rows)));
}

/// "Born 11 November 1974 in Los Angeles, California, USA · Died …".
fn about(born: Option<&str>, died: Option<&str>, place: Option<&str>) -> String {
    let mut parts = Vec::new();
    match (born.and_then(long_date), place.filter(|p| !p.is_empty())) {
        (Some(d), Some(p)) => parts.push(format!("Born {d} in {p}")),
        (Some(d), None) => parts.push(format!("Born {d}")),
        (None, Some(p)) => parts.push(format!("Born in {p}")),
        (None, None) => {}
    }
    if let Some(d) = died.and_then(long_date) {
        parts.push(format!("Died {d}"));
    }
    parts.join(" · ")
}

/// `1974-11-11` → "11 November 1974".
fn long_date(d: &str) -> Option<String> {
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let mut it = d.get(..10)?.split('-');
    let (y, m, day) = (it.next()?, it.next()?.parse::<usize>().ok()?, it.next()?.parse::<u32>().ok()?);
    Some(format!("{day} {} {y}", MONTHS.get(m.checked_sub(1)?)?))
}

/// The biography, cut after the sentence that passes `max` characters.
fn short_bio(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    match cut.rfind(". ") {
        Some(end) if end > max / 3 => cut[..=end].to_string(),
        _ => format!("{}…", cut.trim_end()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn birth_line() {
        assert_eq!(super::about(Some("1974-11-11"), None, Some("Los Angeles, California, USA")), "Born 11 November 1974 in Los Angeles, California, USA");
        assert_eq!(super::about(Some("1930-08-25T00:00:00.000Z"), Some("2020-10-31"), None), "Born 25 August 1930 · Died 31 October 2020");
        assert_eq!(super::about(None, None, None), "");
        assert_eq!(super::long_date("1974-13-01"), None);
    }

    #[test]
    fn bio_cut_at_a_sentence() {
        let bio = "First sentence here. Second one is a little longer. Third.";
        assert_eq!(super::short_bio(bio, 200), bio);
        assert_eq!(super::short_bio(bio, 40), "First sentence here.");
    }
}
