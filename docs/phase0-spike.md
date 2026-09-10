# Phase 0 spike — is an isolated Homebrew prefix actually viable?

Verified by hand against **Homebrew 6.0.22**, macOS 15 (Darwin 25.6.0), arm64,
using a throwaway tank at `/opt/bt/tst01`. Every claim below was executed, not
inferred.

**Verdict: viable.** Isolation is real — installed binaries link against the
tank's own libraries, not the global ones. One design flaw was found and fixed
(see "The `brew update` problem").

## The three mechanisms this design rests on

### 1. `HOMEBREW_PREFIX` comes from the path of `bin/brew`, not the environment

`/opt/homebrew/bin/brew:73-75`:

```bash
BREW_FILE_DIRECTORY="$(quiet_cd "${0%/*}/" && pwd -P)"
HOMEBREW_BREW_FILE="${BREW_FILE_DIRECTORY%/}/${0##*/}"
HOMEBREW_PREFIX="${HOMEBREW_BREW_FILE%/*/*}"
```

`HOMEBREW_PREFIX` and `HOMEBREW_REPOSITORY` are explicitly listed as *not*
user-overridable. So switching environments means putting a different physical
`bin/brew` on `PATH` — exactly the venv model.

Two consequences:

- `pwd -P` resolves symlinks, so **a tank prefix must be a real path**. A
  symlink pointing at the active tank would resolve to its target and break.
- The derivation depends only on the script's *location*, never its *contents* —
  so we are free to modify our copy of `bin/brew`. This is what makes the guard
  below possible.

Verified: with the global `HOMEBREW_PREFIX`/`CELLAR`/`REPOSITORY` still exported
in the environment, the tank's `brew` still reported its own paths. Inherited
values do not leak in.

### 2. Bottles relocate into a shorter prefix — but only a shorter one

`formula_installer.rb:1753` calls `keg.relocate_build_prefix(...)`, patching a
bottle built for `/opt/homebrew` to work under another prefix. The old rule that
a non-standard prefix forces source builds no longer applies.

The constraint is in `bottle_specification.rb:99-106`:

```ruby
cellar_relocatable = relocatable && cellar.to_s.bytesize >= HOMEBREW_CELLAR.to_s.bytesize
prefix_relocatable = relocatable && prefix.bytesize  >= HOMEBREW_PREFIX.to_s.bytesize
```

The comment explains why: *"Raw prefix strings are patched in place, so the byte
length decides."* Paths are overwritten within the existing bytes, so the target
can only get shorter.

> **A tank prefix must be no longer, in bytes, than the global prefix.**

13 bytes on Apple Silicon (`/opt/homebrew`), 10 on Intel (`/usr/local`). Hence
`/opt/bt/<id>` with a 5-character id — exactly 13 bytes. This must be computed
from the *actual* global prefix at runtime, never hard-coded.

Current homebrew-core bottle metadata still reports
`"cellar": "/opt/homebrew/Cellar"` (not the padded form Homebrew supports), so
the length rule is live today.

### 3. `Library` can be shared while `Cellar` stays private

`brew.sh:33-39` picks the Cellar from the *repository*, falling back to the prefix:

```bash
if [[ -d "${HOMEBREW_REPOSITORY}/Cellar" ]]; then
  HOMEBREW_CELLAR="${HOMEBREW_REPOSITORY}/Cellar"
else
  HOMEBREW_CELLAR="${HOMEBREW_PREFIX}/Cellar"
fi
```

and `HOMEBREW_LIBRARY="${HOMEBREW_REPOSITORY}/Library"` (`bin/brew:108`).

`bin/brew:86-89` resolves a symlinked `bin/brew` back to its target's parent and
uses *that* as `HOMEBREW_REPOSITORY`. So the tank's `bin/brew` must be a **real
file copy, not a symlink** — a symlink would point `HOMEBREW_REPOSITORY` at
`/opt/homebrew` and drag the Cellar back to the global one with it.

With a real copy plus `Library` as a symlink to the global `Library`:

| Variable | Value |
|---|---|
| `HOMEBREW_PREFIX` | `/opt/bt/<id>` |
| `HOMEBREW_REPOSITORY` | `/opt/bt/<id>` |
| `HOMEBREW_LIBRARY` | `/opt/bt/<id>/Library` → shared `/opt/homebrew/Library` |
| `HOMEBREW_CELLAR` | `/opt/bt/<id>/Cellar` (private) |

An empty tank costs ~10KB, and taps plus the API cache are shared for free.

## Results

