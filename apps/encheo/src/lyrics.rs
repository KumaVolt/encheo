//! Song lyrics: from the server first (Plex's LyricFind lyrics, Jellyfin's .lrc files), else from
//! LRCLIB (lrclib.net, a free, open lyrics database; no account or key). Only the song's title,
//! artist, album and length are sent to LRCLIB, and only when the server has no synced lyrics.

use encheo_plex::{Item, LyricLine, Lyrics};
use serde_json::Value;

const LRCLIB: &str = "https://lrclib.net/api";
const UA: &str = concat!("Encheo/", env!("CARGO_PKG_VERSION"), " (https://github.com/KumaVolt/encheo)");

/// Lyrics for `item` from LRCLIB: synced when it has them, else plain text.
pub async fn lrclib(item: &Item) -> Option<Lyrics> {
    let artist = item.artist()?;
    let secs = item.duration.map(|d| d as f64 / 1000.0);
    let client = encheo_plex::with_platform_roots(reqwest::Client::builder())
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    // The exact song first (LRCLIB matches the length within a couple of seconds)…
    let mut q: Vec<(&str, String)> = vec![("artist_name", artist.to_string()), ("track_name", item.title.clone())];
    if let Some(album) = &item.parent_title {
        q.push(("album_name", album.clone()));
    }
    if let Some(s) = secs {
        q.push(("duration", format!("{}", s.round())));
    }
    if let Ok(r) = client.get(format!("{LRCLIB}/get")).query(&q).send().await
        && r.status().is_success()
        && let Some(l) = r.json::<Value>().await.ok().as_ref().and_then(from_record)
    {
        return Some(l);
    }
    // …else a search, which forgives a different album name or spelling.
    let q = [("artist_name", artist.to_string()), ("track_name", item.title.clone())];
    let r = client.get(format!("{LRCLIB}/search")).query(&q).send().await.ok()?;
    let found: Vec<Value> = r.json().await.ok()?;
    let close = |v: &&Value| match (secs, v["duration"].as_f64()) {
        (Some(want), Some(got)) => (want - got).abs() <= 3.0,
        _ => true,
    };
    let synced = found.iter().filter(close).find(|v| v["syncedLyrics"].is_string());
    synced.or_else(|| found.iter().find(close)).and_then(from_record)
}

/// One LRCLIB record: `syncedLyrics` (LRC) and/or `plainLyrics`.
fn from_record(v: &Value) -> Option<Lyrics> {
    if v["instrumental"].as_bool() == Some(true) {
        return None;
    }
    if let Some(l) = v["syncedLyrics"].as_str().map(parse_lrc).filter(|l| !l.lines.is_empty()) {
        return Some(l);
    }
    let plain = v["plainLyrics"].as_str()?;
    let lines: Vec<LyricLine> = plain.lines().map(|t| LyricLine { start: 0.0, text: t.trim().to_string() }).collect();
    (!lines.iter().all(|l| l.text.is_empty())).then_some(Lyrics { lines, synced: false, credit: None })
}

/// LRC: `[mm:ss.xx]words` lines. A line can carry several times (a chorus sung twice); tags such as
/// `[ar:Artist]` are skipped, and `[offset:+500]` (ms) shifts every line. Lines without a time are
/// dropped once any line has one.
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut offset = 0.0;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            let (inside, after) = tag;
            if let Some(t) = timestamp(inside) {
                times.push(t);
            } else if let Some(ms) = inside.strip_prefix("offset:").and_then(|v| v.trim().parse::<f64>().ok()) {
                offset = ms / 1000.0;
            }
            rest = after;
        }
        // Enhanced LRC marks each word: `<00:12.34>word`. Keep the words.
        let text = strip_word_times(rest.trim());
        for t in times {
            lines.push(LyricLine { start: t, text: text.clone() });
        }
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    // A positive offset means the lyrics come earlier.
    for l in &mut lines {
        l.start = (l.start - offset).max(0.0);
    }
    Lyrics { synced: !lines.is_empty(), lines, credit: None }
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss:xx` in seconds.
fn timestamp(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let rest = rest.replacen(':', ".", 1);
    let sec: f64 = rest.trim().parse().ok()?;
    (sec < 60.0).then_some(m as f64 * 60.0 + sec)
}

fn strip_word_times(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) if timestamp(&rest[i + 1..i + j]).is_some() => rest = &rest[i + j + 1..],
            _ => {
                out.push('<');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line being sung at `t` seconds: the last that has started. None before the first.
pub fn current(lines: &[LyricLine], t: f64) -> Option<usize> {
    // Shown a touch early, so the eye arrives as the voice does.
    let t = t + 0.25;
    let n = lines.partition_point(|l| l.start <= t);
    n.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc() {
        let l = parse_lrc(
            "[ar:Someone]\n[ti:Song]\n[00:04.17]I woke up\n[00:07.10][01:30.00]Chorus\n\n[00:10]\n[00:12.5]<00:12.50>Word <00:13.00>by word\n",
        );
        assert!(l.synced);
        let got: Vec<(f64, &str)> = l.lines.iter().map(|l| (l.start, l.text.as_str())).collect();
        assert_eq!(
            got,
            [(4.17, "I woke up"), (7.1, "Chorus"), (10.0, ""), (12.5, "Word by word"), (90.0, "Chorus")]
        );
        let shifted = parse_lrc("[offset:+500]\n[00:01.00]a\n[00:00.20]b");
        assert_eq!(shifted.lines.iter().map(|l| l.start).collect::<Vec<_>>(), [0.0, 0.5]);
        assert!(!parse_lrc("just words\nno times").synced);
        assert_eq!(strip_word_times("a <b> c"), "a <b> c", "angle brackets that aren't times stay");
    }

    #[test]
    fn current_line() {
        let l = parse_lrc("[00:05.00]a\n[00:10.00]b\n[00:20.00]c").lines;
        assert_eq!(current(&l, 0.0), None);
        assert_eq!(current(&l, 4.9), Some(0), "a quarter second early");
        assert_eq!(current(&l, 12.0), Some(1));
        assert_eq!(current(&l, 300.0), Some(2));
    }

    #[test]
    fn lrclib_record() {
        let v = serde_json::json!({"instrumental": false, "plainLyrics": "One\nTwo", "syncedLyrics": "[00:01.00]One\n[00:02.00]Two"});
        assert!(from_record(&v).unwrap().synced);
        let v = serde_json::json!({"plainLyrics": "One\nTwo", "syncedLyrics": null});
        let l = from_record(&v).unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 2);
        assert!(from_record(&serde_json::json!({"instrumental": true})).is_none());
    }
}
