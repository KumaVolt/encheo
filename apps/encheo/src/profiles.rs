//! Plex Home profiles: "Who's watching?".
//!
//! A Plex Home is one account shared by several people, each with their own watched list,
//! Continue Watching and watchlist. With two or more people the picker opens at start (on the
//! profile used last) and from Settings. Choosing someone swaps the token Encheo uses and reloads
//! Home with that person's server access; protected profiles ask for their 4-digit PIN.
//!
//! Jellyfin and Emby users aren't here yet: each of those servers is still one signed-in user.

use super::*;
use encheo_plex::HomeUser;
use slint::platform::Key;
use std::cell::RefCell;

thread_local! {
    /// The PIN being typed: for which profile (index in the list), and the digits so far.
    static PIN: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
}

/// The keypad, row by row: digits, then "cancel" and "delete" either side of 0.
const PAD: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "cancel", "0", "delete"];
/// Profiles per row in the picker.
const PER_ROW: i32 = 5;

/// Startup with a saved session: the picker straight away when the Home has two or more people
/// (from the last check), and a fresh check in the background.
pub(crate) fn at_start() {
    let several = with_state(|s| s.session.home_users.len() >= 2);
    show_list(&ui());
    if several {
        open();
    }
    refresh(false);
}

/// Just signed in: ask plex.tv who's in the Home, and open the picker when there's a choice.
pub(crate) fn after_signin() {
    with_state(|s| {
        s.session.account_token = None;
        s.session.profile = None;
        s.session.home_users.clear();
    });
    show_list(&ui());
    refresh(true);
}

/// The Home's people from plex.tv; `open_after` shows the picker when there are two or more.
fn refresh(open_after: bool) {
    let (plex, token) = with_state(|s| (plex(&s.session), s.session.account_token()));
    let Some(token) = token else { return };
    rt().spawn(async move {
        let Ok(users) = plex.home_users(&token).await else { return };
        on_ui(move |ui| {
            let several = users.len() >= 2;
            with_state(|s| {
                s.session.home_users = users;
                s.session.save();
            });
            show_list(ui);
            if ui.get_sources_open() {
                shares::fill_sources();
            }
            if open_after && several {
                open();
            }
        });
    });
}

/// The profile in use: the chosen one, or the account owner.
fn current(s: &State) -> Option<usize> {
    let users = &s.session.home_users;
    match &s.session.profile {
        Some(uuid) => users.iter().position(|u| &u.uuid == uuid),
        None => users.iter().position(|u| u.admin),
    }
}

/// Who's watching, for Settings ("" without a Home).
pub(crate) fn current_name() -> String {
    with_state(|s| {
        if s.session.home_users.len() < 2 {
            return String::new();
        }
        current(s).map(|i| s.session.home_users[i].title.clone()).unwrap_or_default()
    })
}

fn show_list(ui: &AppWindow) {
    let (users, cur) = with_state(|s| (s.session.home_users.clone(), current(s)));
    let entries: Vec<ProfileEntry> = users
        .iter()
        .enumerate()
        .map(|(i, u)| ProfileEntry {
            name: u.title.clone().into(),
            initial: u.title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
            art_url: u.thumb.clone().unwrap_or_default().into(),
            locked: u.protected,
            current: Some(i) == cur,
        })
        .collect();
    ui.set_profiles(ModelRc::new(VecModel::from(entries)));
    ui.set_can_switch_profile(users.len() >= 2);
}

/// Open the picker on the profile in use.
pub(crate) fn open() {
    let ui = ui();
    PIN.with(|p| *p.borrow_mut() = None);
    ui.set_pin_name(SharedString::new());
    ui.set_profile_error(SharedString::new());
    ui.set_profile_busy(false);
    ui.set_sources_open(false);
    ui.set_profile_focus(with_state(|s| current(s)).unwrap_or(0) as i32);
    ui.set_profiles_open(true);
}

fn close(ui: &AppWindow) {
    PIN.with(|p| *p.borrow_mut() = None);
    ui.set_profiles_open(false);
    ui.set_pin_name(SharedString::new());
}

/// Back while the picker is open: out of the PIN pad, or close (staying with the profile in use).
pub(crate) fn back() -> bool {
    let ui = ui();
    if !ui.get_profiles_open() {
        return false;
    }
    if ui.get_profile_busy() {
        return true;
    }
    if PIN.with(|p| p.borrow_mut().take()).is_some() {
        ui.set_pin_name(SharedString::new());
        ui.set_profile_error(SharedString::new());
    } else {
        close(&ui);
    }
    true
}

/// A profile clicked or chosen with OK.
pub(crate) fn pick(index: usize) {
    let ui = ui();
    let (user, cur) = with_state(|s| (s.session.home_users.get(index).cloned(), current(s)));
    let Some(user) = user else { return };
    ui.set_profile_error(SharedString::new());
    if Some(index) == cur {
        return close(&ui);
    }
    if user.protected {
        PIN.with(|p| *p.borrow_mut() = Some((index, String::new())));
        ui.set_pin_name(user.title.into());
        ui.set_pin_length(0);
        ui.set_pin_focus(0);
        return;
    }
    switch(user, None);
}

