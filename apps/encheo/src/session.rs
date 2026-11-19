//! What survives a restart: this install's Plex client id, the account token and the server we
//! last connected to.
//!
//! Stored as JSON in the user's config folder with owner-only permissions. Moving the tokens into
//! the OS keychain is on the list; this is enough to sign in once.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub client_id: String,
    pub token: Option<String>,
    pub server: Option<SavedServer>,
    /// File sources (folders, WebDAV, SMB) the user added.
    #[serde(default)]
    pub sources: Vec<encheo_vfs::Config>,
    /// Jellyfin and Emby servers the user signed in to.
    #[serde(default)]
    pub media_servers: Vec<encheo_jellyfin::Login>,
    /// Stable numbers for `sources` and `media_servers`, in the same order. Item ids, saved
    /// progress and favourites carry these, so removing a source doesn't shift another's ids.
    /// Sessions from before they existed get the positions, which is what ids used then.
    #[serde(default)]
    pub source_ids: Vec<usize>,
    #[serde(default)]
    pub media_server_ids: Vec<usize>,
    /// The next number to hand out; numbers are never reused.
    #[serde(default)]
    pub next_id: usize,
    /// The user chose to use Encheo without Plex.
    #[serde(default)]
    pub skip_plex: bool,
    #[serde(default)]
    pub prefs: Prefs,
}

/// Player preferences that carry over between files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub auto_skip: bool,
    #[serde(default = "one")]
    pub sub_scale: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for Prefs {
    fn default() -> Self {
        Self { auto_skip: false, sub_scale: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedServer {
    pub name: String,
    pub client_identifier: String,
    pub base_url: String,
    pub token: String,
}

static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Use this folder for everything Encheo stores (Android: the app's private data folder).
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// Where Encheo keeps its files: the platform config folder (`~/Library/Application Support/Encheo`,
/// `%APPDATA%\Encheo`, `~/.config/Encheo`) unless set with [`set_data_dir`].
pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.get().cloned().or_else(|| Some(dirs::config_dir()?.join("Encheo")))
}

fn path() -> Option<PathBuf> {
    Some(data_dir()?.join("session.json"))
}

impl Session {
    pub fn load() -> Self {
        let mut s: Session = path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if s.client_id.is_empty() {
            s.client_id = uuid::Uuid::new_v4().to_string();
            s.save();
        }
        s.fill_ids();
        s
    }

    fn fill_ids(&mut self) {
        for (ids, len) in [(&mut self.source_ids, self.sources.len()), (&mut self.media_server_ids, self.media_servers.len())] {
            ids.truncate(len);
            while ids.len() < len {
                ids.push(ids.len());
            }
        }
        let highest = self.source_ids.iter().chain(&self.media_server_ids).max().map_or(0, |n| n + 1);
        self.next_id = self.next_id.max(highest);
    }

    fn new_id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id - 1
    }

    /// Add a file source; returns its position.
    pub fn add_source(&mut self, config: encheo_vfs::Config) -> usize {
        let id = self.new_id();
        self.sources.push(config);
        self.source_ids.push(id);
        self.save();
        self.sources.len() - 1
    }

    pub fn remove_source(&mut self, index: usize) {
        if index < self.sources.len() {
            self.sources.remove(index);
            self.source_ids.remove(index);
            self.save();
        }
    }

    /// Add a Jellyfin/Emby login, or put it in place of `replace` (a position) or of an earlier
    /// login to the same server and user, keeping that one's number.
    pub fn add_media_server(&mut self, login: encheo_jellyfin::Login, replace: Option<usize>) {
        let same = replace.filter(|&i| i < self.media_servers.len()).or_else(|| {
            self.media_servers.iter().position(|l| l.base_url == login.base_url && l.user_id == login.user_id)
        });
        match same {
            Some(i) => self.media_servers[i] = login,
            None => {
                let id = self.new_id();
                self.media_servers.push(login);
                self.media_server_ids.push(id);
            }
        }
        self.save();
    }

    pub fn remove_media_server(&mut self, index: usize) {
        if index < self.media_servers.len() {
            self.media_servers.remove(index);
            self.media_server_ids.remove(index);
            self.save();
        }
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_vec_pretty(self) else { return };
        if std::fs::write(&p, json).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    /// Forget the account but keep the client id, so Plex doesn't list a new device next time.
    pub fn sign_out(&mut self) {
        self.token = None;
        self.server = None;
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    fn source(name: &str) -> encheo_vfs::Config {
        encheo_vfs::Config {
            kind: encheo_vfs::Kind::Local,
            name: name.into(),
            location: format!("/{name}"),
            username: String::new(),
            password: String::new(),
            host_key: None,
        }
    }

    #[test]
    fn source_numbers_stay_put() {
        // Saving writes session.json: keep it away from the real one.
        super::set_data_dir(std::env::temp_dir().join("encheo-session-test"));
        // An older session: sources without numbers get their positions.
        let mut s = Session { sources: vec![source("a"), source("b")], ..Default::default() };
        s.fill_ids();
        assert_eq!(s.source_ids, [0, 1]);
        s.remove_source(0);
        assert_eq!(s.source_ids, [1]);
        // A new source never takes a number that was used before.
        s.add_source(source("c"));
        assert_eq!(s.source_ids, [1, 2]);
    }
}
