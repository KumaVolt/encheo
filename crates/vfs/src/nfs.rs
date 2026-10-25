//! NFSv3, pure Rust. `nfs://host/export` mounts through the portmapper (port 111);
//! `nfs://host:port/export` talks to a server serving mount and NFS on that one port.
//!
//! Many NAS boxes only accept clients from privileged ports (< 1024), which apps can't open on
//! Linux and Android. We try one first and fall back; the error then says which NAS setting
//! allows it ("Allow connections from non-privileged ports" on Synology, "insecure" in exports).

use crate::{Config, Entry, Error, ReadSeek, Result, runtime};
use nfs3_client::nfs3_types::nfs3::{self, Nfs3Option, nfs_fh3};
use nfs3_client::nfs3_types::rpc::{auth_unix, opaque_auth};
use nfs3_client::nfs3_types::xdr_codec::Opaque;
use nfs3_client::tokio::{TokioConnector, TokioIo};
use nfs3_client::{Nfs3Connection, Nfs3ConnectionBuilder};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use tokio::sync::Mutex;

type Conn = Nfs3Connection<TokioIo<tokio::net::TcpStream>>;

pub struct Nfs {
    conn: Arc<Mutex<Conn>>,
    root: nfs_fh3,
}

fn parse(location: &str) -> Result<(String, Option<u16>, String)> {
    let u = url::Url::parse(location.trim()).map_err(|e| Error::Connect(format!("not an nfs:// address: {e}")))?;
    if u.scheme() != "nfs" {
        return Err(Error::Connect("NFS addresses look like nfs://server/export".into()));
    }
    let host = u.host_str().ok_or_else(|| Error::Connect("missing server name".into()))?.to_string();
    let export = crate::webdav_decode(u.path());
    let export = if export.len() > 1 { export.trim_end_matches('/').to_string() } else { "/".into() };
    Ok((host, u.port(), export))
}

async fn mount(host: &str, port: Option<u16>, export: &str, privileged: bool) -> std::result::Result<Conn, String> {
    // The same credentials the Windows NFS client sends ("nobody").
    let auth = auth_unix { stamp: 0xaaaa_aaaa, machinename: Opaque::borrowed(b"encheo"), uid: 0xffff_fffe, gid: 0xffff_fffe, gids: vec![] };
    let mut b = Nfs3ConnectionBuilder::new(TokioConnector, host.to_string(), export.to_string())
        .credential(opaque_auth::auth_unix(&auth))
        .connect_from_privileged_port(privileged);
    if let Some(p) = port.filter(|p| *p != 111) {
        b = b.mount_port(p).nfs3_port(p);
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), b.mount()).await.map_err(|_| "timed out".to_string())?.map_err(|e| e.to_string())
}

impl Nfs {
    pub fn connect(config: &Config) -> Result<Self> {
        let (host, port, export) = parse(&config.location)?;
        let conn = runtime().block_on(async {
            match mount(&host, port, &export, true).await {
                Ok(c) => Ok(c),
                Err(_) => mount(&host, port, &export, false).await,
            }
        });
        let conn = conn.map_err(|e| {
            let lower = e.to_ascii_lowercase();
            if lower.contains("perm") || lower.contains("acces") || lower.contains("auth") {
                Error::Connect(format!(
                    "{e}. The server may only accept privileged ports: allow non-privileged ports for this export on the NAS (\"insecure\")."
                ))
            } else {
                Error::Connect(e)
            }
        })?;
        let root = conn.root_nfs_fh3();
        Ok(Self { conn: Arc::new(Mutex::new(conn)), root })
    }

