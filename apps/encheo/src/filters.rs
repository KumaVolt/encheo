//! Movie and TV libraries: sort, filters and the A–Z rail.
//!
//! A bar of chips above the grid (Sort, watched state, Genre, Country, Decade, Age rating, Clear) opens a
//! panel on the right with the choices; the choices are asked of the server, and kept per library
//! across restarts. Sorted by title, a rail of letters on the right jumps through the grid: the
//! grid holds the whole library once loaded, so the letters come from the titles themselves.

use super::*;
use encheo_plex::{Filter, LibraryQuery, Sort, Watched};
use slint::platform::Key;

/// The kinds of chip, in bar order. Country shows where the server can filter by it, Clear
/// while something is filtered; `kinds` says which are in the bar.
const SORT: i32 = 0;
const WATCHED: i32 = 1;
const GENRE: i32 = 2;
const COUNTRY: i32 = 3;
const DECADE: i32 = 4;
const RATING: i32 = 5;
const CLEAR: i32 = 6;
/// Not a chip: the panel listing a show's seasons to request (see `requests`).
pub(crate) const SEASONS: i32 = 100;

const SORTS: [(Sort, &str); 4] =
    [(Sort::Title, "Title"), (Sort::Added, "Recently added"), (Sort::Released, "Release date"), (Sort::Rating, "Rating")];

/// The library whose grid is shown, when it's a movie or TV library.
#[derive(Clone, Debug)]
pub(crate) struct Current {
    key: String,
    kind: String,
    query: LibraryQuery,
}

pub(crate) fn applies(kind: &str) -> bool {
    kind == "movie" || kind == "show"
}

/// Where a library's choices are saved. Jellyfin and Emby keys already name their server;
/// Plex's are section numbers ("1"), so they get the server's id.
fn saved_key(library_key: &str) -> String {
    if backend::jf_number(library_key).is_some() {
        return library_key.to_string();
    }
    let server = with_state(|s| s.session.server.as_ref().map(|x| x.client_identifier.clone())).unwrap_or_default();
    format!("plex:{server}:{library_key}")
}

/// A movie or TV library is opening: its saved choices, the bar, and (once) what it can be
/// filtered by. Returns the query to list it with.
pub(crate) fn begin(ui: &AppWindow, key: &str, kind: &str, backend: &backend::Backend) -> LibraryQuery {
    // (The key first: it reads the state too.)
    let saved = saved_key(key);
    let query = with_state(|s| s.session.library_filters.get(&saved).cloned()).unwrap_or_default();
    with_state(|s| s.filters = Some(Current { key: key.into(), kind: kind.into(), query: query.clone() }));
    ui.set_az(ModelRc::default());
    show_chips(ui);
    if with_state(|s| !s.filter_options.contains_key(key)) {
        let (backend, key, kind) = (backend.clone(), key.to_string(), kind.to_string());
        rt().spawn(async move {
            let Ok(options) = backend.filter_options(&key, &kind).await else { return };
            on_ui(move |ui| {
                with_state(|s| s.filter_options.insert(key, options));
                show_chips(ui);
                // A panel waiting for these: fill it in.
                let open = ui.get_filter_sheet();
                if open >= 0 {
                    let focus = ui.get_filter_sheet_focus();
                    open_sheet(ui, open);
                    ui.set_filter_sheet_focus(focus);
                }
            });
        });
    }
    query
}

/// Any other grid (music, a folder, search): no bar, no rail.
pub(crate) fn clear(ui: &AppWindow) {
    with_state(|s| s.filters = None);
    ui.set_filter_chips(ModelRc::default());
    ui.set_az(ModelRc::default());
    ui.set_filter_sheet(-1);
}

fn current() -> Option<Current> {
    with_state(|s| s.filters.clone())
}

fn watched_name(w: Watched) -> &'static str {
    match w {
        Watched::All => "All titles",
        Watched::Unwatched => "Unwatched",
        Watched::Watched => "Watched",
        Watched::Progress => "In progress",
    }
}

/// The chips in the bar, left to right.
fn kinds() -> Vec<i32> {
    let Some(c) = current() else { return vec![] };
    let countries = with_state(|s| s.filter_options.get(&c.key).is_some_and(|o| !o.countries.is_empty()));
    let mut v = vec![SORT, WATCHED, GENRE];
    if countries || c.query.country.is_some() {
        v.push(COUNTRY);
    }
    v.extend([DECADE, RATING]);
    if c.query.filtered() {
        v.push(CLEAR);
    }
    v
}

