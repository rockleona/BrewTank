mod bundle;
mod cli;
mod homebrew;
mod paths;
mod provision;
mod registry;
mod shell;
mod tank;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cli::{Cli, Command, EnvAction};
use homebrew::Homebrew;
use paths::Paths;
use registry::{Entry, Registry};
use tank::{Meta, Tank, human_size};

fn main() {
    if let Err(e) = run() {
        eprintln!("brewtank: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Init { shell } => {
            print!("{}", shell::init(shell));
            Ok(())
        }
        Command::Create { name, inherit } => create(&name, !inherit),
        Command::List => list(),
        Command::Remove { name, force } => remove(&name, force),
        Command::Which { name } => which(name.as_deref()),
        Command::Doctor => doctor(),
        Command::Sync {
            name,
            repair_library,
        } => sync(name.as_deref(), repair_library),
        Command::Activate { .. } | Command::Deactivate => bail!(
            "the shell integration is not installed, so this cannot change your shell.\n\
             Add this to your shell startup file and restart it:\n    \
             eval \"$(brewtank init zsh)\"    # or: bash"
        ),
        Command::Freeze {
            tank,
            output,
            force,
        } => bundle::freeze(&target_tank(tank.as_deref())?, &output, force),
        Command::Restore { file, tank } => bundle::restore(&target_tank(tank.as_deref())?, &file),
        Command::Env { shell, action } => env(shell, action),
    }
}

/// Resolve the tank a command should act on: the one named, else the active one.
fn target_tank(name: Option<&str>) -> Result<Tank> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    let name = match name {
        Some(n) => n.to_string(),
        None => tank::active_name()
            .context("no tank is active — activate one, or name it with --tank <name>")?,
    };
    let t = Tank::resolve(&paths, &reg, &name)?;
    if !t.brew_file().is_file() {
        bail!("tank '{name}' has no bin/brew — run `brewtank doctor`");
    }
    Ok(t)
}

fn create(name: &str, strict: bool) -> Result<()> {
    registry::validate_name(name)?;
    let paths = Paths::resolve()?;
    let mut reg = Registry::load(&paths.registry())?;
    if reg.tanks.contains_key(name) {
        bail!("a tank named '{name}' already exists");
    }

    let brew = Homebrew::detect()?;
    let id_len = brew.id_len(&paths.root)?;

    if !paths.root.is_dir() {
        bail!(
            "the tank root {root} does not exist. Create it once with:\n    \
             sudo mkdir -p {root} && sudo chown \"$(id -un):admin\" {root}",
            root = paths.root.display()
        );
    }

    let id = (0..16)
        .map(|_| provision::random_id(id_len))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|id| !reg.id_taken(id) && !paths.prefix_for(id).exists())
        .context("could not find an unused tank id; remove some tanks first")?;

    let prefix = paths.prefix_for(&id);
    let created_at = provision::now();

    provision::create_prefix(&prefix, &brew)?;

    // From here on a failure would leave a half-built prefix behind, so undo it.
    let mut finish = || -> Result<()> {
        Meta {
            name: name.to_string(),
            id: id.clone(),
            created_at,
            strict,
            brewtank_version: env!("CARGO_PKG_VERSION").to_string(),
            brew_version: brew.version.clone(),
        }
        .write(&prefix)?;

        reg.tanks.insert(
            name.to_string(),
            Entry {
                id: id.clone(),
                created_at,
                strict,
            },
        );
        reg.save(&paths.registry())?;
        link_by_name(&paths, name, &prefix)
    };
    if let Err(e) = finish() {
        let _ = std::fs::remove_dir_all(&prefix);
        return Err(e);
    }

    println!("Created tank '{name}' at {}", prefix.display());
    if strict {
        println!(
            "  strict: the global Homebrew is hidden from PATH while active \
             (create with --inherit to keep it)"
        );
    } else {
        println!("  inherit: the global Homebrew stays on PATH behind this tank");
    }
    println!("\nActivate it with:\n    brewtank activate {name}");
    Ok(())
}

