//! WebDAV: folders with PROPFIND, files with ranged GETs. mpv plays files straight from the URL
//! (with a Basic auth header), which beats streaming through us.

use crate::{Config, DirectUrl, Entry, Error, ReadSeek, Result};
use base64::Engine;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::io::{self, Read, Seek, SeekFrom};

pub struct WebDav {
    http: reqwest::blocking::Client,
    /// Ends with '/'.
    base: url::Url,
    auth: Option<String>,
}

impl WebDav {
    pub fn new(config: &Config) -> Result<Self> {
        let mut base = url::Url::parse(config.location.trim())
            .map_err(|e| Error::Connect(format!("not a URL: {e}")))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(Error::Connect("WebDAV addresses start with http:// or https://".into()));
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let auth = (!config.username.is_empty()).then(|| {
            let raw = format!("{}:{}", config.username, config.password);
            format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))
        });
        let builder = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30));
        let http = encheo_plex::with_platform_roots_blocking(builder).build().map_err(|e| Error::Connect(e.to_string()))?;
        Ok(Self { http, base, auth })
    }

    fn url(&self, rel: &str) -> url::Url {
        let mut u = self.base.clone();
        {
            let mut segs = u.path_segments_mut().expect("http url");
            segs.pop_if_empty();
            for s in rel.split('/').filter(|s| !s.is_empty()) {
                segs.push(s);
            }
        }
        u
    }

    fn request(&self, method: reqwest::Method, url: url::Url) -> reqwest::blocking::RequestBuilder {
        let r = self.http.request(method, url);
        match &self.auth {
            Some(a) => r.header("Authorization", a),
            None => r,
        }
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let mut dir = self.url(rel);
        if !dir.path().ends_with('/') {
            dir.set_path(&format!("{}/", dir.path()));
        }
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"#;
        let resp = self
            .request(reqwest::Method::from_bytes(b"PROPFIND").expect("method"), dir.clone())
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .map_err(|e| Error::Connect(e.to_string()))?;
        match resp.status().as_u16() {
            401 | 403 => return Err(Error::Auth),
            404 => return Err(Error::NotFound(rel.into())),
            s if !(200..300).contains(&s) => return Err(Error::Other(format!("server answered {s}"))),
            _ => {}
        }
        let xml = resp.text().map_err(|e| Error::Other(e.to_string()))?;
        Ok(parse_propfind(&xml, &dir, rel))
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let url = self.url(rel);
        // Size up front, so seeking from the end works before the first read.
        let resp = self.request(reqwest::Method::HEAD, url.clone()).send().map_err(|e| Error::Connect(e.to_string()))?;
        match resp.status().as_u16() {
            401 | 403 => return Err(Error::Auth),
            404 => return Err(Error::NotFound(rel.into())),
            _ => {}
        }
        let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        Ok(Box::new(HttpFile { dav: self.http.clone(), url, auth: self.auth.clone(), pos: 0, len, buf: vec![], buf_start: 0 }))
    }

    pub fn direct_url(&self, rel: &str) -> DirectUrl {
        DirectUrl {
            url: self.url(rel).to_string(),
            headers: self.auth.iter().map(|a| format!("Authorization: {a}")).collect(),
        }
    }
}

