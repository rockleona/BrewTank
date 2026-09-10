//! `Brewfile` export and restore, delegating to Homebrew's built-in
//! `brew bundle`.

use crate::tank::Tank;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_FILE: &str = "Brewfile";

/// Entry types a tank can meaningfully hold. Casks install into `/Applications`
/// regardless of prefix, and `vscode`/`mas` entries are user-global, so none of
/// them describe a tank — they are dumped away and skipped on restore.
const SUPPORTED: &[&str] = &["brew", "tap"];

/// Dump the tank's installed formulae as a Brewfile.
pub fn freeze(tank: &Tank, output: &Path, force: bool) -> Result<()> {
    let to_stdout = output == Path::new("-");
    if !to_stdout && output.exists() && !force {
        bail!(
            "{} already exists — pass --force to overwrite",
            output.display()
        );
    }

    // Taps are included because a formula from a third-party tap cannot be
    // restored without one. Note that taps live in the shared Library, so these
    // are the global Homebrew's taps.
    let out = tank
        .brew_command()
        .args([
            "bundle",
            "dump",
            "--force",
            "--file=-",
            "--formula",
            "--tap",
        ])
        .output()
        .context("failed to run `brew bundle dump` in the tank")?;
    if !out.status.success() {
        bail!(
            "`brew bundle dump` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    if to_stdout {
        print!("{}", String::from_utf8_lossy(&out.stdout));
        return Ok(());
    }
    fs::write(output, &out.stdout)
        .with_context(|| format!("failed to write {}", output.display()))?;
    println!(
        "Wrote {} ({} formula(e)) from tank '{}'",
        output.display(),
        count_entries(&String::from_utf8_lossy(&out.stdout), "brew"),
        tank.name
    );
    Ok(())
}

/// Install everything in a Brewfile into the tank.
pub fn restore(tank: &Tank, input: &Path) -> Result<()> {
    let body =
        fs::read_to_string(input).with_context(|| format!("failed to read {}", input.display()))?;

    let (kept, skipped) = filter(&body);
    if count_entries(&kept, "brew") == 0 && count_entries(&kept, "tap") == 0 {
        bail!("{} has no formulae or taps to install", input.display());
    }
    for (kind, n) in &skipped {
        let entries = if *n == 1 { "entry" } else { "entries" };
        println!("Skipping {n} '{kind}' {entries}: a tank holds only formulae and taps");
    }

    // `brew bundle install` writes a lockfile beside its Brewfile, so hand it a
    // scratch copy rather than touching the user's directory.
    let scratch = ScratchFile::new(&kept)?;
    let status = tank
        .brew_command()
        .args(["bundle", "install"])
        .arg(format!("--file={}", scratch.path.display()))
        .status()
        .context("failed to run `brew bundle install` in the tank")?;
    if !status.success() {
        bail!("`brew bundle install` failed");
    }
    println!("\nRestored {} into tank '{}'", input.display(), tank.name);
    Ok(())
}

/// Split a Brewfile into the lines a tank can act on, and a count of what was
/// dropped, keyed by entry type.
fn filter(body: &str) -> (String, Vec<(String, usize)>) {
    let mut kept = String::new();
    let mut skipped: Vec<(String, usize)> = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim_start();
        let kind = trimmed.split([' ', '"', '\'']).next().unwrap_or("");
        if trimmed.is_empty() || trimmed.starts_with('#') || SUPPORTED.contains(&kind) {
            kept.push_str(line);
            kept.push('\n');
        } else {
            match skipped.iter_mut().find(|(k, _)| k == kind) {
                Some((_, n)) => *n += 1,
                None => skipped.push((kind.to_string(), 1)),
            }
        }
    }
    (kept, skipped)
}

fn count_entries(body: &str, kind: &str) -> usize {
    body.lines()
        .filter(|l| l.trim_start().starts_with(&format!("{kind} ")))
        .count()
}

/// A temporary file removed when it goes out of scope.
struct ScratchFile {
    path: PathBuf,
}

impl ScratchFile {
    fn new(contents: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "brewtank-{}-{}.Brewfile",
            std::process::id(),
            crate::provision::now()
        ));
        fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        // brew bundle leaves a lockfile beside the Brewfile.
        let _ = fs::remove_file(self.path.with_extension("Brewfile.lock.json"));
        let _ = fs::remove_file(format!("{}.lock.json", self.path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"tap "homebrew/core"
# a comment
brew "wget"
brew "jq"
cask "firefox"
vscode "anthropic.claude-code"
mas "Xcode", id: 497799835
"#;

    #[test]
    fn filter_keeps_formulae_and_taps() {
        let (kept, _) = filter(SAMPLE);
        assert!(kept.contains(r#"brew "wget""#));
        assert!(kept.contains(r#"tap "homebrew/core""#));
        assert!(kept.contains("# a comment"));
        assert!(!kept.contains("firefox"));
        assert!(!kept.contains("vscode"));
        assert!(!kept.contains("mas "));
    }

    #[test]
    fn filter_reports_what_it_dropped() {
        let (_, skipped) = filter(SAMPLE);
        assert_eq!(
            skipped,
            vec![
                ("cask".to_string(), 1),
                ("vscode".to_string(), 1),
                ("mas".to_string(), 1),
            ]
        );
    }

    #[test]
    fn entries_are_counted_by_kind() {
        assert_eq!(count_entries(SAMPLE, "brew"), 2);
        assert_eq!(count_entries(SAMPLE, "tap"), 1);
    }
}
