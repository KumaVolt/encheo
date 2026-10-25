//! Work out the language of subtitle tracks that aren't tagged with one.
//!
//! Matroska files often leave the language out; the spec then says "English", which is what Plex
//! and ffprobe report even when the track is Portuguese. Here we read a stretch of the file,
//! collect the text of every untagged text-subtitle track in one pass, and detect the language
//! from that text (offline, with `whatlang`).
//!
//! ```no_run
//! # fn demo(file: std::fs::File) -> Result<(), encheo_sublang::Error> {
//! let samples = encheo_sublang::untagged_subtitle_text(file, &encheo_sublang::Budget::default())?;
//! for (track_index, text) in samples {
//!     if let Some(lang) = encheo_sublang::detect(&text) {
//!         println!("track {track_index}: {} ({:.0}%)", lang.name, lang.confidence * 100.0);
//!     }
//! }
//! # Ok(()) }
//! ```

mod http;

pub use http::HttpReader;
use matroska_demuxer::{Frame, MatroskaFile, TrackType};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a Matroska file or unreadable: {0}")]
    Demux(String),
}

/// How much to read before giving up.
#[derive(Clone, Debug)]
pub struct Budget {
    /// Where to start, as a fraction of the duration. Openings are often music or silence, a
    /// quarter in there's usually dialogue.
    pub start_fraction: f64,
    /// Stop once every track has this much text.
    pub chars_per_track: usize,
    /// Stop after reading this many bytes of the file (video included).
    pub max_bytes: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self { start_fraction: 0.25, chars_per_track: 500, max_bytes: 40 * 1024 * 1024 }
    }
}

/// Text subtitle tracks with no language (or "und"), keyed by their index among all tracks, which
/// is the stream index ffmpeg, mpv and Plex use. Tracks that yielded no text are left out.
pub fn untagged_subtitle_text<R: Read + Seek>(reader: R, budget: &Budget) -> Result<BTreeMap<usize, String>, Error> {
    let counter = Rc::new(Cell::new(0u64));
    let reader = Counting { inner: reader, read: counter.clone() };
    let mut mkv = MatroskaFile::open(reader).map_err(|e| Error::Demux(e.to_string()))?;

    // Track number (as used in blocks) -> index, for the tracks we care about.
    let wanted: BTreeMap<u64, (usize, bool)> = mkv
        .tracks()
        .iter()
        .enumerate()
        .filter(|(_, t)| t.track_type() == TrackType::Subtitle && is_text_codec(t.codec_id()))
        .filter(|(_, t)| matches!(t.language(), None | Some("und")) && t.language_bcp47().is_none_or(|l| l == "und"))
        .map(|(i, t)| (t.track_number().get(), (i, t.codec_id().contains("SSA") || t.codec_id().contains("ASS"))))
        .collect();
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }

    if let Some(duration) = mkv.info().duration().filter(|d| *d > 0.0) {
        // Duration and seek targets are both in timestamp-scale units.
        let target = (duration * budget.start_fraction) as u64;
        if mkv.seek(target).is_err() {
            mkv.seek(0).map_err(|e| Error::Demux(e.to_string()))?;
        }
    }
    counter.set(0); // headers and seeking don't count against the budget

    let mut texts: BTreeMap<usize, String> = BTreeMap::new();
    let mut frame = Frame::default();
    while counter.get() < budget.max_bytes {
        match mkv.next_frame(&mut frame) {
            Ok(true) => {}
            Ok(false) | Err(_) => break,
        }
        let Some(&(index, ass)) = wanted.get(&frame.track) else { continue };
        let line = clean(&String::from_utf8_lossy(&frame.data), ass);
        if !line.is_empty() {
            let t = texts.entry(index).or_default();
            if t.len() < budget.chars_per_track * 2 {
                t.push_str(&line);
                t.push('\n');
            }
        }
        if texts.len() == wanted.len() && texts.values().all(|t| t.chars().count() >= budget.chars_per_track) {
            break;
        }
    }
    Ok(texts)
}

fn is_text_codec(codec: &str) -> bool {
    matches!(codec, "S_TEXT/UTF8" | "S_TEXT/ASCII" | "S_TEXT/ASS" | "S_TEXT/SSA" | "S_TEXT/WEBVTT" | "S_ASS" | "S_SSA")
}

/// The words of one subtitle event, without markup. Matroska ASS events are
/// `ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text`.
pub fn clean(raw: &str, ass: bool) -> String {
    let text = if ass { raw.splitn(9, ',').nth(8).unwrap_or("") } else { raw };
    let mut out = String::with_capacity(text.len());
    let mut depth_brace = 0;
    let mut in_tag = false;
    for c in text.replace("\\N", " ").replace("\\n", " ").replace("\\h", " ").chars() {
        match c {
            '{' => depth_brace += 1,
            '}' if depth_brace > 0 => depth_brace -= 1,
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if depth_brace > 0 || in_tag => {}
            '\r' | '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A detected language.
#[derive(Clone, Debug, PartialEq)]
pub struct Language {
    /// ISO 639-3, e.g. "por", "eng", "nld".
    pub code: &'static str,
    /// English name, e.g. "Portuguese".
    pub name: &'static str,
    /// 0.0–1.0.
    pub confidence: f64,
}

/// The language of a piece of text, if whatlang is reasonably sure. Short or mixed text returns
/// `None` rather than a guess.
pub fn detect(text: &str) -> Option<Language> {
    if text.chars().filter(|c| c.is_alphabetic()).count() < 40 {
        return None;
    }
    let info = whatlang::detect(text)?;
    (info.confidence() >= 0.5).then(|| Language { code: info.lang().code(), name: info.lang().eng_name(), confidence: info.confidence() })
}

/// Counts bytes read, so the scan can stop after a budget.
struct Counting<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.set(self.read.get() + n as u64);
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_srt_and_ass() {
        assert_eq!(clean("<i>Olá,</i>\r\ntudo bem?", false), "Olá, tudo bem?");
        assert_eq!(clean("0,0,Default,,0,0,0,,{\\i1}Hello{\\i0}\\Nthere, friend", true), "Hello there, friend");
        assert_eq!(clean("3,0,Sign,,0,0,0,,{\\pos(10,10)\\fs40}", true), "");
    }

    #[test]
    fn detects_common_languages() {
        let pt = "As noites são longas no nordeste. Eu não sei o que fazer com a minha vida, mas vou tentar. \
                  Precisamos de ir embora antes que eles cheguem.";
        let en = "The nights are long in the north east. I don't know what to do with my life, but I will try. \
                  We need to leave before they get here.";
        let nl = "De nachten zijn lang in het noordoosten. Ik weet niet wat ik met mijn leven moet doen, maar ik ga het proberen.";
        assert_eq!(detect(pt).unwrap().code, "por");
        assert_eq!(detect(en).unwrap().code, "eng");
        assert_eq!(detect(nl).unwrap().code, "nld");
        assert_eq!(detect("Hi!"), None);
    }
}
