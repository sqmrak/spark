// throwaway test environment under /tmp. nexus runs here instead of /rust
// teardown removes the tree on drop

use nexus::Layout;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Error {
    Stale(PathBuf),
    Io { op: String, source: std::io::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Stale(p) => write!(f, "stale sandbox at {p:?} could not be cleared"),
            Error::Io { op, .. } => write!(f, "{op}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Stale(_) => None,
        }
    }
}

pub struct Sandbox {
    base: PathBuf,
    layout: Layout,
}

impl Sandbox {
    pub fn base(&self) -> &PathBuf {
        &self.base
    }

    pub fn create(name: &str) -> Result<Sandbox, Error> {
        let base = Self::base_path(name);
        if base.exists() {
            unmount_under(&base);
            force_remove(&base);
            if base.exists() {
                return Err(Error::Stale(base));
            }
        }
        std::fs::create_dir_all(&base)
            .map_err(|e| Error::Io { op: format!("mkdir {base:?}"), source: e })?;

        let root = base.join("rust");
        std::fs::create_dir_all(&root)
            .map_err(|e| Error::Io { op: format!("mkdir {root:?}"), source: e })?;
        Ok(Sandbox { base, layout: Layout::new(root) })
    }

    pub fn path_only(name: &str) -> Sandbox {
        let base = Self::base_path(name);
        Sandbox { base: base.clone(), layout: Layout::new(base.join("rust")) }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn base_path(name: &str) -> PathBuf {
        let pid = unsafe { libc::getpid() };
        let slug = name.replace([' ', '/', ':', '\\'], "-");
        std::env::temp_dir().join(format!("spark-{pid}-{slug}"))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        unmount_under(&self.base);
        force_remove(&self.base);
    }
}

// recursive unmount of everything under a path. best effort
fn unmount_under(base: &Path) {
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                unmount_under(&e.path());
            }
            targets.push(e.path());
        }
    }
    targets.sort_by(|a, b| b.cmp(a));
    for t in &targets {
        let rc = unsafe {
            libc::umount2(t.as_os_str().as_encoded_bytes().as_ptr() as *const _, libc::MNT_DETACH)
        };
        if rc == 0 {
            continue;
        }
        let _ = std::fs::remove_dir_all(t);
    }
}

// force-remove a tree, chmod-ing dirs writable on the way down. best effort
fn force_remove(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            force_remove(&path);
        }
        // chmod so a readonly dir or file can be deleted
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::remove_dir(base);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_cleans_up() {
        let sb = Sandbox::create("unit").unwrap();
        assert!(sb.base.exists());
        let p = sb.base.clone();
        drop(sb);
        assert!(!p.exists());
    }

    #[test]
    fn stale_tree_is_cleared() {
        let sb = Sandbox::create("teardown").unwrap();
        let _ = std::fs::write(sb.base.join("marker"), b"x");
        drop(sb);
        assert!(!Sandbox::base_path("teardown").exists());
    }
}
