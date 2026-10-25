//! Turning file names into something a person would call the thing: sorting like a human
//! ("Episode 2" before "Episode 10") and reading titles, years and episode numbers out of
//! scene-style names ("The.Expanse.S02E05.1080p.WEB.x264-GRP.mkv").

use std::cmp::Ordering;

/// Compare strings with runs of digits compared as numbers, case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na: String = std::iter::from_fn(|| a.next_if(|c| c.is_ascii_digit())).collect();
                let nb: String = std::iter::from_fn(|| b.next_if(|c| c.is_ascii_digit())).collect();
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// What a file name says about its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Movie { title: String, year: Option<u32> },
    Episode { show: String, season: u32, episode: u32, episode_title: Option<String> },
}

impl Parsed {
    pub fn title(&self) -> &str {
        match self {
            Parsed::Movie { title, .. } => title,
            Parsed::Episode { show, .. } => show,
        }
    }
}

/// Release tags that end a title.
const TAGS: &[&str] = &[
    "2160p", "1080p", "1080i", "720p", "576p", "480p", "4k", "uhd", "hdr", "hdr10", "dv", "dovi", "bluray", "blu-ray",
    "bdrip", "brrip", "webrip", "web-dl", "webdl", "web", "hdtv", "dvdrip", "remux", "x264", "x265", "h264", "h265",
    "hevc", "avc", "aac", "ac3", "eac3", "dts", "truehd", "atmos", "ddp5", "dd5", "proper", "repack", "extended",
    "unrated", "imax", "multi", "10bit", "nf", "amzn", "dsnp", "hmax", "atvp",
];

fn clean_words(s: &str) -> String {
    // Dots and underscores are spaces in scene names, unless the dot is part of an initialism
    // like "S.H.I.E.L.D." - keep it simple and treat all as spaces.
    let s = s.replace(['.', '_'], " ");
    let s = s.trim_matches(|c: char| c == '-' || c == ' ' || c == '[' || c == ']' || c == '(' || c == ')');
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_tag(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-').to_ascii_lowercase();
    TAGS.contains(&w.as_str())
}

fn year_of(word: &str) -> Option<u32> {
    let w = word.trim_matches(|c: char| !c.is_ascii_digit());
    (w.len() == 4).then(|| w.parse::<u32>().ok()).flatten().filter(|y| (1900..=2100).contains(y))
}

/// `S02E05`, `s2e5`, `2x05` (word on its own). Returns (season, episode).
fn episode_marker(word: &str) -> Option<(u32, u32)> {
    let w = word.to_ascii_lowercase();
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    if let Some(rest) = w.strip_prefix('s') {
        let (s, e) = rest.split_once('e')?;
        let e = e.split(|c: char| !c.is_ascii_digit()).next()?; // S01E01E02 → first
        if !s.is_empty() && s.len() <= 3 && !e.is_empty() && e.len() <= 4 && s.chars().all(|c| c.is_ascii_digit()) {
            return Some((s.parse().ok()?, e.parse().ok()?));
        }
    }
    if let Some((s, e)) = w.split_once('x') {
        if (1..=2).contains(&s.len()) && (2..=3).contains(&e.len()) && s.chars().chain(e.chars()).all(|c| c.is_ascii_digit()) {
            return Some((s.parse().ok()?, e.parse().ok()?));
        }
    }
    None
}

/// Read a file name (with or without extension).
pub fn parse(file_name: &str) -> Parsed {
    let stem = match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.len() <= 4 && !ext.contains(' ') && crate::media_kind(file_name).is_some() => stem,
        _ => file_name,
    };
    // Drop bracketed group tags at the start: "[SubsPlease] Show - 01 (1080p)".
    let stem = stem.trim_start();
    let stem = if stem.starts_with('[') { stem.split_once(']').map(|(_, r)| r).unwrap_or(stem) } else { stem };
    let spaced = stem.replace(['.', '_'], " ");
    let words: Vec<&str> = spaced.split_whitespace().collect();

    // Episodes: a marker word splits show and episode title.
    for (i, w) in words.iter().enumerate() {
        if let Some((season, episode)) = episode_marker(w) {
            let show = clean_words(&words[..i].join(" "));
            let rest: Vec<&str> = words[i + 1..].iter().take_while(|w| !is_tag(w) && year_of(w).is_none()).copied().collect();
            let title = clean_words(&rest.join(" "));
            return Parsed::Episode {
                show: strip_year_suffix(&show),
                season,
                episode,
                episode_title: (!title.is_empty()).then_some(title),
            };
        }
    }
    // Anime style: "Show - 07".
    if let Some(pos) = words.iter().position(|w| *w == "-") {
        if let Some(n) = words.get(pos + 1).and_then(|w| w.trim_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok()) {
            if n < 2000 && words[pos + 1].chars().all(|c| c.is_ascii_digit() || c == 'v') {
                return Parsed::Episode { show: clean_words(&words[..pos].join(" ")), season: 1, episode: n, episode_title: None };
            }
        }
    }

    // Movies: title runs until the year or the first release tag. The last year wins, so
    // "2001 A Space Odyssey 1968" keeps "2001" in the title.
    let year_pos = words.iter().rposition(|w| year_of(w).is_some()).filter(|p| *p > 0);
    let tag_pos = words.iter().position(|w| is_tag(w));
    let end = match (year_pos, tag_pos) {
        (Some(y), Some(t)) => y.min(t),
        (Some(y), None) => y,
        (None, Some(t)) => t,
        (None, None) => words.len(),
    };
    let title = clean_words(&words[..end.max(1).min(words.len())].join(" "));
    Parsed::Movie { title, year: year_pos.and_then(|p| year_of(words[p])) }
}