/// Entries of a Depth:1 PROPFIND, minus the folder itself.
fn parse_propfind(xml: &str, dir: &url::Url, rel: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let (mut href, mut is_dir, mut size) = (String::new(), false, None);
    let mut in_href = false;
    let mut in_len = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                "response" => {
                    href.clear();
                    is_dir = false;
                    size = None;
                }
                "href" => in_href = true,
                "getcontentlength" => in_len = true,
                "collection" => is_dir = true,
                _ => {}
            },
            Ok(Event::Empty(e)) if e.local_name().as_ref() == "collection" => is_dir = true,
            Ok(Event::Text(t)) => {
                let text = t.xml10_content().into_owned();
                if in_href {
                    href.push_str(&text);
                } else if in_len {
                    size = text.trim().parse().ok();
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "href" => in_href = false,
                "getcontentlength" => in_len = false,
                "response" => {
                    let target = dir.join(&href).ok();
                    // Servers differ in what they percent-encode ("(" vs "%28"), so compare decoded.
                    let own = target.as_ref().is_some_and(|t| {
                        percent_decode(t.path().trim_end_matches('/')) == percent_decode(dir.path().trim_end_matches('/'))
                    });
                    if let (Some(t), false) = (target, own) {
                        let name = t
                            .path()
                            .trim_end_matches('/')
                            .rsplit('/')
                            .next()
                            .map(|n| percent_decode(n))
                            .unwrap_or_default();
                        if !name.is_empty() {
                            out.push(Entry { path: crate::join(rel, &name), name, is_dir, size: if is_dir { None } else { size } });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Ranged GETs in 1 MiB chunks.
struct HttpFile {
    dav: reqwest::blocking::Client,
    url: url::Url,
    auth: Option<String>,
    pos: u64,
    len: Option<u64>,
    buf: Vec<u8>,
    buf_start: u64,
}

impl Read for HttpFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            if self.len.is_some_and(|l| self.pos >= l) {
                return Ok(0);
            }
            let mut r = self.dav.get(self.url.clone()).header("Range", format!("bytes={}-{}", self.pos, self.pos + (1 << 20) - 1));
            if let Some(a) = &self.auth {
                r = r.header("Authorization", a);
            }
            let resp = r.send().map_err(io::Error::other)?;
            if !resp.status().is_success() {
                return Err(io::Error::other(format!("HTTP {}", resp.status())));
            }
            if self.len.is_none() {
                self.len = resp
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|v| v.parse().ok());
            }
            self.buf = resp.bytes().map_err(io::Error::other)?.to_vec();
            self.buf_start = self.pos;
            if self.buf.is_empty() {
                return Ok(0);
            }
        }
        let off = (self.pos - self.buf_start) as usize;
        let n = out.len().min(self.buf.len() - off);
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => {
                let len = self.len.ok_or_else(|| io::Error::other("unknown length"))?;
                len.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?
            }
        };
        Ok(self.pos)
    }
}

impl ReadSeek for HttpFile {
    fn len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_propfind() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
 <D:response><D:href>/media/Movies/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
 <D:response><D:href>/media/Movies/Arrival%20(2016)/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
 <D:response><D:href>/media/Movies/Heat%20(1995).mkv</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>1234</D:getcontentlength></D:prop></D:propstat></D:response>
</D:multistatus>"#;
        let dir = url::Url::parse("https://nas.local/media/Movies/").unwrap();
        // The folder itself, encoded differently than we would, must not show up as a child.
        let paren = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/a/Big%20Test%20%282024%29/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response></D:multistatus>"#;
        assert!(parse_propfind(paren, &url::Url::parse("https://n/a/Big%20Test%20(2024)/").unwrap(), "x").is_empty());
        let e = parse_propfind(xml, &dir, "Movies");
        assert_eq!(
            e,
            [
                Entry { name: "Arrival (2016)".into(), path: "Movies/Arrival (2016)".into(), is_dir: true, size: None },
                Entry { name: "Heat (1995).mkv".into(), path: "Movies/Heat (1995).mkv".into(), is_dir: false, size: Some(1234) },
            ]
        );
    }

    #[test]
    fn urls_and_auth() {
        let dav = WebDav::new(&Config {
            kind: crate::Kind::WebDav,
            name: "n".into(),
            location: "https://nas.local/dav".into(),
            username: "me".into(),
            password: "pw".into(),
            host_key: None,
        })
        .unwrap();
        let d = dav.direct_url("Movies/Heat (1995).mkv");
        assert_eq!(d.url, "https://nas.local/dav/Movies/Heat%20(1995).mkv");
        assert_eq!(d.headers, ["Authorization: Basic bWU6cHc="]);
    }
}
