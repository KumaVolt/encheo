//! FTP and FTPS (explicit TLS). Seeking restarts the transfer at the new offset (REST), on a
//! connection of the file's own, so listing and playing don't get in each other's way.

use crate::{Config, Entry, Error, ReadSeek, Result};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;
use suppaftp::RustlsFtpStream as FtpStream;
use suppaftp::types::FileType;

#[derive(Clone)]
struct Login {
    addr: String,
    host: String,
    secure: bool,
    user: String,
    password: String,
}

pub struct Ftp {
    login: Login,
    root: String,
    control: Mutex<FtpStream>,
}

/// `ftp://host[:port]/path` or `ftps://…` → (login, root path).
fn parse(config: &Config) -> Result<(Login, String)> {
    let u = url::Url::parse(config.location.trim()).map_err(|e| Error::Connect(format!("not an ftp:// address: {e}")))?;
    let secure = match u.scheme() {
        "ftp" => false,
        "ftps" => true,
        _ => return Err(Error::Connect("FTP addresses start with ftp:// or ftps://".into())),
    };
    let host = u.host_str().ok_or_else(|| Error::Connect("missing server name".into()))?.to_string();
    let addr = format!("{host}:{}", u.port().unwrap_or(21));
    let user = if config.username.is_empty() { "anonymous".into() } else { config.username.clone() };
    let root = crate::webdav_decode(u.path()).trim_matches('/').to_string();
    Ok((Login { addr, host, secure, user, password: config.password.clone() }, root))
}

fn connect(login: &Login) -> Result<FtpStream> {
    let mut ftp = FtpStream::connect(&login.addr).map_err(|e| Error::Connect(e.to_string()))?;
    if login.secure {
        let connector = suppaftp::RustlsConnector::from(std::sync::Arc::new(crate::tls_config()));
        ftp = ftp.into_secure(connector, &login.host).map_err(|e| Error::Connect(format!("TLS: {e}")))?;
    }
    ftp.login(&login.user, &login.password).map_err(|e| {
        if e.to_string().contains("530") { Error::Auth } else { Error::Connect(e.to_string()) }
    })?;
    ftp.transfer_type(FileType::Binary).map_err(|e| Error::Other(e.to_string()))?;
    let _ = ftp.get_ref().set_read_timeout(Some(std::time::Duration::from_secs(30)));
    Ok(ftp)
}

impl Ftp {
    pub fn connect(config: &Config) -> Result<Self> {
        let (login, root) = parse(config)?;
        let control = connect(&login)?;
        Ok(Self { login, root, control: Mutex::new(control) })
    }

    fn full(&self, rel: &str) -> String {
        format!("/{}", crate::join(&self.root, rel.trim_matches('/')))
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let path = self.full(rel);
        let mut ftp = self.control.lock().expect("ftp");
        // A dropped control connection (servers time out idle ones) is reopened once.
        let lines = match ftp.list(Some(&path)) {
            Ok(l) => l,
            Err(_) => {
                *ftp = connect(&self.login)?;
                ftp.list(Some(&path)).map_err(|e| Error::Other(e.to_string()))?
            }
        };
        Ok(lines
            .iter()
            .filter_map(|l| suppaftp::list::File::try_from(l.as_str()).ok())
            .filter(|f| f.name() != "." && f.name() != "..")
            .map(|f| Entry { path: crate::join(rel, f.name()), name: f.name().to_string(), is_dir: f.is_directory(), size: (!f.is_directory()).then(|| f.size() as u64) })
            .collect())
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let path = self.full(rel);
        let mut ftp = connect(&self.login)?;
        let len = ftp.size(&path).ok().map(|s| s as u64);
        Ok(Box::new(FtpFile { login: self.login.clone(), path, conn: Some(ftp), stream: None, pos: 0, len }))
    }
}

struct FtpFile {
    login: Login,
    path: String,
    conn: Option<FtpStream>,
    stream: Option<Box<dyn Read + Send>>,
    pos: u64,
    len: Option<u64>,
}

impl FtpFile {
    fn start(&mut self) -> io::Result<()> {
        // A transfer that was cut short leaves the control connection waiting; start clean.
        let mut ftp = match self.conn.take() {
            Some(c) => c,
            None => connect(&self.login).map_err(io::Error::other)?,
        };
        if self.pos > 0 {
            ftp.resume_transfer(self.pos as usize).map_err(io::Error::other)?;
        }
        let stream = ftp.retr_as_stream(&self.path).map_err(io::Error::other)?;
        self.stream = Some(Box::new(stream));
        // The control connection stays open until the data stream ends; keep it alive with it.
        std::mem::forget(ftp);
        Ok(())
    }
}

impl Read for FtpFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.len.is_some_and(|l| self.pos >= l) {
            return Ok(0);
        }
        if self.stream.is_none() {
            self.start()?;
        }
        let n = self.stream.as_mut().expect("started").read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for FtpFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => self.len.ok_or_else(|| io::Error::other("unknown size"))?.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
        };
        if new != self.pos {
            self.stream = None; // restart at the new offset on the next read
            self.pos = new;
        }
        Ok(self.pos)
    }
}

impl ReadSeek for FtpFile {
    fn len(&self) -> Option<u64> {
        self.len
    }
}
