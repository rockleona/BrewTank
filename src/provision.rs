//! Building a tank on disk, and keeping its `bin/brew` in step with the global one.

use crate::homebrew::Homebrew;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Directories a fresh Homebrew prefix is expected to have.
const SKELETON: &[&str] = &[
    "bin",
    "sbin",
    "etc",
    "include",
    "lib",
    "opt",
    "share",
    "var",
    "Cellar",
    "Caskroom",
    "Frameworks",
];

/// Marker used to detect (and re-apply) the guard in a tank's `bin/brew`.
pub const GUARD_MARKER: &str = "==> BrewTank guard";

/// Injected immediately after the shebang of the tank's copy of `bin/brew`.
///
/// `brew update` runs git against `HOMEBREW_REPOSITORY` — which is the tank —
/// and checks the whole Homebrew repository out on top of it, replacing the
/// shared `Library` symlink with a 60MB working copy. `brew bundle` used to
/// trigger the same thing via auto-update. See docs/phase0-spike.md.
///
/// Guarding here rather than in the `brewtank` wrapper is deliberate: a tank's
/// `bin/brew` is the only `brew` on `PATH` while the tank is active, so this
/// holds even when the user invokes `brew` directly. It is safe because
/// Homebrew derives `HOMEBREW_PREFIX` from the *path* of this script, never
/// from its contents.
const GUARD: &str = r#"
# ==> BrewTank guard (injected by brewtank; edits will be overwritten) <==
export HOMEBREW_NO_AUTO_UPDATE=1
case "${1-}" in
  update|update-reset|update-report)
    echo "brewtank: 'brew ${1}' is disabled inside a tank." >&2
    echo "          It would check the Homebrew repository out over this prefix." >&2
    echo "          Update the global Homebrew instead, then run: brewtank sync" >&2
    exit 1
    ;;
esac
# ==> end BrewTank guard <=="#;

/// Create the directory skeleton, the shared-`Library` symlink and the guarded
/// `bin/brew`.
pub fn create_prefix(prefix: &Path, brew: &Homebrew) -> Result<()> {
    if prefix.exists() {
        bail!("{} already exists", prefix.display());
    }
    for dir in SKELETON {
        let d = prefix.join(dir);
        fs::create_dir_all(&d).with_context(|| format!("failed to create {}", d.display()))?;
    }
    // Sharing Library keeps a tank at a few KB and gives it the global taps and
    // API cache for free; only Cellar has to be private.
    std::os::unix::fs::symlink(brew.library(), prefix.join("Library"))
        .with_context(|| format!("failed to link Library into {}", prefix.display()))?;
    install_brew(prefix, brew)
}

/// Copy the global `bin/brew` into the tank and re-apply the guard.
///
/// A copy, never a symlink: `bin/brew:86-89` resolves a symlinked `bin/brew`
/// back to its target's parent and uses that as `HOMEBREW_REPOSITORY`, which
/// would drag the Cellar back to the global one.
pub fn install_brew(prefix: &Path, brew: &Homebrew) -> Result<()> {
    let source = fs::read_to_string(&brew.brew_file)
        .with_context(|| format!("failed to read {}", brew.brew_file.display()))?;
    let dest = prefix.join("bin").join("brew");
    fs::write(&dest, guarded(&source)?)
        .with_context(|| format!("failed to write {}", dest.display()))?;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("failed to chmod {}", dest.display()))
}

fn guarded(source: &str) -> Result<String> {
    let Some((shebang, rest)) = source.split_once('\n') else {
        bail!("the global bin/brew has no shebang line; refusing to guard it");
    };
    if !shebang.starts_with("#!") {
        bail!("the global bin/brew does not start with a shebang; refusing to guard it");
    }
    Ok(format!("{shebang}{GUARD}\n{rest}"))
}

pub fn is_guarded(brew_file: &Path) -> bool {
    fs::read_to_string(brew_file).is_ok_and(|s| s.contains(GUARD_MARKER))
}

/// A short, unambiguous id. The alphabet omits `l`, `o`, `0` and `1`.
pub fn random_id(len: usize) -> Result<String> {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut buf = vec![0u8; len];
    fs::File::open("/dev/urandom")
        .context("failed to open /dev/urandom")?
        .read_exact(&mut buf)
        .context("failed to read /dev/urandom")?;
    // 256 is a multiple of 32, so masking stays uniform.
    Ok(buf
        .iter()
        .map(|b| ALPHABET[(b & 31) as usize] as char)
        .collect())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Format epoch seconds as `YYYY-MM-DD` (UTC).
pub fn format_date(secs: i64) -> String {
    // Howard Hinnant's civil-from-days.
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_goes_after_the_shebang() {
        let out = guarded("#!/bin/bash -pu\nset -u\necho hi\n").unwrap();
        let mut lines = out.lines();
        assert_eq!(lines.next().unwrap(), "#!/bin/bash -pu");
        assert!(out.contains(GUARD_MARKER));
        // The original body must survive intact.
        assert!(out.contains("\nset -u\necho hi\n"));
    }

    #[test]
    fn guard_refuses_a_script_without_a_shebang() {
        assert!(guarded("set -u\n").is_err());
    }

    #[test]
    fn ids_use_the_unambiguous_alphabet() {
        let id = random_id(5).unwrap();
        assert_eq!(id.len(), 5);
        assert!(!id.contains(['l', 'o', '0', '1']));
    }

    #[test]
    fn dates_match_known_values() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(1_767_225_600), "2026-01-01");
    }
}