| # | Test | Result |
|---|---|---|
| 1 | `brew --prefix/--cellar/--repository` inside the tank | Pass — all three resolve to the tank, and inherited global `HOMEBREW_*` values do not leak in |
| 2 | `brew install wget` | Pass — all 8 formulae **poured as bottles**, zero source builds, all under the tank's Cellar |
| 3 | **`otool -L` on the installed binary** | **Pass** — every Homebrew dylib resolves to `/opt/bt/tst01/opt/...`; no path leaks back to `/opt/homebrew` |
| 4 | `ffmpeg` (heavy dependency graph) | Pass — 13 further bottles poured, zero source builds; `ffmpeg -version` runs, and all 19 of its Homebrew dylibs resolve inside the tank |
| 5 | Global Homebrew unaffected | Pass — global Cellar unchanged at 51 formulae, `brew --prefix` still `/opt/homebrew`, repo working tree clean |
| 6 | `brew update` inside a tank | **Fail — flaw found, fixed below** |

Test 3 is the one that decides whether isolation is real rather than cosmetic:

```
/opt/bt/tst01/Cellar/wget/1.25.0/bin/wget:
	/opt/bt/tst01/opt/gettext/lib/libintl.8.dylib
	/opt/bt/tst01/opt/libunistring/lib/libunistring.5.dylib
	/opt/bt/tst01/opt/libidn2/lib/libidn2.0.dylib
	/opt/bt/tst01/opt/openssl@3/lib/libssl.3.dylib
	/opt/bt/tst01/opt/openssl@3/lib/libcrypto.3.dylib
	/opt/bt/tst01/opt/libpsl/lib/libpsl.5.dylib
	/usr/lib/libSystem.B.dylib          # system, expected
```

## The `brew update` problem

`brew update` runs git against `HOMEBREW_REPOSITORY` — which is the tank. It
`git init`s there and checks out the entire Homebrew/brew repository into the
tank prefix, **replacing the `Library` symlink with a real 60MB checkout** and
littering the prefix with `README.md`, `docs/`, `Dockerfile` and the rest. The
shared-Library design is destroyed in one command.

Worse, it is not only the explicit command: `brew bundle` triggered an
auto-update and did the same damage, also pulling a second copy of
portable-ruby into the tank.

The global repository was never touched in either case, so the blast radius is
limited to the tank — but the tank is ruined.

**Fix:** because `HOMEBREW_PREFIX` derives from the script's path and not its
contents (mechanism 1), the tank's copy of `bin/brew` can carry an injected
guard right after the shebang:

```bash
export HOMEBREW_NO_AUTO_UPDATE=1
case "${1-}" in
  update|update-reset|update-report)
    echo "brewtank: 'brew ${1}' is disabled inside a tank." >&2
    exit 1
    ;;
esac
```

This holds even when the user calls `brew` directly rather than going through
`brewtank`, because the tank's `bin/brew` is the only `brew` on `PATH` while the
tank is active. Verified: after injection, `brew update` is refused,
`brew bundle dump` completes without triggering an update, the `Library` symlink
survives, and `--prefix`/`--cellar`/`--repository` still resolve correctly.

Updates therefore happen against the global Homebrew, followed by
`brewtank sync` to refresh each tank's `bin/brew` copy.

## Other findings

- **Static archives fail ad-hoc signing during relocation.** Formulae shipping
  `.a` files (`gettext`, `openssl@3`) print
  `signing failed: Unrecognized Mach-O magic: 0x213c6172` — that magic is
  `!<ar`, an ar archive, which is not Mach-O and never was signable. Homebrew
  prints these as `Error:` but installation succeeds and the resulting binaries
  run. Cosmetic, but noisy enough that `brewtank` should explain it.
- **`brew bundle dump` behaves correctly in a tank**, listing only explicitly
  installed formulae (`wget`, not its seven dependencies). It also dumps
  `vscode` entries, which are irrelevant here — Phase 2 should pass
  `--no-vscode`.
- **Disk cost** is entirely the Cellar: 92MB for a `wget` tank, 199MB once
  `ffmpeg` was added (21 formulae). The tank scaffolding itself is negligible.
- **Relocation did not fail once.** Across 21 formulae — including `ffmpeg`,
  `openssl@3` and `gettext` — every single one poured a bottle and none fell
  back to a source build.

## Consequences for the implementation

1. Compute the prefix length budget from `brew --prefix` at runtime; reject tank
   creation with a clear error when the budget is exceeded.
2. The tank's `bin/brew` is a **copy**, never a symlink, and always carries the
   injected guard.
3. `brewtank doctor` must detect a `Library` that is no longer a symlink (a tank
   damaged by an older `brew update`) and offer to repair it.
4. `brewtank sync` re-copies `bin/brew` from the global Homebrew after a global
   update, re-injecting the guard.
5. Set `HOMEBREW_NO_AUTO_UPDATE=1` in the activation environment as well, so
   subprocesses that bypass the tank's `bin/brew` still behave.
