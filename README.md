# BrewTank

Isolated Homebrew environments — a `venv` for `brew`.

Homebrew installs everything into a single prefix (`/opt/homebrew` on Apple
Silicon). BrewTank lets you create isolated *tanks*: activate one and every
`brew install` — and every binary it links — lands in that tank instead. Your
global Homebrew keeps working, untouched, and `deactivate` restores your shell
exactly as it was.

```console
$ brewtank create scratch
$ brewtank activate scratch
(scratch) $ brew install wget      # installs into the tank, not /opt/homebrew
(scratch) $ which wget
/opt/bt/erjwu/bin/wget
(scratch) $ brewtank deactivate
$ which wget
wget not found
```

## Install

Requires macOS, Homebrew 6.x, and a Rust toolchain.

```sh
cargo build --release
cp target/release/brewtank /usr/local/bin/     # or anywhere on PATH
```

Tanks live under `/opt/bt`, which needs creating once:

```sh
sudo mkdir -p /opt/bt && sudo chown "$(id -un):admin" /opt/bt
```

Then add the shell integration to `~/.zshrc` (or `~/.bashrc`):

```sh
eval "$(brewtank init zsh)"     # or: bash
```

This step is required. Activating changes `PATH`, and a child process cannot
change its parent's environment — so `brewtank` defines a shell function, the
same way `pyenv` and `direnv` do.

## Commands

| Command | |
|---|---|
| `brewtank create <name> [--inherit]` | Create a tank |
| `brewtank activate <name>` | Switch into it |
| `brewtank deactivate` | Switch back |
| `brewtank list` | All tanks, with size and formula count |
| `brewtank which [<name>]` | Print a tank's prefix |
| `brewtank remove <name> [--force]` | Delete a tank and its contents |
| `brewtank doctor` | Check tanks for damage and drift |
| `brewtank sync [<name>]` | Re-copy `bin/brew` after updating global Homebrew |
| `brewtank freeze [-o <file>]` | Write the tank's formulae to a `Brewfile` |
| `brewtank restore [<file>]` | Install a `Brewfile` into the tank |

By default a tank is *strict*: activating it removes the global prefix from
`PATH` entirely, so only the tank's tools are visible — a formula the tank's
`brew` doesn't list can't be run by name either. That includes everyday tools
you installed globally (`git`, `python`, ...); install them into the tank if
you need them there.

`--inherit` keeps the global Homebrew reachable *behind* the tank instead, like
a virtualenv: the tank's tools win ties by going first, and everything else
falls through to `/opt/homebrew`.

Strictness is fixed at `create` time and stored in `~/.brewtank/registry.toml`
(`strict = true/false`); edit that line to change an existing tank.

## Reproducing a tank

`freeze` and `restore` wrap Homebrew's own `brew bundle`, so a tank can be kept
in version control and rebuilt elsewhere:

```sh
brewtank activate api
brewtank freeze                 # writes ./Brewfile
git add Brewfile

brewtank create api2
brewtank restore Brewfile --tank api2
```

`freeze` records only what you asked for, not the dependency closure — a tank
with `wget` and `jq` installed yields two `brew` lines, not ten.

Only `brew` and `tap` entries are handled. A `Brewfile` from elsewhere may also
carry `cask`, `vscode` or `mas` entries; those install outside any prefix, so
`restore` reports and skips them rather than acting on them.

## Updating Homebrew

`brew update` is refused inside a tank — it would check the Homebrew repository
out over the tank's prefix. Update the global Homebrew instead, then bring the
tanks along:

```sh
/opt/homebrew/bin/brew update
brewtank sync
```

`brewtank doctor` tells you when a tank has drifted from the global version.

## How it works

Homebrew derives `HOMEBREW_PREFIX` from the path of the `bin/brew` script that
was invoked — it is not an environment variable, and cannot be overridden. So a
tank is a second, minimal Homebrew prefix with its own copy of `bin/brew`, and
activating one is just a `PATH` change.

Homebrew 6.x relocates bottles into non-default prefixes, so tanks install real
bottles rather than building from source. Because relocation rewrites paths in
place, a tank prefix can never be longer than the global one — hence the short
`/opt/bt/<id>` paths and the name-to-id registry in `~/.brewtank`.

Only `Cellar` is private. `Library` is a symlink to the global Homebrew, so
tanks share its code, taps and API cache and an empty tank costs a few KB.

Sharing `Library` has one visible consequence: **taps are global**. A tap added
inside a tank is added for every tank and for the global Homebrew, and the
`tap` lines in a frozen `Brewfile` are the global Homebrew's taps. Formulae,
which is what actually gets installed, stay private to the tank.

[docs/phase0-spike.md](docs/phase0-spike.md) documents every mechanism this
relies on, with the Homebrew source references, all verified against 6.0.22.

## Tests

```sh
cargo test              # units
zsh  tests/e2e.sh       # end-to-end, creates and destroys a real tank
bash tests/e2e.sh
```

The end-to-end test installs a real formula and asserts, among other things,
that the global Homebrew is unchanged afterwards.

## Status

Early. Working: everything in the table above, on `zsh` and `bash`.
Not yet: `fish`, casks, and per-directory auto-activation.

## License

MIT — see [LICENSE](LICENSE).