    /// Walk from the export root to `rel`.
    async fn lookup(&self, conn: &mut Conn, rel: &str) -> Result<nfs_fh3> {
        let mut fh = self.root.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            let args = nfs3::LOOKUP3args { what: nfs3::diropargs3 { dir: fh, name: part.as_bytes().into() } };
            fh = match conn.lookup(&args).await.map_err(|e| Error::Other(e.to_string()))? {
                nfs3::Nfs3Result::Ok(ok) => ok.object,
                nfs3::Nfs3Result::Err((status, _)) => return Err(Error::NotFound(format!("{rel}: {status:?}"))),
            };
        }
        Ok(fh)
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        runtime().block_on(async {
            let mut conn = self.conn.lock().await;
            let dir = self.lookup(&mut conn, rel).await?;
            let mut out = Vec::new();
            let (mut cookie, mut verf) = (nfs3::cookie3::default(), nfs3::cookieverf3::default());
            loop {
                let args = nfs3::READDIRPLUS3args { dir: dir.clone(), cookie, cookieverf: verf, dircount: 64 * 1024, maxcount: 256 * 1024 };
                let res = match conn.readdirplus(&args).await.map_err(|e| Error::Other(e.to_string()))? {
                    nfs3::Nfs3Result::Ok(ok) => ok,
                    nfs3::Nfs3Result::Err((status, _)) => return Err(Error::Other(format!("{status:?}"))),
                };
                let entries = res.reply.entries.into_inner();
                for e in &entries {
                    let name = String::from_utf8_lossy(&e.name.0).into_owned();
                    if name == "." || name == ".." {
                        continue;
                    }
                    let (is_dir, size) = match &e.name_attributes {
                        Nfs3Option::Some(a) => (matches!(a.type_, nfs3::ftype3::NF3DIR), a.size),
                        _ => (false, 0),
                    };
                    out.push(Entry { path: crate::join(rel, &name), name, is_dir, size: (!is_dir).then_some(size) });
                }
                if res.reply.eof || entries.is_empty() {
                    break;
                }
                cookie = entries.last().expect("non-empty").cookie;
                verf = res.cookieverf;
            }
            Ok(out)
        })
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let (fh, len) = runtime().block_on(async {
            let mut conn = self.conn.lock().await;
            let fh = self.lookup(&mut conn, rel).await?;
            let len = match conn.getattr(&nfs3::GETATTR3args { object: fh.clone() }).await {
                Ok(nfs3::Nfs3Result::Ok(ok)) => Some(ok.obj_attributes.size),
                _ => None,
            };
            Ok::<_, Error>((fh, len))
        })?;
        Ok(Box::new(NfsFile { conn: self.conn.clone(), fh, len, pos: 0, buf: vec![], buf_start: 0 }))
    }
}

/// Reads ahead 1 MiB at a time (in NFS-sized requests).
struct NfsFile {
    conn: Arc<Mutex<Conn>>,
    fh: nfs_fh3,
    len: Option<u64>,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
}

impl Read for NfsFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            if self.len.is_some_and(|l| self.pos >= l) {
                return Ok(0);
            }
            let (conn, fh, start) = (self.conn.clone(), self.fh.clone(), self.pos);
            let data = runtime().block_on(async move {
                let mut conn = conn.lock().await;
                let mut data = Vec::with_capacity(1 << 20);
                while data.len() < (1 << 20) {
                    let args = nfs3::READ3args { file: fh.clone(), offset: start + data.len() as u64, count: 256 * 1024 };
                    match conn.read(&args).await.map_err(io::Error::other)? {
                        nfs3::Nfs3Result::Ok(ok) => {
                            let eof = ok.eof;
                            data.extend_from_slice(&ok.data.0);
                            if eof || ok.count == 0 {
                                break;
                            }
                        }
                        nfs3::Nfs3Result::Err((status, _)) => return Err(io::Error::other(format!("{status:?}"))),
                    }
                }
                Ok::<_, io::Error>(data)
            })?;
            if data.is_empty() {
                return Ok(0);
            }
            self.buf = data;
            self.buf_start = self.pos;
        }
        let off = (self.pos - self.buf_start) as usize;
        let n = out.len().min(self.buf.len() - off);
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for NfsFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => self.len.ok_or_else(|| io::Error::other("unknown size"))?.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
        };
        Ok(self.pos)
    }
}

impl ReadSeek for NfsFile {
    fn len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn locations() {
        assert_eq!(super::parse("nfs://nas/volume1/video").unwrap(), ("nas".into(), None, "/volume1/video".into()));
        assert_eq!(super::parse("nfs://10.0.0.2:11111/").unwrap(), ("10.0.0.2".into(), Some(11111), "/".into()));
        assert!(super::parse("smb://nas/x").is_err());
    }
}
