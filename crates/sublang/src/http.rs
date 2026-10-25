//! A `Read + Seek` over HTTP range requests, so the demuxer can read a file on a media server as
//! if it were local. Reads in 1 MiB chunks; seeking just moves the position.

use std::io::{self, Read, Seek, SeekFrom};

const CHUNK: u64 = 1024 * 1024;

pub struct HttpReader {
    client: reqwest::blocking::Client,
    url: String,
    pos: u64,
    len: Option<u64>,
    buf: Vec<u8>,
    buf_start: u64,
}

impl HttpReader {
    pub fn new(url: impl Into<String>) -> io::Result<Self> {
        let builder = reqwest::blocking::Client::builder();
        // Android: bundled roots (see encheo_plex::with_platform_roots).
        #[cfg(target_os = "android")]
        let builder = builder.tls_certs_only(
            webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| reqwest::Certificate::from_der(c).ok()),
        );
        let client = builder
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self { client, url: url.into(), pos: 0, len: None, buf: Vec::new(), buf_start: 0 })
    }

    fn fill(&mut self) -> io::Result<()> {
        let end = self.pos + CHUNK - 1;
        let resp = self
            .client
            .get(&self.url)
            .header("Range", format!("bytes={}-{end}", self.pos))
            .send()
            .map_err(io::Error::other)?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT && !resp.status().is_success() {
            return Err(io::Error::other(format!("HTTP {}", resp.status())));
        }
        if self.len.is_none() {
            // "bytes 0-1048575/34587282096"
            self.len = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse().ok());
        }
        self.buf = resp.bytes().map_err(io::Error::other)?.to_vec();
        self.buf_start = self.pos;
        Ok(())
    }
}

impl Read for HttpReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            if self.len.is_some_and(|l| self.pos >= l) {
                return Ok(0);
            }
            self.fill()?;
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

impl Seek for HttpReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?,
            SeekFrom::End(d) => {
                if self.len.is_none() {
                    self.fill()?;
                }
                let len = self.len.ok_or_else(|| io::Error::other("unknown length"))?;
                len.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before start"))?
            }
        };
        Ok(self.pos)
    }
}
