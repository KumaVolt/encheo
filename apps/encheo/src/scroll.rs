//! Where Home was scrolled to. Home is rebuilt each time it's shown (after a title's page, the
//! player, …), so its positions live on the window (the page and grid) and here (each row's
//! sideways position), and come back with it. The grid goes back to the top when the user picks
//! something new to show in it (a library, a filter, a search, a folder); reloads, such as after
//! playback, keep the position.

use super::*;

thread_local! {
    /// Each Home row's sideways position, by the row's title.
    static ROWS: RefCell<HashMap<String, f32>> = RefCell::new(HashMap::new());
}

pub(crate) fn install(ui: &AppWindow) {
    ui.on_home_row_x(|title| ROWS.with(|r| r.borrow().get(title.as_str()).copied().unwrap_or(0.0)));
    ui.on_home_set_row_x(|title, x| {
        ROWS.with(|r| {
            let mut rows = r.borrow_mut();
            match rows.get_mut(title.as_str()) {
                Some(saved) => *saved = x,
                None => {
                    rows.insert(title.to_string(), x);
                }
            }
        })
    });
}

/// New content for the grid: it starts at the top.
pub(crate) fn grid_to_top(ui: &AppWindow) {
    ui.set_home_grid_y(0.0);
    ui.set_home_grid_reset(ui.get_home_grid_reset().wrapping_add(1));
}
