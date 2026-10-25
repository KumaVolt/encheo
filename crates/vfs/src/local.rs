//! A folder on this computer (or a share the OS has already mounted).

use crate::{DirectUrl, Entry, Error, ReadSeek, Result, join};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub struct Local {
    root: PathBuf,
}

impl Local {
    pub fn new(root: &str) -> Result<Self> {
        let root = PathBuf::from(root);
        if !root.is_dir() {
            return Err(Error::NotFound(root.display().to_string()));
        }
        Ok(Self { root })
    }

    fn path(&self, rel: &str) -> Result<PathBuf> {
        let rel = Path::new(rel.trim_matches('/'));
        // Never step outside the chosen folder.
        if rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(Error::NotFound(rel.display().to_string()));
        }
        Ok(self.root.join(rel))
    }

    pub fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.path(rel)?)? {
            let e = e?;
            let meta = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue, // broken links and the like
            };
            let name = e.file_name().to_string_lossy().into_owned();
            out.push(Entry {
                path: join(rel, &name),
                name,
                is_dir: meta.is_dir(),
                size: meta.is_file().then(|| meta.len()),
            });
        }
        Ok(out)
    }

    pub fn open(&self, rel: &str) -> Result<Box<dyn ReadSeek>> {
        let f = File::open(self.path(rel)?)?;
        let len = f.metadata().ok().map(|m| m.len());
        Ok(Box::new(LocalFile { f, len }))
    }

    pub fn direct_url(&self, rel: &str) -> DirectUrl {
        DirectUrl { url: self.path(rel).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), headers: vec![] }
    }
}

struct LocalFile {
    f: File,
    len: Option<u64>,
}

impl Read for LocalFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.f.read(buf)
    }
}

impl Seek for LocalFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.f.seek(pos)
    }
}

impl ReadSeek for LocalFile {
    fn len(&self) -> Option<u64> {
        self.len
    }
}
