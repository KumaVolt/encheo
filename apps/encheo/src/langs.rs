//! Language codes and the cache of detected subtitle languages.
//!
//! Files tag languages with ISO 639-2 codes (`dut`, `fre`, `ger`, often the "bibliographic"
//! ones), Plex profiles use ISO 639-1 (`nl`, `en`), and detection gives ISO 639-3 (`nld`, `cmn`).
//! Everything is compared in the detector's ISO 639-3 form.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Bring any of the code styles above to ISO 639-3 as `whatlang` uses it.
pub fn normalize(code: &str) -> String {
    let c = code.trim().to_ascii_lowercase();
    let c = c.split(['-', '_']).next().unwrap_or("").to_string(); // "pt-BR" -> "pt"
    let mapped = match c.as_str() {
        // ISO 639-1
        "en" => "eng",
        "nl" => "nld",
        "fr" => "fra",
        "de" => "deu",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" => "cmn",
        "ru" => "rus",
        "ar" => "arb",
        "hi" => "hin",
        "sv" => "swe",
        "da" => "dan",
        "no" | "nb" => "nob",
        "fi" => "fin",
        "pl" => "pol",
        "tr" => "tur",
        "cs" => "ces",
        "el" => "ell",
        "ro" => "ron",
        "hu" => "hun",
        "he" => "heb",
        "id" => "ind",
        "th" => "tha",
        "uk" => "ukr",
        "vi" => "vie",
        "ms" => "zsm",
        // ISO 639-2/B and other variants
        "dut" => "nld",
        "fre" => "fra",
        "ger" => "deu",
        "chi" | "zho" => "cmn",
        "cze" => "ces",
        "gre" => "ell",
        "rum" => "ron",
        "may" | "msa" => "zsm",
        "ara" => "arb",
        "nor" => "nob",
        other => other,
    };
    mapped.to_string()
}

/// A detected language, as stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Detected {
    pub code: String,
    pub name: String,
}

/// Detected languages per Plex part (file), per stream index. Scanning costs a few seconds and
/// some megabytes, so each file is scanned once.
#[derive(Default, Serialize, Deserialize)]
pub struct Cache {
    parts: HashMap<u64, HashMap<i64, Detected>>,
}

fn path() -> Option<PathBuf> {
    Some(crate::session::data_dir()?.join("detected-languages.json"))
}

