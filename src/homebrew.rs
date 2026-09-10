//! Discovery of the *global* Homebrew installation that tanks are derived from.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Candidate locations for a global Homebrew, most likely first.
const CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/brew",
    "/usr/local/bin/brew",
    "/home/linuxbrew/.linuxbrew/bin/brew",
];

pub struct Homebrew {
    /// Path to the global `bin/brew`, the template every tank copies.
    pub brew_file: PathBuf,
    pub prefix: PathBuf,
    pub version: String,
}

impl Homebrew {
    /// Locate the global Homebrew.
    ///
    /// Deliberately does *not* search `PATH`: while a tank is active the `brew`
    /// on `PATH` is the tank's own copy, and resolving to that would make every
    /// tank derive from the previous one.
    pub fn detect() -> Result<Self> {
        let brew_file = match std::env::var_os("BREWTANK_BREW") {
            Some(p) => {
                let p = PathBuf::from(p);
                if !p.is_file() {
                    bail!(
                        "BREWTANK_BREW points at {}, which is not a file",
                        p.display()
                    );
                }
                p
            }
            None => CANDIDATES
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
                .context(
                    "no global Homebrew found in /opt/homebrew or /usr/local.\n\
                     Install Homebrew first, or set BREWTANK_BREW to its bin/brew.",
                )?,
        };

        let prefix = PathBuf::from(capture(&brew_file, "--prefix")?);
        let version = capture(&brew_file, "--version")?
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            brew_file,
            prefix,
            version,
        })
    }

    pub fn library(&self) -> PathBuf {
        self.prefix.join("Library")
    }

    /// How many characters a tank id may have under `root`.
    ///
    /// Bottle relocation patches prefix strings in place, so a tank prefix can
    /// never be longer in bytes than the prefix the bottle was built for. See
    /// `bottle_specification.rb:99-106` and docs/phase0-spike.md.
    pub fn id_len(&self, root: &Path) -> Result<usize> {
        let budget = self.prefix.as_os_str().len();
        let used = root.as_os_str().len() + 1; // + the separating '/'
        let Some(len) = budget.checked_sub(used).filter(|n| *n >= 2) else {
            bail!(
                "tank root {} is too long: prefixes must fit in {} bytes to match {},\n\
                 which leaves no room for a tank id. Set BREWTANK_ROOT to something shorter.",
                root.display(),
                budget,
                self.prefix.display(),
            );
        };
        Ok(len)
    }
}

fn capture(brew: &Path, arg: &str) -> Result<String> {
    let out = Command::new(brew)
        .arg(arg)
        // Keep a tank's exported HOMEBREW_* from confusing the global brew.
        .env_remove("HOMEBREW_PREFIX")
        .env_remove("HOMEBREW_CELLAR")
        .env_remove("HOMEBREW_REPOSITORY")
        .output()
        .with_context(|| format!("failed to run {} {arg}", brew.display()))?;
    if !out.status.success() {
        bail!(
            "{} {arg} failed: {}",
            brew.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
