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

# **Short, and rooted at `/tmp` rather than under whatever `$TMPDIR` already is.** A unix socket
# path may not exceed `SUN_LEN` — 108 bytes — and several tests here bind one inside a scratch
# directory inside this root. On a developer's machine `$TMPDIR` is `/tmp` and nesting is free; on
# the runner it is `/home/runner/work/_temp`, and the same test failed with "path must be shorter
# than SUN_LEN" in the one place the gate was supposed to be proving something. The two subtrees
# are one character each for the same reason.
#
# Isolation comes from the directories being ours, not from where they hang.
base=/tmp
[ -d "$base" ] && [ -w "$base" ] || base="${TMPDIR:-.}"
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
