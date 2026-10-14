//! Locates and loads game assets from an existing Continuum install directory.
//! AIContinuum ships no original art or sound; it reads them from the player's install.

use std::path::{Path, PathBuf};

use crate::bmp::{self, IndexedImage};

pub const ASSETS_ENV: &str = "AICONTINUUM_ASSETS";

pub struct AssetDir {
    pub root: PathBuf,
}

impl AssetDir {
    /// Picks the asset root: explicit path, then `$AICONTINUUM_ASSETS`, then the executable's directory.
    pub fn locate(explicit: Option<&Path>) -> Result<Self, String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = explicit {
            candidates.push(p.to_path_buf());
        }
        if let Ok(p) = std::env::var(ASSETS_ENV) {
            candidates.push(PathBuf::from(p));
        }
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            candidates.push(dir);
        }
        candidates
            .into_iter()
            .find(|c| c.join("graphics").is_dir())
            .map(|root| Self { root })
            .ok_or_else(|| {
                format!("no Continuum install found (need a dir containing graphics/); pass a path or set {ASSETS_ENV}")
            })
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>, String> {
        let p = self.path(rel);
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Loads `graphics/<name>.bm2`.
    pub fn bm2(&self, name: &str) -> Result<IndexedImage, String> {
        let rel = format!("graphics/{name}.bm2");
        bmp::decode(&self.read(&rel)?).map_err(|e| format!("{rel}: {e}"))
    }
}
