//! On-disk layout of a Mandala home directory.
//!
//! ```text
//! <root>/
//!   modules/<id>.so        signed module binaries
//!   modules/<id>.so.sig    detached Ed25519 signature (hex or raw)
//!   trust/*.pub            trusted public keys (hex)
//!   themes/*.json          user theme manifests
//!   data/<id>/             per-module private storage
//!   runtime/               verified copies actually loaded (private)
//!   quarantine/            rejected binaries
//! ```

use crate::error::{MandalaError, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct HostPaths {
    pub root: PathBuf,
}

impl HostPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$MANDALA_HOME`, else `./mandala-home`.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("MANDALA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("mandala-home")),
        )
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.root.join("modules")
    }
    pub fn trust_dir(&self) -> PathBuf {
        self.root.join("trust")
    }
    pub fn themes_dir(&self) -> PathBuf {
        self.root.join("themes")
    }
    pub fn data_dir(&self, module_id: &str) -> PathBuf {
        self.root.join("data").join(module_id)
    }
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }
    pub fn quarantine_dir(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    pub fn module_file(&self, id: &str) -> PathBuf {
        self.modules_dir()
            .join(format!("{id}.{}", std::env::consts::DLL_EXTENSION))
    }

    pub fn signature_file(&self, id: &str) -> PathBuf {
        let mut p = self.module_file(id).into_os_string();
        p.push(".sig");
        PathBuf::from(p)
    }

    /// Create the directory tree. `runtime/` is made owner-only on Unix.
    pub fn ensure(&self) -> Result<()> {
        for d in [
            self.modules_dir(),
            self.trust_dir(),
            self.themes_dir(),
            self.root.join("data"),
            self.runtime_dir(),
            self.quarantine_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| MandalaError::io(&d, e))?;
        }
        restrict_dir(&self.runtime_dir())?;
        restrict_dir(&self.quarantine_dir())?;
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_dir(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| MandalaError::io(p, e))
}

#[cfg(not(unix))]
fn restrict_dir(_p: &Path) -> Result<()> {
    Ok(())
}
