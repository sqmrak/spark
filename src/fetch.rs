// pinned rootfs delivery. cached by sha256, streamed to disk while hashing

use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const STREAM_BUF: usize = 64 * 1024;

#[derive(Debug)]
pub enum Error {
    NoCache,
    Http { url: String, source: Box<ureq::Error> },
    Digest { want: String, got: String },
    Io { op: String, source: std::io::Error },
    Unpack(String),
}

impl Error {
    fn io(op: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io { op: op.into(), source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoCache => write!(f, "no HOME or XDG_CACHE_HOME for cache"),
            Error::Http { url, .. } => write!(f, "download {url}"),
            Error::Digest { want, got } => write!(f, "sha256 mismatch: want {want}, got {got}"),
            Error::Io { op, .. } => write!(f, "{op}"),
            Error::Unpack(m) => write!(f, "unpack: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http { source, .. } => Some(&**source),
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub struct Source {
    pub name: String,
    pub path: String,
    pub sha256: String,
}

impl Source {
    pub fn url(&self) -> String {
        format!("{VOID_MIRROR}/{}", self.path)
    }
}

const VOID_MIRROR: &str = "https://repo-default.voidlinux.org/live/current";

pub fn void_glibc() -> Result<Source, Error> {
    refresh("void-x86_64-ROOTFS-")
}

pub fn void_musl() -> Result<Source, Error> {
    refresh("void-x86_64-musl-ROOTFS-")
}

// fetch sha256sum.txt from the void mirror and find the latest rootfs
fn refresh(prefix: &str) -> Result<Source, Error> {
    let url = format!("{VOID_MIRROR}/sha256sum.txt");
    let resp = ureq::get(&url).call().map_err(|e| Error::Http { url, source: Box::new(e) })?;
    let text = resp.into_body().read_to_string().map_err(|e| Error::Io {
        op: "read sha256sum".into(),
        source: std::io::Error::other(e.to_string()),
    })?;
    for line in text.lines() {
        if !line.starts_with("SHA256 (") {
            continue;
        }
        let rest = &line["SHA256 (".len()..];
        let (filename, hash) = rest
            .split_once(") = ")
            .ok_or_else(|| Error::Unpack("malformed sha256sum line".into()))?;
        if filename.starts_with(prefix) && filename.ends_with(".tar.xz") {
            let name = if prefix.contains("musl") { "void-musl" } else { "void-glibc" };
            return Ok(Source {
                name: name.to_string(),
                path: filename.to_string(),
                sha256: hash.to_string(),
            });
        }
    }
    Err(Error::Unpack(format!("no {prefix}* found in sha256sum")))
}

// fetch and extract a rootfs, cached by sha256. cache layout: <cache>/<sha256>/rootfs
pub fn fetch(src: &Source) -> Result<PathBuf, Error> {
    let root = cache_dir()?.join(&src.sha256);
    let rootfs = root.join("rootfs");
    if rootfs.is_dir() {
        return Ok(rootfs);
    }
    std::fs::create_dir_all(&root).map_err(|e| Error::io(format!("mkdir {root:?}"), e))?;

    let tarball = root.join("download.tar.xz");
    download_verified(src, &tarball)?;
    let extracted = extract(&tarball, &rootfs);
    let _ = std::fs::remove_file(&tarball);
    extracted?;
    Ok(rootfs)
}

// stream url to disk, hashing as it lands. reject on digest mismatch
fn download_verified(src: &Source, dst: &Path) -> Result<(), Error> {
    let url = src.url();
    let mut resp = ureq::get(&url).call().map_err(|e| Error::Http { url, source: Box::new(e) })?;
    let mut reader = resp.body_mut().as_reader();
    let mut file = File::create(dst).map_err(|e| Error::io(format!("create {dst:?}"), e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_BUF];
    loop {
        let n = reader.read(&mut buf).map_err(|e| Error::io("read body", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| Error::io(format!("write {dst:?}"), e))?;
    }
    let got = hex(&hasher.finalize());
    if got != src.sha256 {
        let _ = std::fs::remove_file(dst);
        return Err(Error::Digest { want: src.sha256.to_string(), got });
    }
    Ok(())
}

pub fn verify(bytes: &[u8], want: &str) -> Result<(), Error> {
    let mut h = Sha256::new();
    h.update(bytes);
    let got = hex(&h.finalize());
    if got == want {
        Ok(())
    } else {
        Err(Error::Digest { want: want.to_string(), got })
    }
}

// extract tar.xz into dst via a temp sibling (atomic rename)
fn extract(tarball: &Path, dst: &Path) -> Result<(), Error> {
    let tmp = dst.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| Error::io(format!("mkdir {tmp:?}"), e))?;

    let f = File::open(tarball).map_err(|e| Error::io(format!("open {tarball:?}"), e))?;
    let xz = xz2::read::XzDecoder::new(f);
    let mut ar = tar::Archive::new(xz);
    ar.set_preserve_permissions(true);
    ar.unpack(&tmp).map_err(|e| Error::Unpack(e.to_string()))?;

    std::fs::rename(&tmp, dst).map_err(|e| Error::io("rename", e))
}

fn cache_dir() -> Result<PathBuf, Error> {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map_err(|_| Error::NoCache)?;
    Ok(base.join("spark"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn verify_accepts_match() {
        assert!(verify(b"", EMPTY_SHA).is_ok());
    }

    #[test]
    fn verify_rejects_tamper() {
        assert!(verify(b"tampered", EMPTY_SHA).is_err());
    }

    #[test]
    fn hex_is_lowercase_two_digit() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
