#!/bin/sh
# End-to-end test for brewtank. Run under both zsh and bash:
#
#     zsh  tests/e2e.sh
#     bash tests/e2e.sh
#
# Creates a real tank, installs a real formula into it, then removes it. The
# most important assertion is the last one: the global Homebrew must be byte-for
# byte unchanged afterwards.

set -u

BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/brewtank"
TANK_NAME="bte2e"
FAILED=0

case "${ZSH_VERSION-}" in
  "") SHELL_KIND=bash ;;
  *)  SHELL_KIND=zsh ;;
esac

say()  { printf '\n=== %s ===\n' "$1"; }
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n     expected: %s\n     actual:   %s\n' "$1" "$2" "$3"; FAILED=$((FAILED + 1)); }

check() { # check <label> <expected> <actual>
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1" "$2" "$3"; fi
}
check_contains() { # check_contains <label> <needle> <haystack>
  case "$3" in
    *"$2"*) pass "$1" ;;
    *) fail "$1" "something containing '$2'" "$3" ;;
  esac
}

say "setup ($SHELL_KIND)"
[ -x "$BIN" ] || { echo "build it first: cargo build"; exit 1; }
PATH="$(dirname "$BIN"):$PATH"
export PATH
eval "$("$BIN" init "$SHELL_KIND")"
pass "shell integration loaded"

GLOBAL_PREFIX="$(/opt/homebrew/bin/brew --prefix)"
GLOBAL_LIST_BEFORE="$(/opt/homebrew/bin/brew list --formula | sort)"
PS1="base> "
ORIGINAL_PATH="$PATH"
ORIGINAL_MANPATH="${MANPATH-<unset>}"

brewtank remove "$TANK_NAME" --force >/dev/null 2>&1 || true

say "create"
brewtank create "$TANK_NAME" >/dev/null || { echo "create failed"; exit 1; }
PREFIX="$(brewtank which "$TANK_NAME")"
pass "created at $PREFIX"
check "prefix is no longer than the global prefix" \
  "yes" "$([ ${#PREFIX} -le ${#GLOBAL_PREFIX} ] && echo yes || echo no)"
check "Library is a symlink to the global Library" \
  "$GLOBAL_PREFIX/Library" "$(readlink "$PREFIX/Library")"
check "bin/brew is a regular file, not a symlink" \
  "no" "$([ -L "$PREFIX/bin/brew" ] && echo yes || echo no)"

say "activate"
brewtank activate "$TANK_NAME"
check "BREWTANK_ACTIVE is set"      "$TANK_NAME" "${BREWTANK_ACTIVE-}"
check "brew resolves to the tank"   "$PREFIX/bin/brew" "$(command -v brew)"
check "brew --prefix is the tank"   "$PREFIX" "$(brew --prefix)"
check "brew --cellar is the tank"   "$PREFIX/Cellar" "$(brew --cellar)"
check "HOMEBREW_PREFIX is exported" "$PREFIX" "${HOMEBREW_PREFIX-}"
check_contains "MANPATH points into the tank" "$PREFIX/share/man" "${MANPATH-}"
check_contains "PS1 carries the tank name" "($TANK_NAME)" "$PS1"

say "the guard holds"
UPDATE_OUT="$(brew update 2>&1)"
check "brew update is refused" "1" "$?"
check_contains "and says why" "disabled inside a tank" "$UPDATE_OUT"

say "install into the tank"
brew install wget >/dev/null 2>&1 || { echo "install failed"; FAILED=$((FAILED + 1)); }
check "wget resolves to the tank"  "$PREFIX/bin/wget" "$(command -v wget)"
check "wget is in the tank Cellar" "yes" "$([ -d "$PREFIX/Cellar/wget" ] && echo yes || echo no)"
check "wget runs"                  "0" "$(wget --version >/dev/null 2>&1; echo $?)"
check "no dylib leaks to the global prefix" \
  "0" "$(otool -L "$PREFIX"/Cellar/wget/*/bin/wget | grep -c "$GLOBAL_PREFIX" || true)"
check "Library survived the install" \
  "$GLOBAL_PREFIX/Library" "$(readlink "$PREFIX/Library")"

say "freeze and restore"
FREEZE_FILE="${TMPDIR:-/tmp}/brewtank-e2e.Brewfile"
rm -f "$FREEZE_FILE"
brewtank freeze -o "$FREEZE_FILE" >/dev/null
check_contains "the Brewfile lists the installed formula" 'brew "wget"' "$(cat "$FREEZE_FILE")"
check "dependencies are not listed as top-level entries" \
  "0" "$(grep -c 'brew "openssl@3"' "$FREEZE_FILE" || true)"

