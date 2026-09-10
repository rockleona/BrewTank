//! Where BrewTank keeps its own state, and where tanks live on disk.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Default parent directory for tank prefixes.
///
/// Kept short on purpose: every byte here is one fewer available for the tank
/// id, because a tank prefix cannot exceed the global prefix in length.
pub const DEFAULT_ROOT: &str = "/opt/bt";

pub struct Paths {
    /// `~/.brewtank`
    pub home: PathBuf,
    /// Parent of all tank prefixes, `/opt/bt` by default.
    pub root: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let home = match std::env::var_os("BREWTANK_HOME") {
            Some(p) => PathBuf::from(p),
            None => {
                let h = std::env::var_os("HOME").context("HOME is not set")?;
                PathBuf::from(h).join(".brewtank")
            }
        };
        let root = match std::env::var_os("BREWTANK_ROOT") {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_ROOT),
        };
        Ok(Self { home, root })
    }

    pub fn registry(&self) -> PathBuf {
        self.home.join("registry.toml")
    }

    /// Directory of `<name> -> <prefix>` symlinks, for browsing only.
    ///
    /// These are never used as a prefix: `bin/brew` resolves its own path with
    /// `pwd -P`, so a symlinked prefix would collapse to its target.
    pub fn by_name(&self) -> PathBuf {
        self.home.join("by-name")
    }

    pub fn prefix_for(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// True if `p` is a direct child of the tank root — checked before any
    /// recursive delete.
    pub fn is_tank_prefix(&self, p: &Path) -> bool {
        p.parent() == Some(self.root.as_path())
    }
}