fn show_chips(ui: &AppWindow) {
    let Some(c) = current() else { return };
    let q = &c.query;
    let sort = SORTS.iter().find(|(s, _)| *s == q.sort).map_or("Title", |(_, n)| n);
    let chip = |text: String, active: bool| FilterChip { text: text.into(), active, menu: true };
    let chips: Vec<FilterChip> = kinds()
        .into_iter()
        .map(|k| match k {
            SORT => chip(format!("Sort: {sort}"), false),
            WATCHED => chip(watched_name(q.watched).into(), q.watched != Watched::All),
            GENRE => chip(q.genre.as_ref().map_or("Genre".into(), |g| g.title.clone()), q.genre.is_some()),
            COUNTRY => chip(q.country.as_ref().map_or("Country".into(), |c| c.title.clone()), q.country.is_some()),
            DECADE => chip(q.decade.map_or("Decade".into(), |d| format!("{d}s")), q.decade.is_some()),
            RATING => chip(q.rating.as_ref().map_or("Age rating".into(), |r| r.title.clone()), q.rating.is_some()),
            _ => FilterChip { text: "Clear".into(), active: false, menu: false },
        })
        .collect();
    ui.set_filter_chips(ModelRc::new(VecModel::from(chips)));
}

/// The choices behind a chip's panel: what each row sets, in order.
enum Choice {
    Sort(Sort),
    Watched(Watched),
    Genre(Option<Filter>),
    Country(Option<Filter>),
    Decade(Option<u32>),
    Rating(Option<Filter>),
}

fn choices(chip: i32) -> (String, Vec<(String, Choice)>, usize) {
    let Some(c) = current() else { return (String::new(), vec![], 0) };
    let options = with_state(|s| s.filter_options.get(&c.key).cloned()).unwrap_or_default();
    let q = &c.query;
    let pos = |found: Option<usize>| found.map_or(0, |i| i + 1);
    match chip {
        SORT => (
            "Sort by".into(),
            SORTS.iter().map(|(s, n)| (n.to_string(), Choice::Sort(*s))).collect(),
            SORTS.iter().position(|(s, _)| *s == q.sort).unwrap_or(0),
        ),
        WATCHED => {
            // Plex has no "in progress" for shows.
            let mut list = vec![Watched::All, Watched::Unwatched, Watched::Watched];
            if c.kind == "movie" || backend::jf_number(&c.key).is_some() {
                list.push(Watched::Progress);
            }
            let at = list.iter().position(|w| *w == q.watched).unwrap_or(0);
            ("Show".into(), list.into_iter().map(|w| (watched_name(w).to_string(), Choice::Watched(w))).collect(), at)
        }
        GENRE => {
            let at = pos(options.genres.iter().position(|g| Some(&g.key) == q.genre.as_ref().map(|g| &g.key)));
            let mut list = vec![("All genres".to_string(), Choice::Genre(None))];
            list.extend(options.genres.iter().map(|g| (g.title.clone(), Choice::Genre(Some(g.clone())))));
            ("Genre".into(), list, at)
        }
        COUNTRY => {
            let at = pos(options.countries.iter().position(|x| Some(&x.key) == q.country.as_ref().map(|x| &x.key)));
            let mut list = vec![("All countries".to_string(), Choice::Country(None))];
            list.extend(options.countries.iter().map(|x| (x.title.clone(), Choice::Country(Some(x.clone())))));
            ("Country".into(), list, at)
        }
        DECADE => {
            let at = pos(options.decades.iter().position(|d| Some(*d) == q.decade));
            let mut list = vec![("All decades".to_string(), Choice::Decade(None))];
            list.extend(options.decades.iter().map(|d| (format!("{d}s"), Choice::Decade(Some(*d)))));
            ("Decade".into(), list, at)
        }
        RATING => {
            let at = pos(options.ratings.iter().position(|r| Some(&r.key) == q.rating.as_ref().map(|r| &r.key)));
            let mut list = vec![("All age ratings".to_string(), Choice::Rating(None))];
            list.extend(options.ratings.iter().map(|r| (r.title.clone(), Choice::Rating(Some(r.clone())))));
            ("Age rating".into(), list, at)
        }
        _ => (String::new(), vec![], 0),
    }
}

/// A chip clicked, or chosen with OK (`index` in the bar).
pub(crate) fn chip(index: i32) {
    let ui = ui();
    let Some(&kind) = kinds().get(index.max(0) as usize) else { return };
    if kind == CLEAR {
        return apply(&ui, |q| *q = LibraryQuery { sort: q.sort, ..Default::default() });
    }
    open_sheet(&ui, kind);
}