# Entries a tank cannot hold must be dropped rather than acted on.
printf 'cask "firefox"\nvscode "some.extension"\n' >> "$FREEZE_FILE"
brewtank remove "${TANK_NAME}r" --force >/dev/null 2>&1 || true
brewtank create "${TANK_NAME}r" >/dev/null
RESTORE_OUT="$(brewtank restore "$FREEZE_FILE" --tank "${TANK_NAME}r" 2>&1)"
check "restore succeeds" "0" "$?"
check_contains "casks are skipped" "Skipping 1 'cask' entry" "$RESTORE_OUT"
check_contains "vscode entries are skipped" "Skipping 1 'vscode' entry" "$RESTORE_OUT"
RESTORED_PREFIX="$(brewtank which "${TANK_NAME}r")"
check "the formula landed in the second tank" \
  "yes" "$([ -d "$RESTORED_PREFIX/Cellar/wget" ] && echo yes || echo no)"
check "no cask was staged" \
  "0" "$(ls -1 "$RESTORED_PREFIX/Caskroom" 2>/dev/null | wc -l | tr -d ' ')"
brewtank remove "${TANK_NAME}r" --force >/dev/null
rm -f "$FREEZE_FILE"

say "deactivate"
brewtank deactivate
check "BREWTANK_ACTIVE is cleared" ""              "${BREWTANK_ACTIVE-}"
check "PATH is restored exactly"   "$ORIGINAL_PATH" "$PATH"
check "MANPATH is restored"        "$ORIGINAL_MANPATH" "${MANPATH-<unset>}"
check "PS1 is restored"            "base> "        "$PS1"
check "brew is the global one"     "$GLOBAL_PREFIX/bin/brew" "$(command -v brew)"
check "wget is gone from PATH"     ""              "$(command -v wget || true)"

say "doctor"
DOCTOR_OUT="$(brewtank doctor)"
check "doctor is happy" "0" "$?"
check_contains "doctor lists the tank as ok" "ok   $TANK_NAME" "$DOCTOR_OUT"

say "remove"
REFUSAL="$(brewtank remove "$TANK_NAME" 2>&1)"
check_contains "a non-empty tank is not removed by accident" "still has" "$REFUSAL"
brewtank remove "$TANK_NAME" --force >/dev/null
check "prefix is gone" "no" "$([ -e "$PREFIX" ] && echo yes || echo no)"

say "switching tanks, and strict mode"
brewtank remove "${TANK_NAME}a" --force >/dev/null 2>&1 || true
brewtank remove "${TANK_NAME}b" --force >/dev/null 2>&1 || true
brewtank create "${TANK_NAME}a" --inherit >/dev/null
brewtank create "${TANK_NAME}b" >/dev/null
A="$(brewtank which "${TANK_NAME}a")"
B="$(brewtank which "${TANK_NAME}b")"

brewtank activate "${TANK_NAME}a"
check "an --inherit tank keeps the global Homebrew behind it" \
  "yes" "$(printf '%s' "$PATH" | tr ':' '\n' | grep -q "^$GLOBAL_PREFIX/bin$" && echo yes || echo no)"
# Switching without deactivating first must not stack the two tanks on PATH.
brewtank activate "${TANK_NAME}b"
check "switching leaves the new tank active" "${TANK_NAME}b" "${BREWTANK_ACTIVE-}"
check "the previous tank is off PATH" \
  "0" "$(printf '%s' "$PATH" | tr ':' '\n' | grep -c "^$A" || true)"
check "a default (strict) tank hides the global Homebrew" \
  "0" "$(printf '%s' "$PATH" | tr ':' '\n' | grep -c "^$GLOBAL_PREFIX" || true)"
check "the strict tank is still on PATH" \
  "1" "$(printf '%s' "$PATH" | tr ':' '\n' | grep -c "^$B/bin$" || true)"

brewtank deactivate
check "PATH is restored after a switch" "$ORIGINAL_PATH" "$PATH"
brewtank remove "${TANK_NAME}a" --force >/dev/null
brewtank remove "${TANK_NAME}b" --force >/dev/null

say "the global Homebrew is untouched"
check "global brew --prefix"  "$GLOBAL_PREFIX" "$(/opt/homebrew/bin/brew --prefix)"
check "global formula list"   "$GLOBAL_LIST_BEFORE" "$(/opt/homebrew/bin/brew list --formula | sort)"
check "global repo is clean"  "0" "$(git -C "$GLOBAL_PREFIX" status --porcelain | wc -l | tr -d ' ')"

printf '\n'
if [ "$FAILED" -eq 0 ]; then
  printf 'All checks passed (%s).\n' "$SHELL_KIND"
else
  printf '%s check(s) FAILED (%s).\n' "$FAILED" "$SHELL_KIND"
fi
exit "$FAILED"
