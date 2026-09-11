use crate::shell::Shell;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "brewtank",
    version,
    about = "Isolated Homebrew environments — a venv for brew"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print the shell integration to eval from your startup file
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Create a new tank
    Create {
        name: String,
        /// Keep the global Homebrew on PATH behind this tank instead of hiding it
        #[arg(long)]
        inherit: bool,
    },
    /// List tanks
    List,
    /// Delete a tank and everything installed in it
    Remove {
        name: String,
        /// Delete even when the tank still has formulae installed
        #[arg(long, short)]
        force: bool,
    },
    /// Print the prefix of a tank, or of the active one
    Which { name: Option<String> },
    /// Check tanks for damage and drift
    Doctor,
    /// Re-copy bin/brew from the global Homebrew after updating it
    Sync {
        /// Tank to sync; defaults to all of them
        name: Option<String>,
        /// Restore a Library symlink clobbered by `brew update`
        #[arg(long)]
        repair_library: bool,
    },
    /// Write the tank's installed formulae to a Brewfile
    Freeze {
        /// Tank to export; defaults to the active one
        #[arg(long)]
        tank: Option<String>,
        /// Output path, or '-' for stdout
        #[arg(long, short, default_value = crate::bundle::DEFAULT_FILE)]
        output: PathBuf,
        /// Overwrite an existing file
        #[arg(long, short)]
        force: bool,
    },
    /// Install everything in a Brewfile into the tank
    Restore {
        /// Brewfile to read
        #[arg(default_value = crate::bundle::DEFAULT_FILE)]
        file: PathBuf,
        /// Tank to install into; defaults to the active one
        #[arg(long)]
        tank: Option<String>,
    },
    /// Activate a tank (requires the shell integration)
    Activate { name: String },
    /// Leave the active tank (requires the shell integration)
    Deactivate,
    /// Emit the shell code for activate/deactivate (used by the shell function)
    #[command(hide = true)]
    Env {
        #[arg(long, value_enum, default_value = "bash")]
        shell: Shell,
        #[command(subcommand)]
        action: EnvAction,
    },
}

#[derive(Subcommand)]
pub enum EnvAction {
    Activate { name: String },
    Deactivate,
}
