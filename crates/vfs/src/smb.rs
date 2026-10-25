//! SMB2/3 (Windows shares, NAS boxes) with the pure-Rust `smb2` client. Files are read with
//! positioned reads, which is exactly what seeking in a video needs.

use crate::{Config, Entry, Error, ReadSeek, Result};
use smb2::{SmbClient, Tree};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

/// The client is async; a small runtime of our own lets the blocking API (and mpv's reader
/// threads) drive it.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("smb").build().expect("smb runtime")
    })
}

pub struct Smb {
    client: Mutex<SmbClient>,
    tree: Mutex<Tree>,
    /// Folder inside the share that the source starts at, `\`-free, no slashes at the ends.
    root: String,
}

/// `smb://host[:port]/share[/path]` → (host:port, share, path).
pub(crate) fn parse_location(location: &str) -> Result<(String, String, String)> {
    let u = url::Url::parse(location.trim()).map_err(|e| Error::Connect(format!("not an smb:// address: {e}")))?;
    if u.scheme() != "smb" {
        return Err(Error::Connect("SMB addresses look like smb://server/share".into()));
    }
    let host = u.host_str().ok_or_else(|| Error::Connect("missing server name".into()))?;
    let addr = format!("{host}:{}", u.port().unwrap_or(445));
    let mut parts = u.path_segments().into_iter().flatten().filter(|s| !s.is_empty()).map(|s| {
        url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
    });
    let share = parts.next().ok_or_else(|| Error::Connect("add the share name: smb://server/share".into()))?;
    let path = parts.collect::<Vec<_>>().join("/");
    Ok((addr, share, path))
}

impl Smb {
    pub fn connect(config: &Config) -> Result<Self> {
        let (addr, share, root) = parse_location(&config.location)?;
        let user = if config.username.is_empty() { "guest" } else { config.username.as_str() };
        let (client, tree) = runtime()
            .block_on(async {
                let mut client = smb2::connect(&addr, user, &config.password).await?;
                let tree = client.connect_share(&share).await?;
                Ok::<_, smb2::Error>((client, tree))
            })
            .map_err(map_err)?;
        Ok(Self { client: Mutex::new(client), tree: Mutex::new(tree), root })
    }

    fn full(&self, rel: &str) -> String {
        crate::join(&self.root, rel.trim_matches('/'))
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let path = self.full(rel);
        let mut client = self.client.lock().expect("smb client");
        let mut tree = self.tree.lock().expect("smb tree");
        let entries = runtime().block_on(client.list_directory(&mut tree, &path)).map_err(map_err)?;
        Ok(entries
            .into_iter()
            .filter(|e| e.name != "." && e.name != "..")
            .map(|e| Entry { path: crate::join(rel, &e.name), size: (!e.is_directory).then_some(e.size), is_dir: e.is_directory, name: e.name })
            .collect())
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let path = self.full(rel);
        let mut client = self.client.lock().expect("smb client");
        let tree = self.tree.lock().expect("smb tree");
        let reader = runtime().block_on(client.open_file_reader(&tree, &path)).map_err(map_err)?;
        let len = reader.size();
        Ok(Box::new(SmbFile { reader: Arc::new(reader), pos: 0, len, buf: vec![], buf_start: 0 }))
    }
}

fn map_err(e: smb2::Error) -> Error {
    let text = e.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("logon") || lower.contains("access_denied") || lower.contains("access denied") || lower.contains("auth") {
        Error::Auth
    } else if lower.contains("not_found") || lower.contains("not found") || lower.contains("bad_network_name") {
        Error::NotFound(text)
    } else {
        Error::Connect(text)
    }
}

/// Reads ahead in 4 MiB blocks, so mpv's small reads don't each cost a network round trip.
struct SmbFile {
    reader: Arc<smb2::client::stream::FileReader>,
    pos: u64,
    len: u64,
    buf: Vec<u8>,
    buf_start: u64,
}

const BLOCK: u64 = 4 << 20;

impl Read for SmbFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            let want = BLOCK.min(self.len - self.pos);
            let reader = self.reader.clone();
            let at = self.pos;
            self.buf = runtime().block_on(async move { reader.read_at(at, want).await }).map_err(io::Error::other)?;
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

impl Seek for SmbFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => self.len.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
        };
        Ok(self.pos)
    }
}

impl ReadSeek for SmbFile {
    fn len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_location;

    #[test]
    fn locations() {
        assert_eq!(parse_location("smb://nas/Media").unwrap(), ("nas:445".into(), "Media".into(), "".into()));
        assert_eq!(
            parse_location("smb://192.168.1.5:4450/Media/TV%20Shows/").unwrap(),
            ("192.168.1.5:4450".into(), "Media".into(), "TV Shows".into())
        );
        assert!(parse_location("smb://nas").is_err());
        assert!(parse_location("http://nas/x").is_err());
    }
}
