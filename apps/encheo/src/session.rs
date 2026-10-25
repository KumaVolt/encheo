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
    /// The user chose to use Encheo without Plex.
    #[serde(default)]
    pub skip_plex: bool,
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
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
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
        s
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