/// A convenience symlink so tanks can be found by name in a file browser.
/// Never used as a prefix — `bin/brew` resolves its path with `pwd -P`.
fn link_by_name(paths: &Paths, name: &str, prefix: &std::path::Path) -> Result<()> {
    let dir = paths.by_name();
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let link = dir.join(name);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(prefix, &link)
        .with_context(|| format!("failed to create {}", link.display()))
}

fn list() -> Result<()> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    if reg.tanks.is_empty() {
        println!("No tanks yet. Create one with:\n    brewtank create <name>");
        return Ok(());
    }

    let active = tank::active_name();
    println!(
        "  {:<16} {:<8} {:>8} {:>9}  {:<10}  PREFIX",
        "NAME", "ID", "FORMULAE", "SIZE", "CREATED"
    );
    for name in reg.tanks.keys() {
        let t = Tank::resolve(&paths, &reg, name)?;
        let marker = if active.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{marker} {:<16} {:<8} {:>8} {:>9}  {:<10}  {}",
            t.name,
            t.id,
            t.formula_count(),
            human_size(t.size_bytes()),
            provision::format_date(t.created_at),
            t.prefix.display()
        );
    }
    Ok(())
}

fn remove(name: &str, force: bool) -> Result<()> {
    let paths = Paths::resolve()?;
    let mut reg = Registry::load(&paths.registry())?;
    let t = Tank::resolve(&paths, &reg, name)?;

    if t.is_active() {
        bail!("'{name}' is active in this shell — run `brewtank deactivate` first");
    }

    let count = t.formula_count();
    if count > 0 && !force {
        bail!(
            "'{name}' still has {count} formula(e) installed. \
             Re-run with --force to delete it and everything in it."
        );
    }

    // Never hand remove_dir_all anything that is not a direct child of the
    // tank root.
    if !paths.is_tank_prefix(&t.prefix) {
        bail!(
            "refusing to delete {}: it is not directly under {}",
            t.prefix.display(),
            paths.root.display()
        );
    }
    if t.prefix
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        bail!("refusing to delete {}: it is a symlink", t.prefix.display());
    }

    if t.prefix.exists() {
        std::fs::remove_dir_all(&t.prefix)
            .with_context(|| format!("failed to delete {}", t.prefix.display()))?;
    }
    let _ = std::fs::remove_file(paths.by_name().join(name));
    reg.tanks.remove(name);
    reg.save(&paths.registry())?;

    println!("Removed tank '{name}' ({})", t.prefix.display());
    Ok(())
}

fn which(name: Option<&str>) -> Result<()> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    let name = match name {
        Some(n) => n.to_string(),
        None => tank::active_name().context("no tank is active in this shell")?,
    };
    println!("{}", Tank::resolve(&paths, &reg, &name)?.prefix.display());
    Ok(())
}

fn env(shell: shell::Shell, action: EnvAction) -> Result<()> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    match action {
        EnvAction::Activate { name } => {
            let t = Tank::resolve(&paths, &reg, &name)?;
            if !t.brew_file().is_file() {
                bail!(
                    "tank '{name}' has no bin/brew at {} — run `brewtank doctor`",
                    t.prefix.display()
                );
            }
            let brew = Homebrew::detect()?;
            print!("{}", shell::activate(&t, &brew.prefix, shell)?);
        }
        EnvAction::Deactivate => {
            if tank::active_name().is_none() {
                bail!("no tank is active in this shell");
            }
            print!("{}", shell::deactivate(shell)?);
        }
    }
    Ok(())
}