impl Cache {
    pub fn load() -> Self {
        path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn get(&self, part: u64) -> Option<&HashMap<i64, Detected>> {
        self.parts.get(&part)
    }

    pub fn insert(&mut self, part: u64, found: HashMap<i64, Detected>) {
        self.parts.insert(part, found);
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec(self) {
                let _ = std::fs::write(p, json);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn codes_meet_in_the_middle() {
        assert_eq!(normalize("en"), "eng");
        assert_eq!(normalize("eng"), "eng");
        assert_eq!(normalize("dut"), normalize("nl"));
        assert_eq!(normalize("ger"), "deu");
        assert_eq!(normalize("pt-BR"), "por");
        assert_eq!(normalize("chi"), "cmn");
    }
}

/// What to pick for a show, by language rather than track number, so it carries over to every
/// episode. `None` fields mean "no preference".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowLanguages {
    /// ISO 639-3.
    pub audio: Option<String>,
    pub subtitles: Option<SubtitlePref>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SubtitlePref {
    Off,
    /// ISO 639-3.
    Language(String),
}

impl ShowLanguages {
    /// `self` wins where it has an opinion; `other` fills the gaps.
    pub fn or(self, other: ShowLanguages) -> ShowLanguages {
        ShowLanguages { audio: self.audio.or(other.audio), subtitles: self.subtitles.or(other.subtitles) }
    }

    /// From a show's Plex settings (`audioLanguage`, `subtitleLanguage`, `subtitleMode`).
    pub fn from_plex(settings: &[encheo_plex::Setting]) -> ShowLanguages {
        let get = |id: &str| settings.iter().find(|s| s.id == id).map(|s| s.value.as_str()).filter(|v| !v.is_empty());
        let subtitles = match (get("subtitleMode"), get("subtitleLanguage")) {
            (Some("0"), _) => Some(SubtitlePref::Off),
            (_, Some(l)) => Some(SubtitlePref::Language(normalize(l))),
            _ => None,
        };
        ShowLanguages { audio: get("audioLanguage").map(normalize), subtitles }
    }

    /// The Plex settings that say the same thing, using the values the server offers.
    pub fn to_plex(&self, settings: &[encheo_plex::Setting]) -> Vec<(&'static str, String)> {
        let choice = |id: &str, lang: &str| {
            settings.iter().find(|s| s.id == id)?.choices().find(|c| !c.is_empty() && normalize(c) == lang).map(String::from)
        };
        let mut out = Vec::new();
        if let Some(a) = self.audio.as_deref().and_then(|l| choice("audioLanguage", l)) {
            out.push(("audioLanguage", a));
        }
        match &self.subtitles {
            Some(SubtitlePref::Off) => out.push(("subtitleMode", "0".into())),
            Some(SubtitlePref::Language(l)) => {
                if let Some(v) = choice("subtitleLanguage", l) {
                    out.push(("subtitleLanguage", v));
                    out.push(("subtitleMode", "2".into()));
                }
            }
            None => {}
        }
        out
    }
}

/// Per-show language choices made in Encheo, by "<server id>:<show rating key>". Kept locally as
/// well as in Plex because shared users can't change a show's settings on someone else's server.
#[derive(Default, Serialize, Deserialize)]
pub struct ShowStore {
    shows: HashMap<String, ShowLanguages>,
}

fn show_path() -> Option<PathBuf> {
    Some(crate::session::data_dir()?.join("show-languages.json"))
}

impl ShowStore {
    pub fn load() -> Self {
        show_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn get(&self, key: &str) -> ShowLanguages {
        self.shows.get(key).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, key: String, langs: ShowLanguages) {
        self.shows.insert(key, langs);
        if let Some(p) = show_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec_pretty(self) {
                let _ = std::fs::write(p, json);
            }
        }
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;

    fn setting(id: &str, value: &str, enums: &str) -> encheo_plex::Setting {
        encheo_plex::Setting { id: id.into(), value: value.into(), enum_values: enums.into() }
    }

    const LANGS: &str = ":Account default|ja:日本語|en:English|es-419:Español (Latinoamérica)|nl-NL:Nederlands";

    #[test]
    fn plex_settings_round_trip() {
        let settings = vec![
            setting("audioLanguage", "ja", LANGS),
            setting("subtitleLanguage", "en", LANGS),
            setting("subtitleMode", "2", "-1:Account default|0:Manually selected|2:Always enabled"),
        ];
        let langs = ShowLanguages::from_plex(&settings);
        assert_eq!(langs.audio.as_deref(), Some("jpn"));
        assert_eq!(langs.subtitles, Some(SubtitlePref::Language("eng".into())));

        let want = ShowLanguages { audio: Some("eng".into()), subtitles: Some(SubtitlePref::Language("nld".into())) };
        assert_eq!(
            want.to_plex(&settings),
            [("audioLanguage", "en".to_string()), ("subtitleLanguage", "nl-NL".to_string()), ("subtitleMode", "2".to_string())]
        );
        let off = ShowLanguages { audio: None, subtitles: Some(SubtitlePref::Off) };
        assert_eq!(off.to_plex(&settings), [("subtitleMode", "0".to_string())]);
    }

    #[test]
    fn local_choice_wins_over_plex() {
        let local = ShowLanguages { audio: Some("jpn".into()), subtitles: None };
        let plex = ShowLanguages { audio: Some("eng".into()), subtitles: Some(SubtitlePref::Off) };
        let merged = local.or(plex);
        assert_eq!(merged.audio.as_deref(), Some("jpn"));
        assert_eq!(merged.subtitles, Some(SubtitlePref::Off));
    }
}