/// Open the panel for a kind of chip (the panel keeps the kind in `filter-sheet`).
fn open_sheet(ui: &AppWindow, chip: i32) {
    let (title, list, at) = choices(chip);
    if list.is_empty() {
        return;
    }
    let names: Vec<SharedString> = list.iter().map(|(n, _)| n.into()).collect();
    ui.set_filter_sheet_title(title.into());
    ui.set_filter_sheet_options(ModelRc::new(VecModel::from(names)));
    ui.set_filter_sheet_current(at as i32);
    ui.set_filter_sheet_focus(at as i32);
    ui.set_filter_sheet(chip);
}

/// The same panel for another list of choices, `owner` saying whose they are.
pub(crate) fn open_choices(title: &str, options: Vec<String>, owner: i32) {
    let ui = ui();
    let names: Vec<SharedString> = options.into_iter().map(Into::into).collect();
    ui.set_filter_sheet_title(title.into());
    ui.set_filter_sheet_options(ModelRc::new(VecModel::from(names)));
    ui.set_filter_sheet_current(-1);
    ui.set_filter_sheet_focus(0);
    ui.set_filter_sheet(owner);
}

pub(crate) fn close_sheet() {
    ui().set_filter_sheet(-1);
}

/// Back while the panel is open closes it.
pub(crate) fn back() -> bool {
    let ui = ui();
    if ui.get_filter_sheet() < 0 {
        return false;
    }
    ui.set_filter_sheet(-1);
    true
}

/// A choice in the panel: apply it and list the library again.
pub(crate) fn pick(index: i32) {
    let ui = ui();
    let chip = ui.get_filter_sheet();
    ui.set_filter_sheet(-1);
    if chip == SEASONS {
        return crate::requests::pick_season(index.max(0) as usize);
    }
    let (_, list, _) = choices(chip);
    let Some((_, choice)) = list.into_iter().nth(index.max(0) as usize) else { return };
    apply(&ui, move |q| match choice {
        Choice::Sort(s) => q.sort = s,
        Choice::Watched(w) => q.watched = w,
        Choice::Genre(g) => q.genre = g,
        Choice::Country(c) => q.country = c,
        Choice::Decade(d) => q.decade = d,
        Choice::Rating(r) => q.rating = r,
    });
}

fn apply(ui: &AppWindow, change: impl FnOnce(&mut LibraryQuery)) {
    let Some(mut c) = current() else { return };
    let before = c.query.clone();
    change(&mut c.query);
    if c.query == before {
        return;
    }
    let key = saved_key(&c.key);
    with_state(|s| {
        if c.query == LibraryQuery::default() {
            s.session.library_filters.remove(&key);
        } else {
            s.session.library_filters.insert(key, c.query.clone());
        }
        s.session.save();
        s.filters = Some(c);
    });
    select_library(ui.get_selected_library());
    // Clear goes away once nothing is filtered: the focus stays on the bar.
    let nav = ui.global::<Nav>();
    if nav.get_zone() == "filters" {
        nav.set_col(nav.get_col().min(ui.get_filter_chips().row_count() as i32 - 1));
    }
}

/// Remote and keyboard while the panel is open. Nothing under it gets the keys.
pub(crate) fn sheet_key(key: &str) {
    let ui = ui();
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let n = ui.get_filter_sheet_options().row_count() as i32;
    let f = ui.get_filter_sheet_focus();
    if is(Key::Escape) || is(Key::Back) || is(Key::Backspace) || is(Key::LeftArrow) {
        close_sheet();
    } else if key == "\n" || key == "\r" || key == " " {
        pick(f);
    } else if is(Key::UpArrow) {
        ui.set_filter_sheet_focus((f - 1).max(0));
    } else if is(Key::DownArrow) {
        ui.set_filter_sheet_focus((f + 1).min(n - 1));
    } else if is(Key::PageUp) {
        ui.set_filter_sheet_focus((f - 8).max(0));
    } else if is(Key::PageDown) {
        ui.set_filter_sheet_focus((f + 8).min(n - 1));
    }
}

/// The letter a title files under: A–Z, accents folded, or # for digits, symbols and other
/// scripts. Goes by the sort title, as the server sorts ("The Matrix" under M).
pub(crate) fn letter(item: &Item) -> char {
    let title = item.title_sort.as_deref().filter(|t| !t.is_empty()).unwrap_or(&item.title);
    match title.chars().next().map(fold) {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
        _ => '#',
    }
}

fn fold(c: char) -> char {
    crate::suggest::fold(c.to_lowercase().next().unwrap_or(c))
}

/// The rail's letters (#, A–Z) for titles in grid order: where each letter starts, and for a
/// letter without titles, where the next one does (or the last title).
pub(crate) fn rail(letters: &[char]) -> Vec<AzLetter> {
    let labels: Vec<char> = std::iter::once('#').chain('A'..='Z').collect();
    let first: Vec<Option<usize>> = labels.iter().map(|l| letters.iter().position(|c| c == l)).collect();
    let last = letters.len().saturating_sub(1);
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let index = first[i..].iter().flatten().next().copied().unwrap_or(last);
            AzLetter { label: l.to_string().into(), index: index as i32, present: first[i].is_some() }
        })
        .collect()
}

