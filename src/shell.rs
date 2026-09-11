//! Generating the shell code that actually switches environments.
//!
//! A child process cannot change its parent's `PATH`, so `brewtank` follows the
//! pyenv/direnv pattern: this binary only *prints* shell statements, and a shell
//! function installed by `brewtank init` evaluates them.

use crate::tank::Tank;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
}

/// Environment variables activation takes over. All are real environment
/// variables, so `deactivate` can read their saved values back out of its own
/// environment. `PS1` and `FPATH` are handled with shell-side snippets instead,
/// because a prompt is a shell variable this process cannot see.
const MANAGED: &[&str] = &[
    "PATH",
    "MANPATH",
    "INFOPATH",
    "HOMEBREW_PREFIX",
    "HOMEBREW_CELLAR",
    "HOMEBREW_REPOSITORY",
    "HOMEBREW_NO_AUTO_UPDATE",
];

const UNSET_LIST: &str = "_BREWTANK_UNSET";

/// Quote a value so the shell reads it back verbatim.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn saved_name(var: &str) -> String {
    format!("_BREWTANK_OLD_{var}")
}

/// The values in effect *before* any tank was activated.
///
/// When a tank is already active the current environment is the wrong baseline:
/// layering a second tank on top of it would leave the first tank's entries on
/// `PATH` forever. So in that case the saved values are used instead.
fn baseline() -> BTreeMap<&'static str, Option<String>> {
    let stacked = std::env::var_os("BREWTANK_ACTIVE").is_some();
    let previously_unset: Vec<String> = std::env::var(UNSET_LIST)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();

    MANAGED
        .iter()
        .map(|&var| {
            let value = if stacked {
                if previously_unset.iter().any(|v| v == var) {
                    None
                } else {
                    std::env::var(saved_name(var)).ok()
                }
            } else {
                std::env::var(var).ok()
            };
            (var, value)
        })
        .collect()
}

/// Prepend `entry` to a colon-separated search path.
///
/// An unset variable becomes `entry:`; the trailing colon is what tells `man`
/// and `info` to fall back to their built-in defaults, matching what
/// `brew shellenv` does.
fn prepend(entry: &Path, existing: Option<&String>) -> String {
    match existing {
        Some(v) => format!("{}:{v}", entry.display()),
        None => format!("{}:", entry.display()),
    }
}

pub fn activate(tank: &Tank, global_prefix: &Path, shell: Shell) -> Result<String> {
    let base = baseline();
    let mut out = String::new();

    if std::env::var_os("BREWTANK_ACTIVE").is_some() {
        out.push_str(&deactivate(shell)?);
        out.push('\n');
    }

    // Remember what to restore, and which variables to unset again.
    let mut unset_before: Vec<&str> = Vec::new();
    for (&var, value) in &base {
        match value {
            Some(v) => {
                let _ = writeln!(out, "{n}={v}; export {n}", n = saved_name(var), v = q(v));
            }
            None => {
                let _ = writeln!(out, "unset {}", saved_name(var));
                unset_before.push(var);
            }
        }
    }
    let _ = writeln!(
        out,
        "{UNSET_LIST}={}; export {UNSET_LIST}",
        q(&unset_before.join(" "))
    );

    // A strict tank (the default) hides the global Homebrew entirely; an
    // inheriting one only wins ties by going first, the way a virtualenv does.
    let mut entries: Vec<String> = base["PATH"]
        .as_deref()
        .unwrap_or_default()
        .split(':')
        .filter(|e| !e.is_empty())
        .filter(|e| !(tank.strict && Path::new(e).starts_with(global_prefix)))
        .map(str::to_string)
        .collect();
    entries.insert(0, tank.prefix.join("sbin").display().to_string());
    entries.insert(0, tank.prefix.join("bin").display().to_string());

    let assignments: Vec<(&str, String)> = vec![
        ("PATH", entries.join(":")),
        (
            "MANPATH",
            prepend(&tank.prefix.join("share/man"), base["MANPATH"].as_ref()),
        ),
        (
            "INFOPATH",
            prepend(&tank.prefix.join("share/info"), base["INFOPATH"].as_ref()),
        ),
        ("HOMEBREW_PREFIX", tank.prefix.display().to_string()),
        (
            "HOMEBREW_CELLAR",
            tank.prefix.join("Cellar").display().to_string(),
        ),
        ("HOMEBREW_REPOSITORY", tank.prefix.display().to_string()),
        // Auto-update would check the Homebrew repository out over the prefix.
        // The tank's bin/brew guards this too; this covers anything that finds
        // brew by another route.
        ("HOMEBREW_NO_AUTO_UPDATE", "1".to_string()),
        ("BREWTANK_ACTIVE", tank.name.clone()),
        ("BREWTANK_PREFIX", tank.prefix.display().to_string()),
        ("BREWTANK_ID", tank.id.clone()),
    ];
    for (var, value) in assignments {
        let _ = writeln!(out, "{var}={}; export {var}", q(&value));
    }

    if shell == Shell::Zsh {
        let site = tank.prefix.join("share/zsh/site-functions");
        let _ = writeln!(out, "_BREWTANK_OLD_FPATH=\"${{FPATH-}}\"");
        let _ = writeln!(
            out,
            "FPATH={}\"${{FPATH:+:$FPATH}}\"; export FPATH",
            q(&site.display().to_string())
        );
    }

    // PS1 is a shell variable, not an environment variable, so this process
    // cannot read it — do the work shell-side.
    let _ = writeln!(
        out,
        "if [ -z \"${{BREWTANK_DISABLE_PROMPT-}}\" ]; then\n  \
         _BREWTANK_OLD_PS1=\"${{PS1-}}\"\n  \
         PS1={}\"${{PS1-}}\"\nfi",
        q(&format!("({}) ", tank.name))
    );

    Ok(out)
}

