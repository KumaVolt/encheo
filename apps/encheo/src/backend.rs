//! Media servers behind one face: Plex, and Jellyfin/Emby. Items say which server they belong to
//! through their id (Jellyfin/Emby ids are prefixed `jf<n>:`; Plex ids are bare).

use super::*;
use encheo_jellyfin as jf;

#[derive(Clone)]
pub(crate) enum Backend {
    Plex(Server),
    Jf(jf::Server),
}

/// Prefix of the n-th Jellyfin/Emby server's ids.
pub(crate) fn jf_prefix(n: usize) -> String {
    format!("jf{n}:")
}

fn jf_index(id: &str) -> Option<usize> {
    id.strip_prefix("jf")?.split_once(':')?.0.parse().ok()
}

/// The server an item (or library key) belongs to.
pub(crate) fn backend_for(id: &str) -> Option<Backend> {
    match jf_index(id) {
        Some(n) => with_state(|s| s.jf_servers.get(n).cloned()).map(Backend::Jf),
        None => server().map(Backend::Plex),
    }
}

/// Every connected server, Plex first.
pub(crate) fn all_backends() -> Vec<Backend> {
    let mut v: Vec<Backend> = server().into_iter().map(Backend::Plex).collect();
    v.extend(with_state(|s| s.jf_servers.clone()).into_iter().map(Backend::Jf));
    v
}

pub(crate) fn jf_client(session: &Session) -> jf::ClientInfo {
    jf::ClientInfo {
        device_id: session.client_id.clone(),
        device_name: format!("Encheo ({})", std::env::consts::OS),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// Build the Jellyfin/Emby servers from the saved logins.
pub(crate) fn load_jf_servers() {
    with_state(|s| {
        let client = jf_client(&s.session);
        s.jf_servers =
            s.session.media_servers.iter().enumerate().map(|(i, l)| jf::Server::new(l.clone(), client.clone(), jf_prefix(i))).collect();
    });
}

type R<T> = Result<T, String>;

impl Backend {
    /// Shown above its libraries in the sidebar.
    pub fn label(&self) -> String {
        match self {
            Backend::Plex(s) => s.name().to_string(),
            Backend::Jf(s) => s.name().to_string(),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Backend::Plex(_) => "Plex",
            Backend::Jf(s) if s.login().flavor == jf::Flavor::Emby => "Emby",
            Backend::Jf(_) => "Jellyfin",
        }
    }

    pub async fn libraries(&self) -> R<Vec<encheo_plex::Library>> {
        match self {
            Backend::Plex(s) => s.libraries().await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.libraries().await.map_err(|e| e.to_string()),
        }
    }

    pub async fn library_items(&self, key: &str, kind: &str, start: u32, size: u32) -> R<encheo_plex::Page> {
        match self {
            Backend::Plex(s) => s.library_items(key, start, size).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.library_items(key, kind, start, size).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn recently_added(&self, key: &str, size: u32) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.recently_added(key, size).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.recently_added(key, size).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn on_deck(&self) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.on_deck().await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.on_deck().await.map_err(|e| e.to_string()),
        }
    }

    pub async fn metadata(&self, id: &str) -> R<Option<Item>> {
        match self {
            Backend::Plex(s) => s.metadata(id).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.metadata(id).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn children(&self, id: &str) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.children(id).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.children(id).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn all_episodes(&self, show: &str) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.all_episodes(show).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.all_episodes(show).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn related(&self, id: &str) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.related(id).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.related(id).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn search(&self, q: &str) -> R<Vec<Item>> {
        match self {
            Backend::Plex(s) => s.search(q).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.search(q).await.map_err(|e| e.to_string()),
        }
    }

    pub fn stream_url(&self, item: &Item) -> Option<String> {
        match self {
            Backend::Plex(s) => item.primary_part().map(|p| s.stream_url(p)),
            Backend::Jf(s) => Some(s.stream_url(item)),
        }
    }

    pub fn subtitle_url(&self, stream: &encheo_plex::Stream) -> Option<String> {
        match self {
            Backend::Plex(s) => s.subtitle_url(stream),
            Backend::Jf(s) => s.subtitle_url(stream),
        }
    }

    pub async fn report_start(&self, item: &Item) {
        if let Backend::Jf(s) = self {
            let _ = s.report_start(item).await;
        }
    }

    pub async fn report_progress(&self, item: &Item, state: PlayState, position_ms: u64) -> R<()> {
        match self {
            Backend::Plex(s) => s.report_progress(item, state, position_ms).await.map_err(|e| e.to_string()),
            Backend::Jf(s) => s.report_progress(item, state, position_ms).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn set_watched(&self, id: &str, watched: bool) -> R<()> {
        match (self, watched) {
            (Backend::Plex(s), true) => s.mark_watched(id).await.map_err(|e| e.to_string()),
            (Backend::Plex(s), false) => s.mark_unwatched(id).await.map_err(|e| e.to_string()),
            (Backend::Jf(s), true) => s.mark_watched(id).await.map_err(|e| e.to_string()),
            (Backend::Jf(s), false) => s.mark_unwatched(id).await.map_err(|e| e.to_string()),
        }
    }

    /// Plex only: a show's per-show settings (language preferences).
    pub async fn preferences(&self, show: &str) -> Vec<encheo_plex::Setting> {
        match self {
            Backend::Plex(s) => s.preferences(show).await.unwrap_or_default(),
            Backend::Jf(_) => vec![],
        }
    }

    /// Plex only; Jellyfin/Emby choices are kept locally.
    pub async fn set_preferences(&self, show: &str, values: &[(&str, &str)]) -> R<()> {
        match self {
            Backend::Plex(s) => s.set_preferences(show, values).await.map_err(|e| e.to_string()),
            Backend::Jf(_) => Ok(()),
        }
    }

    /// Plex only: remember the audio/subtitle track for a file.
    pub async fn set_streams(&self, part: u64, audio: Option<u64>, subtitle: Option<u64>) -> R<()> {
        match self {
            Backend::Plex(s) => s.set_streams(part, audio, subtitle).await.map_err(|e| e.to_string()),
            Backend::Jf(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prefixes() {
        assert_eq!(super::jf_index("jf2:abc"), Some(2));
        assert_eq!(super::jf_index("12345"), None);
        assert_eq!(super::jf_index("share-file:0:x"), None);
    }
}
