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
            .find(|c| resolve_case(c, "graphics").is_dir())
            .map(|root| Self { root })
            .ok_or_else(|| {
                format!("no Continuum install found (need a dir containing graphics/); pass a path or set {ASSETS_ENV}")
            })
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        resolve_case(&self.root, rel)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>, String> {
        let local = self.data_root().join(rel);
        let p = if is_settings(rel) && local.is_file() { local } else { self.path(rel) };
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Settings/maps are writable user data on Windows/Linux; existing Mac installs stay portable.
    pub fn data_root(&self) -> PathBuf {
        if let Some(path) = std::env::var_os("AICONTINUUM_DATA") { return PathBuf::from(path); }
        if self.root.join(".portable").is_file() { return self.root.clone(); }
        #[cfg(target_os = "windows")]
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(path).join("AIContinuum");
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(path) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
                return path.join("aicontinuum");
            }
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(".local/share/aicontinuum");
            }
        }
        self.root.clone()
    }

    pub fn prepare_data(&self) -> std::io::Result<()> {
        let root = self.data_root();
        std::fs::create_dir_all(&root)?;
        for name in ["profile.dat", "zone.dat", "keyboard.dat", "macro.dat"] {
            let source = self.path(name);
            let target = root.join(name);
            if source.is_file() && !target.exists() {
                // create_new never overwrites settings created by a simultaneous launch.
                use std::io::Write;
                let bytes = std::fs::read(&source)?;
                match std::fs::OpenOptions::new().write(true).create_new(true).open(target) {
                    Ok(mut out) => out.write_all(&bytes)?,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }

    /// Loads `graphics/<name>.bm2`.
    pub fn bm2(&self, name: &str) -> Result<IndexedImage, String> {
        let rel = format!("graphics/{name}.bm2");
        bmp::decode(&self.read(&rel)?).map_err(|e| format!("{rel}: {e}"))
    }

    /// Loads a graphic as RGBA (black = transparent), trying `.bm2`, `.png` then `.gif`.
    pub fn graphic(&self, name: &str) -> Result<RgbaImage, String> {
        if self.path(&format!("graphics/{name}.bm2")).is_file() {
            let img = self.bm2(name)?;
            return Ok(RgbaImage { width: img.width, height: img.height, pixels: img.to_rgba(true) });
        }
        for ext in ["png", "gif"] {
            let rel = format!("graphics/{name}.{ext}");
            if !self.path(&rel).is_file() {
                continue;
            }
            let img = image::load_from_memory(&self.read(&rel)?).map_err(|e| format!("{rel}: {e}"))?.to_rgba8();
            let (width, height) = img.dimensions();
            let mut pixels = img.into_raw();
            for px in pixels.chunks_exact_mut(4) {
                if px[0] == 0 && px[1] == 0 && px[2] == 0 {
                    px[3] = 0;
                }
            }
            return Ok(RgbaImage { width, height, pixels });
        }
        Err(format!("graphics/{name}: no .bm2/.png/.gif found"))
    }

    /// Reads `sound/<name>.wa2` (an ordinary WAV file).
    pub fn sound(&self, name: &str) -> Result<Vec<u8>, String> {
        self.read(&format!("sound/{name}.wa2"))
    }
}

pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn is_settings(name: &str) -> bool {
    ["profile.dat", "zone.dat", "keyboard.dat", "macro.dat"].iter().any(|n| name.eq_ignore_ascii_case(n))
}

// Continuum installs use mixed-case Windows filenames. Resolve each asset component
// on case-sensitive Linux filesystems, preferring an exact match.
fn resolve_case(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_owned();
    for part in Path::new(rel).components() {
        let exact = path.join(part.as_os_str());
        if exact.exists() { path = exact; continue; }
        let name = part.as_os_str().to_string_lossy();
        let mut matches: Vec<_> = std::fs::read_dir(&path).into_iter().flatten().flatten()
            .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&name)).map(|e| e.path()).collect();
        matches.sort();
        path = matches.into_iter().next().unwrap_or(exact);
    }
    path
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn mixed_case_assets_and_settings_do_not_overwrite_existing_files() {
        let root = std::env::temp_dir().join(format!("aic-paths-{}", std::process::id()));
        std::fs::create_dir_all(root.join("GRAPHICS")).unwrap();
        std::fs::write(root.join("GRAPHICS/Ships.BM2"), b"fixture").unwrap();
        std::fs::write(root.join(".portable"), b"").unwrap();
        std::fs::write(root.join("profile.dat"), b"preserve").unwrap();
        let assets = AssetDir { root: root.clone() };
        assert_eq!(assets.read("graphics/ships.bm2").unwrap(), b"fixture");
        assets.prepare_data().unwrap();
        assert_eq!(assets.read("profile.dat").unwrap(), b"preserve");
        std::fs::remove_dir_all(root).unwrap();
    }
}
