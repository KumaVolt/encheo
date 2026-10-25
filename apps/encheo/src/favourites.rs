//! Favourites, kept on this device (Plex has no favourites; Jellyfin's would cover only Jellyfin).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Enough to show a poster on the home screen without asking the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Favourite {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    /// The server's image path (resolved to a URL when shown).
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub square: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    items: Vec<Favourite>,
}

fn path() -> Option<PathBuf> {
    Some(crate::session::data_dir()?.join("favourites.json"))
}

impl Store {
    pub fn load() -> Self {
        path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|f| f.id == id)
    }

    /// Newest first.
    pub fn all(&self) -> &[Favourite] {
        &self.items
    }

    /// Add or remove; returns whether it's a favourite now.
    pub fn toggle(&mut self, fav: Favourite) -> bool {
        let now = if let Some(i) = self.items.iter().position(|f| f.id == fav.id) {
            self.items.remove(i);
            false
        } else {
            self.items.insert(0, fav);
            true
        };
        self.save();
        now
    }

    fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(p, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(id: &str) -> Favourite {
        Favourite { id: id.into(), title: id.into(), subtitle: String::new(), thumb: None, square: false }
    }

    #[test]
    fn toggles_newest_first() {
        let mut s = Store::default();
        s.items.insert(0, fav("a"));
        s.items.insert(0, fav("b"));
        assert_eq!(s.all().iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(s.contains("a"));
        s.items.retain(|f| f.id != "a");
        assert!(!s.contains("a"));
    }
}
