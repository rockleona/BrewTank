//! `~/.brewtank/registry.toml`: the name -> id mapping.
//!
//! Tank ids are short because the prefix length budget is tight, so users work
//! with names and never see the id unless they ask.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug)]
pub struct Registry {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub tanks: BTreeMap<String, Entry>,
}

fn one() -> u32 {
    1
}

// Not derived: `#[derive(Default)]` would use `u32::default()` and stamp a
// fresh registry with version 0, which no migration would recognise.
impl Default for Registry {
    fn default() -> Self {
        Self {
            version: one(),
            tanks: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry {
    pub id: String,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
    #[serde(default = "strict_by_default")]
    pub strict: bool,
}

fn strict_by_default() -> bool {
    true
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => {
                toml::from_str(&s).with_context(|| format!("{} is not valid TOML", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        let body = toml::to_string_pretty(self).context("failed to serialise the registry")?;
        // Write-then-rename so an interrupted save cannot truncate the registry.
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body).with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))
    }

    pub fn get(&self, name: &str) -> Result<&Entry> {
        self.tanks.get(name).with_context(|| {
            if self.tanks.is_empty() {
                format!(
                    "no tank named '{name}' (no tanks exist yet — try `brewtank create {name}`)"
                )
            } else {
                format!(
                    "no tank named '{name}'. Existing tanks: {}",
                    self.tanks.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            }
        })
    }

    pub fn id_taken(&self, id: &str) -> bool {
        self.tanks.values().any(|e| e.id == id)
    }
}

/// Tank names become directory entries and shell-visible strings, so keep them
/// boring.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("tank name must be between 1 and 64 characters");
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        bail!("tank name must start with a letter or digit: '{name}'");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        bail!(
            "tank name may only contain letters, digits, '-', '_' and '.': '{bad}' is not allowed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_registry_carries_the_current_version() {
        assert_eq!(Registry::default().version, 1);
        let body = toml::to_string_pretty(&Registry::default()).unwrap();
        assert!(body.contains("version = 1"), "{body}");
    }

    #[test]
    fn a_registry_without_a_version_field_is_read_as_version_1() {
        let r: Registry = toml::from_str("[tanks]\n").unwrap();
        assert_eq!(r.version, 1);
    }

    #[test]
    fn names_must_be_shell_and_path_safe() {
        assert!(validate_name("api").is_ok());
        assert!(validate_name("py3.13_x").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("-lead").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("has/slash").is_err());
    }
}