/// Refresh the rail after the grid changed: shown while a movie or TV library is sorted by title.
pub(crate) fn update_rail(ui: &AppWindow) {
    let by_title = current().is_some_and(|c| c.query.sort == Sort::Title);
    let ids: Vec<SharedString> = if by_title { with_models(|m| m.grid.iter().map(|p| p.id).collect()) } else { vec![] };
    if ids.len() < 2 {
        ui.set_az(ModelRc::default());
        return;
    }
    let letters: Vec<char> = with_state(|s| ids.iter().map(|id| s.items.get(id.as_str()).map_or('#', letter)).collect());
    ui.set_az(ModelRc::new(VecModel::from(rail(&letters))));
}

/// Remote on the bar of chips.
pub(crate) fn bar_nav(ui: &AppWindow, col: i32, key: &str, enter: bool, to_side: impl Fn(&AppWindow)) -> bool {
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let n = ui.get_filter_chips().row_count() as i32;
    if enter {
        chip(col);
    } else if is(Key::LeftArrow) {
        if col == 0 { to_side(ui) } else { set_focus(ui, "filters", 0, col - 1) }
    } else if is(Key::RightArrow) {
        set_focus(ui, "filters", 0, (col + 1).min(n - 1));
    } else if is(Key::DownArrow) && with_models(|m| m.grid.row_count()) > 0 {
        set_focus(ui, "content", 0, 0);
    }
    true
}

/// Remote on the A–Z rail: up and down through the letters that have titles (the grid follows),
/// OK or Left to the first title of the letter.
pub(crate) fn rail_nav(ui: &AppWindow, row: i32, key: &str, enter: bool) -> bool {
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let az: Vec<AzLetter> = ui.get_az().iter().collect();
    if az.is_empty() {
        set_focus(ui, "content", 0, 0);
        return true;
    }
    let row = row.clamp(0, az.len() as i32 - 1);
    let step = |dir: i32| {
        let mut r = row + dir;
        while (0..az.len() as i32).contains(&r) {
            if az[r as usize].present {
                return r;
            }
            r += dir;
        }
        row
    };
    if enter || is(Key::LeftArrow) {
        let cols = ui.global::<Nav>().get_grid_cols().max(1);
        let i = az[row as usize].index;
        set_focus(ui, "content", i / cols, i % cols);
    } else if is(Key::UpArrow) {
        set_focus(ui, "az", step(-1), 0);
    } else if is(Key::DownArrow) {
        set_focus(ui, "az", step(1), 0);
    }
    true
}

/// From the grid onto the rail, at the letter of the title the focus was on.
pub(crate) fn to_rail(ui: &AppWindow, index: i32) -> bool {
    let az: Vec<AzLetter> = ui.get_az().iter().collect();
    let Some(row) = az.iter().rposition(|l| l.present && l.index <= index) else { return false };
    set_focus(ui, "az", row as i32, 0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, sort: Option<&str>) -> Item {
        Item { title: title.into(), title_sort: sort.map(String::from), ..Default::default() }
    }

    #[test]
    fn letters_follow_the_sort_title() {
        assert_eq!(letter(&item("The Matrix", Some("Matrix"))), 'M');
        assert_eq!(letter(&item("Amélie", None)), 'A');
        assert_eq!(letter(&item("Élite", None)), 'E');
        assert_eq!(letter(&item("The 'Burbs", Some("'Burbs"))), '#');
        assert_eq!(letter(&item("28 Years Later", None)), '#');
        assert_eq!(letter(&item("進撃の巨人", None)), '#');
        assert_eq!(letter(&item("boy in plastic bubble", Some(""))), 'B');
    }

    #[test]
    fn rail_points_missing_letters_at_the_next_one() {
        let az = rail(&['#', '#', 'A', 'C', 'C', 'Z']);
        assert_eq!(az.len(), 27);
        let at = |l: &str| az.iter().find(|x| x.label == l).unwrap().clone();
        assert_eq!((at("#").index, at("#").present), (0, true));
        assert_eq!((at("A").index, at("A").present), (2, true));
        assert_eq!((at("B").index, at("B").present), (3, false));
        assert_eq!(at("C").index, 3);
        assert_eq!((at("Y").index, at("Y").present), (5, false));
        assert_eq!(at("Z").index, 5);
        // Nothing after the last letter: the last title.
        let az = rail(&['A', 'B']);
        assert_eq!(az.last().unwrap().index, 1);
    }
}