fn doctor() -> Result<()> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    let mut problems = 0usize;

    let brew = Homebrew::detect()?;
    println!(
        "Global Homebrew  {} at {}",
        brew.version,
        brew.prefix.display()
    );

    match brew.id_len(&paths.root) {
        Ok(n) => println!(
            "Tank root        {} ({n}-character ids)",
            paths.root.display()
        ),
        Err(e) => {
            println!("Tank root        {} — {e}", paths.root.display());
            problems += 1;
        }
    }
    if !paths.root.is_dir() {
        println!("  ! {} does not exist", paths.root.display());
        problems += 1;
    }

    if reg.tanks.is_empty() {
        println!("\nNo tanks.");
    } else {
        println!();
    }
    for name in reg.tanks.keys() {
        let t = Tank::resolve(&paths, &reg, name)?;
        let mut issues: Vec<String> = Vec::new();

        if !t.prefix.is_dir() {
            issues.push(format!("prefix {} is missing", t.prefix.display()));
        } else {
            let lib = t.library();
            match lib.symlink_metadata() {
                Ok(m) if m.file_type().is_symlink() => {
                    if !lib.exists() {
                        issues.push("Library symlink is dangling".into());
                    }
                }
                Ok(_) => issues.push(
                    "Library is a real directory, not a symlink — `brew update` was run \
                     inside this tank. Fix with `brewtank sync --repair-library`"
                        .into(),
                ),
                Err(_) => issues.push("Library is missing".into()),
            }

            let bf = t.brew_file();
            if !bf.is_file() {
                issues.push("bin/brew is missing".into());
            } else if !provision::is_guarded(&bf) {
                issues.push("bin/brew is missing the BrewTank guard — run `brewtank sync`".into());
            }

            match Meta::read(&t.prefix) {
                Ok(meta) if meta.brew_version != brew.version => issues.push(format!(
                    "built against {} but the global Homebrew is now {} — run `brewtank sync`",
                    meta.brew_version, brew.version
                )),
                Ok(_) => {}
                Err(_) => issues.push(format!("{} is unreadable", Meta::FILE)),
            }
        }

        if issues.is_empty() {
            println!("ok   {name}");
        } else {
            problems += issues.len();
            println!("FAIL {name}");
            for i in &issues {
                println!("     - {i}");
            }
        }
    }

    if problems > 0 {
        println!("\n{problems} problem(s) found.");
        std::process::exit(1);
    }
    println!("\nNo problems found.");
    Ok(())
}

fn sync(name: Option<&str>, repair_library: bool) -> Result<()> {
    let paths = Paths::resolve()?;
    let reg = Registry::load(&paths.registry())?;
    let brew = Homebrew::detect()?;

    let names: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => reg.tanks.keys().cloned().collect(),
    };
    if names.is_empty() {
        println!("No tanks to sync.");
        return Ok(());
    }

    for name in &names {
        let t = Tank::resolve(&paths, &reg, name)?;
        if !t.prefix.is_dir() {
            println!("skip {name}: prefix {} is missing", t.prefix.display());
            continue;
        }

        if repair_library {
            let lib = t.library();
            let clobbered = lib
                .symlink_metadata()
                .is_ok_and(|m| !m.file_type().is_symlink());
            if clobbered {
                std::fs::remove_dir_all(&lib)
                    .with_context(|| format!("failed to remove {}", lib.display()))?;
                std::os::unix::fs::symlink(brew.library(), &lib)
                    .with_context(|| format!("failed to relink {}", lib.display()))?;
                println!("     {name}: restored the Library symlink");
                println!(
                    "     {name}: note — `brew update` may also have left repository files \
                     (README.md, docs/, ...) in {}",
                    t.prefix.display()
                );
            }
        }

        provision::install_brew(&t.prefix, &brew)?;

        let mut meta = Meta::read(&t.prefix).unwrap_or(Meta {
            name: t.name.clone(),
            id: t.id.clone(),
            created_at: t.created_at,
            strict: t.strict,
            brewtank_version: env!("CARGO_PKG_VERSION").to_string(),
            brew_version: brew.version.clone(),
        });
        meta.brew_version = brew.version.clone();
        meta.brewtank_version = env!("CARGO_PKG_VERSION").to_string();
        meta.write(&t.prefix)?;

        println!("ok   {name}: bin/brew synced from {}", brew.version);
    }
    Ok(())
}