pub fn deactivate(shell: Shell) -> Result<String> {
    let mut out = String::new();
    let previously_unset: Vec<String> = std::env::var(UNSET_LIST)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();

    for &var in MANAGED {
        let saved = saved_name(var);
        // Falling back to `unset` when nothing was saved keeps a half-written
        // environment from leaving a tank path behind.
        match std::env::var(&saved) {
            Ok(v) if !previously_unset.iter().any(|u| u == var) => {
                let _ = writeln!(out, "{var}={}; export {var}", q(&v));
            }
            _ => {
                let _ = writeln!(out, "unset {var}");
            }
        }
        let _ = writeln!(out, "unset {saved}");
    }
    let _ = writeln!(
        out,
        "unset {UNSET_LIST} BREWTANK_ACTIVE BREWTANK_PREFIX BREWTANK_ID"
    );

    if shell == Shell::Zsh {
        let _ = writeln!(
            out,
            "if [ -n \"${{_BREWTANK_OLD_FPATH+x}}\" ]; then\n  \
             FPATH=\"$_BREWTANK_OLD_FPATH\"; export FPATH\n  \
             unset _BREWTANK_OLD_FPATH\nfi"
        );
    }
    let _ = writeln!(
        out,
        "if [ -n \"${{_BREWTANK_OLD_PS1+x}}\" ]; then\n  \
         PS1=\"$_BREWTANK_OLD_PS1\"\n  \
         unset _BREWTANK_OLD_PS1\nfi"
    );

    Ok(out)
}

/// The shell function that makes `activate`/`deactivate` affect the caller.
pub fn init(shell: Shell) -> String {
    let flavour = match shell {
        Shell::Zsh => "zsh",
        Shell::Bash => "bash",
    };
    format!(
        r#"# brewtank shell integration ({flavour})
# Add to your shell startup file:  eval "$(brewtank init {flavour})"
brewtank() {{
  case "${{1-}}" in
    activate|deactivate)
      local _bt_script
      _bt_script="$(command brewtank env --shell {flavour} "$@")" || return $?
      eval "$_bt_script"
      ;;
    *)
      command brewtank "$@"
      ;;
  esac
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(q("a'b"), r"'a'\''b'");
        assert_eq!(q("/opt/bt/x y"), "'/opt/bt/x y'");
    }

    #[test]
    fn prepend_marks_an_unset_path_with_a_trailing_colon() {
        assert_eq!(prepend(Path::new("/a/man"), None), "/a/man:");
        assert_eq!(
            prepend(Path::new("/a/man"), Some(&"/b".to_string())),
            "/a/man:/b"
        );
    }

    #[test]
    fn init_defines_a_function_that_evaluates_env() {
        let s = init(Shell::Zsh);
        assert!(s.contains("brewtank() {"));
        assert!(s.contains("command brewtank env --shell zsh"));
        assert!(s.contains("eval \"$_bt_script\""));
    }
}
