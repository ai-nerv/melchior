#!/bin/sh
# The suite leaves nothing behind, in either tree it writes to.
#
# It used to leave a great deal: every test tidied up on its last line, and `assert!` unwinds
# straight past a trailing `remove_dir_all`. So a *failing* test always leaked, and the
# delete-then-create helpers only ever revisited their own name under their own pid, which never
# repeats. Thousands of directories accumulated across two renames of this project and nothing
# said so, because nothing looked.
#
# **Two trees, because melchior writes to two.** `$TMPDIR` is where a scratch goes;
# `$XDG_RUNTIME_DIR/melchior` is where every socket, note and claim goes, and it is the one the
# code under test reaches for by name. A gate that gave the suite a private `$TMPDIR` and left
# `$XDG_RUNTIME_DIR` pointing at the developer's own `/run/user/<uid>` was watching the smaller
# half: the pile-up that `crate::scratch::Project` was written to stop happened in the tree this
# gate could not see, and would happen again unwatched.
#
# Run under both of its own, so the answer is about this run and not about whatever else the
# machine has. That also means it can be trusted on a developer's laptop, which the equivalent
# check against the shared directories could not.
set -eu

# **Short, and `$TMPDIR` when `$TMPDIR` is short.** A unix socket path may not exceed `SUN_LEN` —
# 108 bytes — and several tests here bind one inside a scratch directory inside this root, which
# costs about sixty of them. So a base over forty characters is refused and `/tmp` is used instead:
# on the runner `$TMPDIR` is `/home/runner/work/_temp` and nesting under it failed with "path must
# be shorter than SUN_LEN" in the one place this gate was supposed to be proving something. The two
# subtrees are one character each for the same reason.
#
# Preferring it when it fits is what makes `TMPDIR=...` mean something here. `/tmp` is a fixed-size
# tmpfs with a per-user quota on at least one machine this runs on, `df` reports free space while
# writes fail, and a suite that cannot write reads as a suite that is broken. Somebody who has
# moved their scratch off `/tmp` has already worked that out, and this is the gate that most needs
# to honour it.
#
# Isolation comes from the directories being ours, not from where they hang.
base=${TMPDIR:-}
case "$base" in
  ?*) [ ${#base} -le 40 ] && [ -d "$base" ] && [ -w "$base" ] || base= ;;
esac
[ -n "$base" ] || base=/tmp
[ -d "$base" ] && [ -w "$base" ] || base=.
root=$(mktemp -d "$base/gh-XXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM

# Kept rather than discarded. When this fails it is a test failing, not a leak, and the name of
# the test is the whole answer — a gate that printed only "exit 101" sent the reader back to
# `cargo test` to find out what it already knew.
out=$(mktemp "$base/gh-log-XXXXXX")
trap 'rm -rf "$root" "$out"' EXIT HUP INT TERM

tmp="$root/t"
run="$root/x"
mkdir -p "$tmp" "$run"

if ! TMPDIR="$tmp" XDG_RUNTIME_DIR="$run" cargo test --all-targets --quiet >"$out" 2>&1; then
  cat "$out" >&2
  # A full disk arrives as a wall of failures that name a file and not the disk, and reads as the
  # code being broken. Named here so nobody debugs the code again.
  if grep -qE 'Disk quota exceeded|No space left on device|disk I/O error' "$out"; then
    echo "gate-hermetic: $base is out of room — that is the failure, not the code." >&2
    echo "gate-hermetic: set TMPDIR to somewhere with space (40 characters or fewer)." >&2
    exit 1
  fi
  echo "gate-hermetic: the suite failed; nothing was checked" >&2
  exit 1
fi

# What a *product* is entitled to leave: the `melchior` directory itself, in either tree, and
# nothing inside it. A project directory, a socket, a note or a `given.lua` under there is a test
# that did not clean up after itself.
strays() {
  ls -A "$1" | grep -v '^melchior$' || true
  if [ -d "$1/melchior" ]; then
    ls -A "$1/melchior" | sed "s|^|melchior/|" || true
  fi
}

left=$(strays "$tmp"; strays "$run")

if [ -n "$left" ]; then
  echo "gate-hermetic: the suite left these behind:" >&2
  printf '  %s\n' $left >&2
  echo "gate-hermetic: failed" >&2
  exit 1
fi
echo "gate-hermetic: ok"
