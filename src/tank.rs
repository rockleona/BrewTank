//! The tank type, and the metadata file each tank carries.

use crate::paths::Paths;
use crate::registry::Registry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Tank {
    pub name: String,
    pub id: String,
    pub prefix: PathBuf,
    pub strict: bool,
    pub created_at: i64,
}

impl Tank {
    pub fn resolve(paths: &Paths, registry: &Registry, name: &str) -> Result<Self> {
        let entry = registry.get(name)?;
        Ok(Self {
            name: name.to_string(),
            id: entry.id.clone(),
            prefix: paths.prefix_for(&entry.id),
            strict: entry.strict,
            created_at: entry.created_at,
        })
    }

    pub fn brew_file(&self) -> PathBuf {
        self.prefix.join("bin").join("brew")
    }

    pub fn library(&self) -> PathBuf {
        self.prefix.join("Library")
    }

    pub fn cellar(&self) -> PathBuf {
        self.prefix.join("Cellar")
    }

    /// Number of formulae installed, i.e. entries in the tank's Cellar.
    pub fn formula_count(&self) -> usize {
        fs::read_dir(self.cellar())
            .map(|d| d.flatten().filter(|e| !is_hidden(&e.file_name())).count())
            .unwrap_or(0)
    }

    /// A `brew` invocation bound to this tank.
    ///
    /// The tank's `bin/brew` derives its own prefix from its path, so no
    /// `HOMEBREW_*` setup is needed — but an *outer* tank's exported variables
    /// are cleared anyway so nothing depends on that override order.
    pub fn brew_command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(self.brew_file());
        cmd.env_remove("HOMEBREW_PREFIX")
            .env_remove("HOMEBREW_CELLAR")
            .env_remove("HOMEBREW_REPOSITORY")
            .env("HOMEBREW_NO_AUTO_UPDATE", "1");
        cmd
    }

    pub fn is_active(&self) -> bool {
        std::env::var("BREWTANK_ACTIVE").is_ok_and(|n| n == self.name)
    }

    /// Total bytes on disk, following no symlinks.
    ///
    /// `Library` is a symlink to the global Homebrew, so this reports what the
    /// tank actually costs rather than counting the shared installation.
    pub fn size_bytes(&self) -> u64 {
        dir_size(&self.prefix)
    }
}

/// The name of the tank currently active in this shell, if any.
pub fn active_name() -> Option<String> {
    std::env::var("BREWTANK_ACTIVE")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Written into the prefix so a tank can be identified from the directory
/// alone, without the registry.
#[derive(Serialize, Deserialize, Debug)]
pub struct Meta {
    pub name: String,
    pub id: String,
    pub created_at: i64,
    pub strict: bool,
    pub brewtank_version: String,
    /// The global Homebrew this tank's `bin/brew` was copied from.
    pub brew_version: String,
}

impl Meta {
    pub const FILE: &'static str = ".brewtank.toml";

    pub fn write(&self, prefix: &Path) -> Result<()> {
        let path = prefix.join(Self::FILE);
        let body = toml::to_string_pretty(self).context("failed to serialise tank metadata")?;
        fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn read(prefix: &Path) -> Result<Self> {
        let path = prefix.join(Self::FILE);
        let body = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&body).with_context(|| format!("{} is not valid TOML", path.display()))
    }
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?; // metadata() on DirEntry does not follow symlinks
            Some(if meta.is_dir() {
                dir_size(&e.path())
            } else {
                meta.len()
            })
        })
        .sum()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}{}", UNITS[0])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_render_in_the_largest_sensible_unit() {
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(2048), "2.0KB");
        assert_eq!(human_size(199 * 1024 * 1024), "199.0MB");
    }
}
