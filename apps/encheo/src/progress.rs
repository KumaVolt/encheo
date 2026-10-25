//! Watch progress for files from file sources. (Media servers keep their own.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    /// Seconds.
    pub position: f64,
    pub duration: f64,
    pub watched: bool,
    /// Unix seconds, for "Continue Watching" order.
    pub updated: u64,
}

impl Progress {
    /// 0..1 while in progress, `None` when not started or finished.
    pub fn fraction(&self) -> Option<f32> {
        (!self.watched && self.position > 5.0 && self.duration > 0.0).then(|| (self.position / self.duration).min(1.0) as f32)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    items: HashMap<String, Progress>,
}

fn path() -> Option<PathBuf> {
    Some(crate::session::data_dir()?.join("progress.json"))
}

impl Store {
    pub fn load() -> Self {
        path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Progress {
        self.items.get(id).copied().unwrap_or_default()
    }

    /// Record a position. Past 90% counts as watched (and drops the resume point), like Plex.
    pub fn record(&mut self, id: &str, position: f64, duration: f64) {
        let watched = duration > 0.0 && position / duration > 0.9;
        let updated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let p = Progress { position: if watched { 0.0 } else { position }, duration, watched, updated };
        self.items.insert(id.to_string(), p);
        self.save();
    }

    /// In-progress items, most recent first.
    pub fn in_progress(&self) -> Vec<(String, Progress)> {
        let mut v: Vec<_> = self.items.iter().filter(|(_, p)| p.fraction().is_some()).map(|(k, p)| (k.clone(), *p)).collect();
        v.sort_by_key(|(_, p)| std::cmp::Reverse(p.updated));
        v
    }

    fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = std::fs::write(p, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn fractions() {
        assert_eq!(Progress { position: 600.0, duration: 1200.0, ..Default::default() }.fraction(), Some(0.5));
        assert_eq!(Progress { position: 2.0, duration: 1200.0, ..Default::default() }.fraction(), None);
        assert_eq!(Progress { position: 0.0, duration: 1200.0, watched: true, ..Default::default() }.fraction(), None);
    }
}
