//! DLNA / UPnP media servers (NAS boxes, TVs, MiniDLNA, Serviio…). Browsing is SOAP calls to the
//! server's ContentDirectory; files are plain HTTP URLs mpv plays directly.
//!
//! The location is the device description URL (`http://host:port/rootDesc.xml`); discovery
//! finds it. Servers address folders by object id, so the paths we hand out are made of display
//! names and mapped back to ids, filled in as folders are listed.

use crate::{DirectUrl, Entry, Error, ReadSeek, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Dlna {
    http: reqwest::blocking::Client,
    control_url: String,
    /// The host we reached the server on.
    host: String,
    /// path → object id (folders) or media URL (files).
    ids: Mutex<HashMap<String, Node>>,
}

#[derive(Clone)]
enum Node {
    Folder(String),
    File { url: String, size: Option<u64> },
}

impl Dlna {
    pub fn connect(location: &str) -> Result<Self> {
        let http = encheo_plex::with_platform_roots_blocking(reqwest::blocking::Client::builder())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Connect(e.to_string()))?;
        let desc = http.get(location.trim()).send().and_then(|r| r.text()).map_err(|e| Error::Connect(e.to_string()))?;
        let control = content_directory_control(&desc).ok_or_else(|| Error::Connect("not a DLNA media server".into()))?;
        let base = url::Url::parse(location.trim()).map_err(|e| Error::Connect(e.to_string()))?;
        let control_url = base.join(&control).map_err(|e| Error::Connect(e.to_string()))?.to_string();
        let mut ids = HashMap::new();
        ids.insert(String::new(), Node::Folder("0".into()));
        let host = base.host_str().unwrap_or_default().to_string();
        Ok(Self { http, control_url, host, ids: Mutex::new(ids) })
    }

    fn browse(&self, object_id: &str) -> Result<Vec<DidlEntry>> {
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><ObjectID>{}</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>{start}</StartingIndex><RequestedCount>200</RequestedCount><SortCriteria></SortCriteria></u:Browse></s:Body></s:Envelope>"#,
                xml_escape(object_id)
            );
            let resp = self
                .http
                .post(&self.control_url)
                .header("Content-Type", r#"text/xml; charset="utf-8""#)
                .header("SOAPAction", r#""urn:schemas-upnp-org:service:ContentDirectory:1#Browse""#)
                .body(body)
                .send()
                .map_err(|e| Error::Connect(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!("server answered {}", resp.status())));
            }
            let soap = resp.text().map_err(|e| Error::Other(e.to_string()))?;
            let (didl, returned, total) = soap_result(&soap);
            out.extend(parse_didl(&didl));
            start += returned;
            if returned == 0 || start >= total {
                break;
            }
        }
        Ok(out)
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let id = match self.ids.lock().expect("ids").get(rel) {
            Some(Node::Folder(id)) => id.clone(),
            _ => return Err(Error::NotFound(rel.into())),
        };
        let entries = self.browse(&id)?;
        let mut ids = self.ids.lock().expect("ids");
        let mut out = Vec::new();
        for e in entries {
            let name = e.title.replace('/', "∕");
            // File names keep an extension so the player knows what they are.
            let name = match (&e.url, e.ext.as_deref()) {
                (Some(_), Some(ext)) if !name.to_ascii_lowercase().ends_with(&format!(".{ext}")) => format!("{name}.{ext}"),
                _ => name,
            };
            let path = crate::join(rel, &name);
            match e.url.map(|u| same_host(&u, &self.host)) {
                Some(url) => {
                    ids.insert(path.clone(), Node::File { url, size: e.size });
                    out.push(Entry { name, path, is_dir: false, size: e.size });
                }
                None => {
                    ids.insert(path.clone(), Node::Folder(e.id));
                    out.push(Entry { name, path, is_dir: true, size: None });
                }
            }
        }
        Ok(out)
    }

    fn file(&self, rel: &str) -> Result<(String, Option<u64>)> {
        match self.ids.lock().expect("ids").get(rel) {
            Some(Node::File { url, size }) => Ok((url.clone(), *size)),
            _ => Err(Error::NotFound(rel.into())),
        }
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let (url, size) = self.file(rel)?;
        let url = url::Url::parse(&url).map_err(|e| Error::Other(e.to_string()))?;
        Ok(Box::new(crate::webdav::HttpFile::new(self.http.clone(), url, None, size)))
    }

    pub fn direct_url(&self, rel: &str) -> Option<DirectUrl> {
        self.file(rel).ok().map(|(url, _)| DirectUrl { url, headers: vec![] })
    }
}

