//! SFTP (file access over SSH), pure Rust with russh. The server's host key is pinned on first
//! connect and checked every time after: a changed key is refused, not silently accepted.

use crate::{Config, Entry, Error, ReadSeek, Result, runtime};
use russh::client;
use russh_sftp::client::SftpSession;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub struct Sftp {
    sftp: Arc<SftpSession>,
    root: String,
    /// SHA-256 of the server's host key, to store with the source.
    pub fingerprint: String,
    _session: client::Handle<HostKey>,
}

struct HostKey {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostKey {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let fp = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(Default::default()).to_string(),
            russh::keys::PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(Default::default()).to_string(),
        };
        *self.seen.lock().expect("fp") = Some(fp.clone());
        Ok(self.expected.as_ref().is_none_or(|e| *e == fp))
    }
}

/// `sftp://host[:port]/path` → (addr, root).
fn parse(location: &str) -> Result<(String, String)> {
    let u = url::Url::parse(location.trim()).map_err(|e| Error::Connect(format!("not an sftp:// address: {e}")))?;
    if u.scheme() != "sftp" {
        return Err(Error::Connect("SFTP addresses look like sftp://server/path".into()));
    }
    let host = u.host_str().ok_or_else(|| Error::Connect("missing server name".into()))?;
    Ok((format!("{host}:{}", u.port().unwrap_or(22)), crate::webdav_decode(u.path()).trim_end_matches('/').to_string()))
}

impl Sftp {
    pub fn connect(config: &Config) -> Result<Self> {
        let (addr, root) = parse(&config.location)?;
        let seen = Arc::new(Mutex::new(None));
        let handler = HostKey { expected: config.host_key.clone(), seen: seen.clone() };
        let user = config.username.clone();
        let password = config.password.clone();
        let result = runtime().block_on(async move {
            let cfg = Arc::new(client::Config { inactivity_timeout: None, ..Default::default() });
            let mut session = client::connect(cfg, addr, handler).await?;
            let auth = session.authenticate_password(user, password).await?;
            if !matches!(auth, client::AuthResult::Success) {
                return Ok(Err(Error::Auth));
            }
            let channel = session.channel_open_session().await?;
            channel.request_subsystem(true, "sftp").await?;
            let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| russh::Error::IO(io::Error::other(e.to_string())))?;
            Ok::<_, russh::Error>(Ok((session, sftp)))
        });
        let seen = seen.lock().expect("fp").clone();
        let (session, sftp) = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(e) => {
                if config.host_key.is_some() && seen.is_some() && seen != config.host_key {
                    return Err(Error::Other(
                        "the server's identity key has changed. If you reinstalled the server, remove and re-add the source.".into(),
                    ));
                }
                return Err(Error::Connect(e.to_string()));
            }
        };
        // Start in the given folder, or the user's home folder.
        let root = if root.is_empty() { runtime().block_on(sftp.canonicalize(".")).unwrap_or_else(|_| "/".into()) } else { root };
        Ok(Self { sftp: Arc::new(sftp), root, fingerprint: seen.unwrap_or_default(), _session: session })
    }

    fn full(&self, rel: &str) -> String {
        let rel = rel.trim_matches('/');
        let root = self.root.trim_end_matches('/');
        if rel.is_empty() { if root.is_empty() { "/".into() } else { root.into() } } else { format!("{root}/{rel}") }
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let dir = runtime().block_on(self.sftp.read_dir(self.full(rel))).map_err(map_err)?;
        Ok(dir
            .map(|e| {
                let m = e.metadata();
                let name = e.file_name();
                Entry { path: crate::join(rel, &name), name, is_dir: m.is_dir(), size: (!m.is_dir()).then(|| m.len()) }
            })
            .filter(|e| e.name != "." && e.name != "..")
            .collect())
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let path = self.full(rel);
        let sftp = self.sftp.clone();
        let (file, len) = runtime()
            .block_on(async move {
                let len = sftp.metadata(path.clone()).await.ok().map(|m| m.len());
                Ok::<_, russh_sftp::client::error::Error>((sftp.open(path).await?, len))
            })
            .map_err(map_err)?;
        Ok(Box::new(SftpFile { file, len, pos: 0, buf: vec![], buf_start: 0 }))
    }
}

fn map_err(e: russh_sftp::client::error::Error) -> Error {
    let t = e.to_string();
    if t.to_ascii_lowercase().contains("no such file") { Error::NotFound(t) } else { Error::Other(t) }
}

/// Reads ahead 2 MiB at a time.
struct SftpFile {
    file: russh_sftp::client::fs::File,
    len: Option<u64>,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
}

impl Read for SftpFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            if self.len.is_some_and(|l| self.pos >= l) {
                return Ok(0);
            }
            let pos = self.pos;
            let file = &mut self.file;
            let mut block = vec![0u8; 2 << 20];
            let n = runtime().block_on(async {
                file.seek(SeekFrom::Start(pos)).await?;
                let mut filled = 0;
                while filled < block.len() {
                    let n = file.read(&mut block[filled..]).await?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                Ok::<_, io::Error>(filled)
            })?;
            block.truncate(n);
            self.buf = block;
            self.buf_start = pos;
            if n == 0 {
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

impl Seek for SftpFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => self.len.ok_or_else(|| io::Error::other("unknown size"))?.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
        };
        Ok(self.pos)
    }
}

impl ReadSeek for SftpFile {
    fn len(&self) -> Option<u64> {
        self.len
    }
}