/// A key on the PIN pad (clicked, chosen with OK, or typed): a digit, "delete" or "cancel".
pub(crate) fn pin_key(key: &str) {
    let ui = ui();
    if ui.get_profile_busy() {
        return;
    }
    if key == "cancel" {
        back();
        return;
    }
    let done = PIN.with(|p| {
        let mut p = p.borrow_mut();
        let (index, digits) = p.as_mut()?;
        if key == "delete" {
            digits.pop();
        } else if key.len() == 1 && key.chars().all(|c| c.is_ascii_digit()) && digits.len() < 4 {
            digits.push_str(key);
        }
        ui.set_pin_length(digits.len() as i32);
        (digits.len() == 4).then(|| (*index, digits.clone()))
    });
    if let Some((index, pin)) = done
        && let Some(user) = with_state(|s| s.session.home_users.get(index).cloned())
    {
        ui.set_profile_error(SharedString::new());
        switch(user, Some(pin));
    }
}

/// Become `user`: their token, then their servers, then Home.
fn switch(user: HomeUser, pin: Option<String>) {
    let ui = ui();
    let (plex, account, server) = with_state(|s| (plex(&s.session), s.session.account_token(), s.session.server.clone()));
    let Some(account) = account else { return };
    ui.set_profile_busy(true);
    rt().spawn(async move {
        let result = plex.switch_user(&account, &user.uuid, pin.as_deref()).await;
        on_ui(move |ui| {
            ui.set_profile_busy(false);
            match result {
                Ok(token) => {
                    with_state(|s| {
                        s.session.account_token = Some(account);
                        s.session.token = Some(token.clone());
                        s.session.profile = Some(user.uuid.clone());
                        s.session.save();
                        // Their own subtitle language, watchlist and server access from here on.
                        s.subtitle_language = None;
                        s.server = None;
                    });
                    close(ui);
                    show_list(ui);
                    ui.set_screen("home".into());
                    ui.set_loading(true);
                    toast(ui, format!("Watching as {}", user.title));
                    choose_server(token, server.map(|s| s.client_identifier));
                }
                Err(encheo_plex::Error::Status { status: 401 | 403, .. }) if pin.is_some() => {
                    PIN.with(|p| {
                        if let Some((_, digits)) = p.borrow_mut().as_mut() {
                            digits.clear();
                        }
                    });
                    ui.set_pin_length(0);
                    ui.set_profile_error("That PIN isn't right. Try again.".into());
                }
                Err(e) => {
                    PIN.with(|p| {
                        if let Some((_, digits)) = p.borrow_mut().as_mut() {
                            digits.clear();
                        }
                    });
                    ui.set_pin_length(0);
                    ui.set_profile_error(format!("Couldn't switch to {}: {e}", user.title).into());
                }
            }
        });
    });
}

/// Remote and keyboard while the picker is open. Always true: nothing under it gets the keys.
pub(crate) fn key(key: &str) -> bool {
    let is = |k: Key| SharedString::from(k).as_str() == key;
    let ui = ui();
    if is(Key::Escape) || is(Key::Back) {
        back();
        return true;
    }
    let enter = key == "\n" || key == "\r" || key == " ";
    let pin = PIN.with(|p| p.borrow().is_some());
    if pin {
        if is(Key::Backspace) || is(Key::Delete) {
            pin_key("delete");
        } else if key.len() == 1 && key.chars().all(|c| c.is_ascii_digit()) {
            pin_key(key);
        } else if enter {
            pin_key(PAD[ui.get_pin_focus().clamp(0, 11) as usize]);
        } else {
            let f = ui.get_pin_focus();
            let (r, c) = (f / 3, f % 3);
            let (r, c) = if is(Key::UpArrow) {
                ((r - 1).max(0), c)
            } else if is(Key::DownArrow) {
                ((r + 1).min(3), c)
            } else if is(Key::LeftArrow) {
                (r, (c - 1).max(0))
            } else if is(Key::RightArrow) {
                (r, (c + 1).min(2))
            } else {
                (r, c)
            };
            ui.set_pin_focus(r * 3 + c);
        }
        return true;
    }
    let n = with_state(|s| s.session.home_users.len()) as i32;
    if n == 0 {
        return true;
    }
    let f = ui.get_profile_focus().clamp(0, n - 1);
    if enter {
        pick(f as usize);
        return true;
    }
    let next = if is(Key::LeftArrow) {
        f - 1
    } else if is(Key::RightArrow) {
        f + 1
    } else if is(Key::UpArrow) {
        f - PER_ROW
    } else if is(Key::DownArrow) {
        f + PER_ROW
    } else {
        f
    };
    if (0..n).contains(&next) {
        ui.set_profile_focus(next);
    } else if is(Key::DownArrow) {
        ui.set_profile_focus(n - 1);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::PAD;

    #[test]
    fn keypad_has_every_digit_once() {
        for d in 0..10 {
            assert_eq!(PAD.iter().filter(|k| **k == d.to_string()).count(), 1);
        }
    }
}
