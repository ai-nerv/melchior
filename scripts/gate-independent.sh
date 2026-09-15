#!/bin/sh
# melchior's Rust knows nothing about its siblings.
#
# The four programs in this family talk over pipes and sockets, in a shape `FAMILY.md` writes
# down and `gate-family.sh` holds each binary to. The moment one of them has a Rust dependency on
# another they are one program in two repositories, the contract stops being the interface, and
# "four independent programs that agree on a wire" becomes marketing. It is also why the wire
# types are duplicated on purpose rather than shared -- see `gate-wire.sh`.
#
# **melchior names magi constantly, and every one of those is legitimate.** It is the program magi
# spawns: `MAGI_MELCHIOR_PROJECT`, `MAGI_MELCHIOR_ID` and `MAGI_MELCHIOR_TOKEN` are the variables
# it is started with, `tied::to_magi` is the function that ties its lifetime to the process that
# started it, and half the module documentation explains melchior by what magi does with it. The
# blanket name match the sibling gates use -- any occurrence of a sibling's name in Rust is a
# finding -- reports nineteen things here and not one of them is a dependency.
#
# So what is looked for is the *shape* of a dependency: a path segment beginning with a sibling's
# name, a `use` of one, an `extern crate` of one. `to_magi()` has no path separator,
# `Magi::starting` is a local fixture type and is capitalised, and `crate::magi::` is preceded by
# `::` and so would be a module of melchior's own.
#
# Comments and string literals -- raw ones included -- go first, for the same reason: a doc
# comment that says `use magi::Session` while explaining what melchior will not do, and a refusal
# message quoting a sibling's path, are both prose.
#
# `#[cfg(test)]` bodies are *not* removed, which is where this differs from the sibling gates. A
# crate reached only from a test is still a line in `Cargo.toml`; there is no version of that
# which is not a dependency.
#
# POSIX for the same reason the others are: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-src tests}"

# The siblings, by the name their repository and binary carry.
SIBLINGS='magi casper balthasar'

# What a dependency looks like once the prose is gone.
REACH='(^|[^A-Za-z0-9_:])(magi|casper|balthasar)[A-Za-z0-9_]*::'
REACH="$REACH"'|^[ \t]*(pub[ \t]+)?use[ \t]+(magi|casper|balthasar)'
REACH="$REACH"'|extern[ \t]+crate[ \t]+(magi|casper|balthasar)'

fail=0

# ---- no Rust file reaches into one ------------------------------------------------------------
found=$(
  # shellcheck disable=SC2086
  find $ROOT -name '*.rs' -not -path '*/target/*' 2>/dev/null | sort | while IFS= read -r file; do
    hit=$(awk '
      {
        line = $0
        # A raw string carries on across lines and closes on a quote with the opening hashes
        # after it, so the state is held across records.
        if (raw) {
          p = index(line, rawclose)
          if (p == 0) next
          line = substr(line, p + length(rawclose)); raw = 0
        }
        # An opener is r, hashes, quote, with the r starting a token. The r-quote inside
        # .expect("cbor") is not one, and reading it as one opened a raw string that never
        # closed -- which swallowed the rest of the file and made this gate green on a planted
        # `use magi::Session`.
        while (match(line, /(^|[^A-Za-z0-9_])r#*"/)) {
          open = substr(line, RSTART, RLENGTH); sub(/^[^r]*/, "", open)
          rawclose = "\""; for (i = 3; i <= length(open); i++) rawclose = rawclose "#"
          head = substr(line, 1, RSTART + RLENGTH - 1 - length(open))
          rest = substr(line, RSTART + RLENGTH)
          p = index(rest, rawclose)
          if (p > 0) { line = head substr(rest, p + length(rawclose)) }
          else { line = head; raw = 1; break }
        }
        sub(/\/\/.*$/, "", line)
        gsub(/"([^"\\]|\\.)*"/, "", line)
        print line
      }
    ' "$file" | grep -nE "$REACH" || true)
    [ -n "$hit" ] || continue
    printf '%s: %s\n' "$file" "$(printf '%s' "$hit" | head -1)"
  done
)
if [ -n "$found" ]; then
  echo "gate-independent: melchior's Rust reaches into a sibling:" >&2
  printf '%s\n' "$found" | sed 's/^/  /' >&2
  echo "gate-independent: the family agrees on a wire, not on a crate. See FAMILY.md" >&2
  fail=1
fi

# ---- and the manifest declares none of them ---------------------------------------------------
for name in $SIBLINGS; do
  if grep -qE "^[ \t]*$name[ \t]*=" Cargo.toml; then
    echo "gate-independent: Cargo.toml depends on $name" >&2
    fail=1
  fi
done

# A local path dependency is the other way in, and it does not have to use a sibling's name to be
# one. melchior is a single crate with no workspace under it, so there is nothing on disk it has
# any business depending on: every `path =` here would be pointing out of the repository.
# `"src/` with the quote against it, so the crate's own `[lib]` and `[[bin]]` are not a finding
# and `"../magi/src/..."` still is.
paths=$(grep -n 'path[ \t]*=[ \t]*"' Cargo.toml | grep -v '"src/' || true)
if [ -n "$paths" ]; then
  echo "gate-independent: Cargo.toml has a path dependency, which can only lead out of the repo:" >&2
  printf '%s\n' "$paths" | sed 's/^/  /' >&2
  fail=1
fi

[ "$fail" -eq 0 ] || { echo "gate-independent: failed" >&2; exit 1; }
echo "gate-independent: ok"