/// Servers sometimes advertise media on an address of another network interface (or a Docker
/// network). The media is on the same device, so use the address we reached it on.
fn same_host(url: &str, host: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if u.host_str() != Some(host) && !host.is_empty() => {
            if u.set_host(Some(host)).is_ok() { u.to_string() } else { url.to_string() }
        }
        _ => url.to_string(),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The ContentDirectory service's control URL from a device description.
pub(crate) fn content_directory_control(desc: &str) -> Option<String> {
    let mut reader = Reader::from_str(desc);
    reader.config_mut().trim_text(true);
    let (mut in_service, mut is_cd, mut field, mut control) = (false, false, String::new(), None::<String>);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let n = e.local_name().as_ref().to_string();
                if n == "service" {
                    in_service = true;
                    is_cd = false;
                    control = None;
                }
                field = n;
            }
            Ok(Event::Text(t)) if in_service => {
                let text = t.xml10_content().into_owned();
                match field.as_str() {
                    "serviceType" => is_cd = text.contains("ContentDirectory"),
                    "controlURL" => control = Some(text),
                    _ => {}
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == "service" => {
                if is_cd && control.is_some() {
                    return control;
                }
                in_service = false;
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// The DIDL-Lite document and counts inside a Browse response.
pub(crate) fn soap_result(soap: &str) -> (String, usize, usize) {
    let mut reader = Reader::from_str(soap);
    let (mut field, mut didl, mut returned, mut total) = (String::new(), String::new(), 0, 0);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => field = e.local_name().as_ref().to_string(),
            Ok(Event::Text(t)) => {
                let text = t.xml10_content().into_owned();
                match field.as_str() {
                    "Result" => didl.push_str(&text),
                    "NumberReturned" => returned = text.trim().parse().unwrap_or(0),
                    "TotalMatches" => total = text.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
            Ok(Event::GeneralRef(r)) if field == "Result" => {
                // Escaped DIDL inside <Result>: put the entities back.
                let name = r.xml10_content().into_owned();
                didl.push_str(match name.as_str() {
                    "lt" => "<",
                    "gt" => ">",
                    "amp" => "&",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            Ok(Event::End(_)) => field.clear(),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (didl, returned, total)
}

#[derive(Debug, PartialEq)]
pub(crate) struct DidlEntry {
    id: String,
    title: String,
    /// Files: the first resource URL.
    url: Option<String>,
    size: Option<u64>,
    ext: Option<String>,
}

pub(crate) fn parse_didl(didl: &str) -> Vec<DidlEntry> {
    // Not trimming: spaces next to entities ("Heat &amp; Dust") are part of the title.
    let mut reader = Reader::from_str(didl);
    let mut out = Vec::new();
    let mut cur: Option<DidlEntry> = None;
    let mut field = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                let attr = |k: &str| e.attributes().flatten().find(|a| a.key.local_name().as_ref() == k).map(|a| a.normalized_value(Default::default()).map(|v| v.into_owned()).unwrap_or_default());
                match name.as_str() {
                    "container" | "item" => {
                        cur = Some(DidlEntry { id: attr("id").unwrap_or_default(), title: String::new(), url: None, size: None, ext: None });
                    }
                    "res" => {
                        if let Some(c) = cur.as_mut().filter(|c| c.url.is_none()) {
                            c.size = attr("size").and_then(|s| s.parse().ok());
                            // "http-get:*:video/x-matroska:*" → a hint for the extension.
                            c.ext = attr("protocolInfo").and_then(|p| p.split(':').nth(2).map(mime_ext)).flatten().map(String::from);
                        }
                    }
                    _ => {}
                }
                field = name;
            }
            Ok(Event::Text(t)) => {
                let text = t.xml10_content().into_owned();
                if let Some(c) = cur.as_mut() {
                    match field.as_str() {
                        "title" => c.title.push_str(&text),
                        "res" => match c.url.as_mut() {
                            Some(u) if !u.starts_with("http") || u.ends_with('&') => u.push_str(&text),
                            Some(_) => {}
                            None => c.url = Some(text),
                        },
                        _ => {}
                    }
                }
            }
            Ok(Event::GeneralRef(r)) => {
                // Entities inside text ("Heat &amp; Dust") arrive separately.
                let ch = match r.xml10_content().as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                };
                if let Some(c) = cur.as_mut() {
                    match field.as_str() {
                        "title" => c.title.push_str(ch),
                        "res" => {
                            if let Some(u) = c.url.as_mut() {
                                u.push_str(ch);
                            } else {
                                c.url = Some(ch.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "container" || name == "item" {
                    out.extend(cur.take().map(|mut c| {
                        c.title = c.title.trim().to_string();
                        c.url = c.url.map(|u| u.trim().to_string());
                        c
                    }));
                }
                field.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn mime_ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "video/x-matroska" | "video/x-mkv" => "mkv",
        "video/mp4" => "mp4",
        "video/x-msvideo" | "video/avi" => "avi",
        "video/quicktime" => "mov",
        "video/mpeg" => "mpg",
        "video/vnd.dlna.mpeg-tts" | "video/mp2t" => "ts",
        "video/webm" => "webm",
        "video/x-ms-wmv" => "wmv",
        "audio/mpeg" => "mp3",
        "audio/x-flac" | "audio/flac" => "flac",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_urls_use_the_reachable_host() {
        assert_eq!(same_host("http://172.17.0.8:8200/MediaItems/24.mkv", "192.168.1.5"), "http://192.168.1.5:8200/MediaItems/24.mkv");
        assert_eq!(same_host("http://nas:8200/a.mkv", "nas"), "http://nas:8200/a.mkv");
    }

    #[test]
    fn finds_content_directory() {
        let desc = r#"<root><device><serviceList>
            <service><serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType><controlURL>/ctl/ConnectionMgr</controlURL></service>
            <service><serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType><controlURL>/ctl/ContentDir</controlURL></service>
        </serviceList></device></root>"#;
        assert_eq!(content_directory_control(desc).as_deref(), Some("/ctl/ContentDir"));
    }

    #[test]
    fn parses_browse_results() {
        let soap = r#"<s:Envelope xmlns:s="x"><s:Body><u:BrowseResponse xmlns:u="y"><Result>&lt;DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="u"&gt;&lt;container id="64$0" parentID="64" childCount="1"&gt;&lt;dc:title&gt;Movies&lt;/dc:title&gt;&lt;/container&gt;&lt;item id="64$1" parentID="64"&gt;&lt;dc:title&gt;Heat &amp;amp; Dust&lt;/dc:title&gt;&lt;res size="1234" protocolInfo="http-get:*:video/x-matroska:*"&gt;http://10.0.0.2:8200/MediaItems/22.mkv&lt;/res&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;</Result><NumberReturned>2</NumberReturned><TotalMatches>2</TotalMatches></u:BrowseResponse></s:Body></s:Envelope>"#;
        let (didl, returned, total) = soap_result(soap);
        assert_eq!((returned, total), (2, 2));
        let e = parse_didl(&didl);
        assert_eq!(e[0], DidlEntry { id: "64$0".into(), title: "Movies".into(), url: None, size: None, ext: None });
        assert_eq!(e[1].title, "Heat & Dust");
        assert_eq!(e[1].url.as_deref(), Some("http://10.0.0.2:8200/MediaItems/22.mkv"));
        assert_eq!(e[1].size, Some(1234));
        assert_eq!(e[1].ext.as_deref(), Some("mkv"));
    }
}