/// "Doctor Who 2005" → "Doctor Who" (shows sometimes carry the year of the remake).
fn strip_year_suffix(show: &str) -> String {
    match show.rsplit_once(' ') {
        Some((head, last)) if year_of(last).is_some() && !head.is_empty() => head.to_string(),
        _ => show.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["Episode 10", "episode 2", "Episode 1", "Episode 02b", "Extras"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Episode 1", "episode 2", "Episode 02b", "Episode 10", "Extras"]);
    }

    fn movie(t: &str, y: Option<u32>) -> Parsed {
        Parsed::Movie { title: t.into(), year: y }
    }

    fn ep(show: &str, s: u32, e: u32, title: Option<&str>) -> Parsed {
        Parsed::Episode { show: show.into(), season: s, episode: e, episode_title: title.map(Into::into) }
    }

    #[test]
    fn movies() {
        assert_eq!(parse("Arrival.2016.2160p.UHD.BluRay.x265-GRP.mkv"), movie("Arrival", Some(2016)));
        assert_eq!(parse("Heat (1995).mkv"), movie("Heat", Some(1995)));
        assert_eq!(parse("Blade Runner 2049 (2017) [1080p].mp4"), movie("Blade Runner 2049", Some(2017)));
        assert_eq!(parse("2001.A.Space.Odyssey.1968.mkv"), movie("2001 A Space Odyssey", Some(1968)));
        assert_eq!(parse("Some_Home_Video.mp4"), movie("Some Home Video", None));
        assert_eq!(parse("The.Matrix.REMASTERED.1080p.mkv").title(), "The Matrix REMASTERED");
    }

    #[test]
    fn episodes() {
        assert_eq!(parse("The.Expanse.S02E05.Home.1080p.WEB.x264-GRP.mkv"), ep("The Expanse", 2, 5, Some("Home")));
        assert_eq!(parse("Doctor Who (2005) - s10e01 - The Pilot.mkv"), ep("Doctor Who", 10, 1, Some("The Pilot")));
        assert_eq!(parse("friends 1x05.avi"), ep("friends", 1, 5, None));
        assert_eq!(parse("[SubsPlease] Frieren - 07 (1080p) [ABCD1234].mkv"), ep("Frieren", 1, 7, None));
        assert_eq!(parse("Show.S01E01E02.mkv"), ep("Show", 1, 1, None));
    }
}
