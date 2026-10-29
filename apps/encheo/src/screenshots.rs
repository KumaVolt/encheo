//! Development: `ENCHEO_SCREENSHOTS=<folder>` walks through the main screens once the app has
//! started, saves a PNG of each and quits. `scripts/screenshots/run.sh` points the app at a demo
//! server first; CI runs that to make the README's screenshots.

use crate::{enter_home, open_item, select_library, ui, with_models, with_state};
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long a screen must stay loaded (artwork included) before it's saved, so fades finish.
const SETTLE: Duration = Duration::from_millis(1500);
/// Save whatever is there after this long rather than hang.
const GIVE_UP: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq)]
enum Shot {
    Home,
    Library,
    Movie,
    Show,
    Album,
}

impl Shot {
    const ALL: [Shot; 5] = [Shot::Home, Shot::Library, Shot::Movie, Shot::Show, Shot::Album];

    fn name(self) -> &'static str {
        match self {
            Shot::Home => "home",
            Shot::Library => "library",
            Shot::Movie => "movie",
            Shot::Show => "show",
            Shot::Album => "album",
        }
    }
}

struct Run {
    dir: PathBuf,
    next: usize,
    /// When the current screen was asked for, and since when it has looked finished.
    started: Instant,
    ready_since: Option<Instant>,
    /// The first title of each Recently Added row, by library kind, picked up on the home screen.
    firsts: Vec<(String, String)>,
}

pub fn start(dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("screenshots: can't create {}: {e}", dir.display());
        return;
    }
    let ui = ui();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    let run = Rc::new(RefCell::new(Run { dir, next: 0, started: Instant::now(), ready_since: None, firsts: Vec::new() }));
    let timer = Box::leak(Box::new(slint::Timer::default()));
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || step(&mut run.borrow_mut()));
}

fn step(run: &mut Run) {
    let Some(&shot) = Shot::ALL.get(run.next) else { return };
    let ready = ready(shot);
    let now = Instant::now();
    run.ready_since = if ready { run.ready_since.or(Some(now)) } else { None };
    let settled = run.ready_since.is_some_and(|t| now - t >= SETTLE);
    let late = now - run.started >= GIVE_UP;
    if !settled && !late {
        return;
    }
    if late {
        eprintln!("screenshots: {} didn't finish loading; saving it as it is", shot.name());
    }
    if shot == Shot::Home {
        run.firsts = first_recent_titles();
    }
    save(&run.dir, shot);
    run.started = now;
    run.ready_since = None;
    // On to the next screen this library has.
    loop {
        run.next += 1;
        let Some(&next) = Shot::ALL.get(run.next) else {
            println!("screenshots: done");
            let _ = slint::quit_event_loop();
            return;
        };
        if show(next, &run.firsts) {
            return;
        }
        eprintln!("screenshots: nothing to show for {}", next.name());
    }
}

/// Go to the screen for `shot`. False if the library has nothing for it.
fn show(shot: Shot, firsts: &[(String, String)]) -> bool {
    let kind = match shot {
        Shot::Home => {
            enter_home();
            return true;
        }
        Shot::Library => {
            let Some(i) = with_models(|m| m.libraries.iter().position(|l| l.kind == "movie")) else { return false };
            select_library(i as i32);
            return true;
        }
        Shot::Movie => "movie",
        Shot::Show => "show",
        Shot::Album => "artist",
    };
    let Some((_, id)) = firsts.iter().find(|(k, _)| k == kind) else { return false };
    open_item(id.clone());
    true
}

/// Whether the screen for `shot` has its content and every image it asked for.
fn ready(shot: Shot) -> bool {
    let ui = ui();
    if ui.get_loading() || !with_state(|s| s.pending_images.is_empty()) {
        return false;
    }
    with_models(|m| match shot {
        Shot::Home => ui.get_screen() == "home" && m.home_rows.row_count() > 0,
        Shot::Library => ui.get_screen() == "home" && m.grid.row_count() > 0,
        Shot::Movie => ui.get_screen() == "detail" && m.people.row_count() > 0,
        Shot::Show => ui.get_screen() == "detail" && m.episodes.row_count() > 0,
        Shot::Album => ui.get_screen() == "detail" && m.tracks.row_count() > 0,
    })
}

/// (library kind, id of the first title) for each home row that belongs to a library.
fn first_recent_titles() -> Vec<(String, String)> {
    with_models(|m| {
        m.home_rows
            .iter()
            .filter(|r| r.library >= 0)
            .filter_map(|r| {
                let kind = m.libraries.row_data(r.library as usize)?.kind.to_string();
                Some((kind, r.items.row_data(0)?.id.to_string()))
            })
            .collect()
    })
}

fn save(dir: &std::path::Path, shot: Shot) {
    let path = dir.join(format!("{}.png", shot.name()));
    let result = ui()
        .window()
        .take_snapshot()
        .map_err(|e| e.to_string())
        .and_then(|buf| {
            image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec()).ok_or_else(|| "bad snapshot".to_string())
        })
        .and_then(|img| img.save(&path).map_err(|e| e.to_string()));
    match result {
        Ok(()) => println!("screenshots: {}", path.display()),
        Err(e) => eprintln!("screenshots: couldn't save {}: {e}", path.display()),
    }
}
