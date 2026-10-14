#!/bin/sh
# The family speaks one way, and a sixth wire cannot invent a sixth spelling.
#
# Five wires had grown five ways to say the same thing: `say`/`heard` on melchior's pipe,
# `to`/`from` on the surface pipe, `message` on the tool-peer pipe, `said` on the streamed
# answer, and a call envelope on the sockets. Nothing anywhere said which was meant, and nothing
# could: each was internally consistent, and each was written by somebody who had not read the
# other four.
#
# **The failure is silent.** Two of those wires exist as byte-identical copies in two
# repositories -- the surface frames, and the streamed answer -- so when two spellings drift
# nothing fails and no test goes red. The surface simply stops being answered, and it presents
# much later as "that tool never draws".
#
# Two shapes only. A **call** is answered and carries a reply; an **event** is not. Every enum
# whose variants cross a process boundary is tagged `event`. The rule is written out in this
# repository's wire module.
#
# A **data** discriminator is a different thing: a tagged enum nested *inside* a message, which
# names a shape rather than an event. Those are allowed their own key, and each is listed below
# with why. The list is the point -- adding to it is a deliberate act, and a new wire that
# invents a tag is not on it.
#
# It also holds every frame to the family's 16 MiB cap, read and written in one module, and every
# runtime directory to the private-dir rule FAMILY.md writes down.
#
# POSIX for the same reason the others are: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-src}"

# Data discriminators, and what each names. Nested inside a message; never a frame of its own.
allowed() {
  case "$1" in
    # a transport, a scope, an action, a body, an endpoint: shapes, named where they are used
    kind) return 0 ;;
    # a provider's own content blocks, which are not this family's to name
    type) return 0 ;;
    # how a tool result is drawn, carried inside the result
    shown) return 0 ;;
    # what a surface was told, carried inside an `answer` event
    answer) return 0 ;;
    # the journal on disk. A file is not a wire, and its records outlive every protocol here
    record) return 0 ;;
    # magi's own UI-to-daemon protocol: CBOR in an envelope, two halves of one program that
    # ship together, deliberately not the family shape -- a peer from another build is turned
    # away at the boundary rather than after its fields have been read
    command) return 0 ;;
    *) return 1 ;;
  esac
}

wrong=""
for file in $(find "$ROOT" -name '*.rs' -not -path '*/target/*' 2>/dev/null | sort); do
  # `#[serde(… tag = "x" …)]`, whatever order the attributes are written in.
  for found in $(grep -o 'tag *= *"[a-z_]*"' "$file" 2>/dev/null | sed 's/.*"\(.*\)"/\1/'); do
    [ "$found" = "event" ] && continue
    allowed "$found" && continue
    wrong="$wrong$file: tag = \"$found\"
"
  done
done

if [ -n "$wrong" ]; then
  echo "gate-wire: a tagged type is neither an \"event\" nor a listed data discriminator:" >&2
  printf '%s' "$wrong" | sed 's/^/  /' >&2
  echo "gate-wire: an event crossing a process boundary is tagged \"event\"; see the wire module" >&2
  echo "gate-wire: a shape nested inside a message goes on this gate's list, with its reason" >&2
  echo "gate-wire: failed" >&2
  exit 1
fi

# A reply says which revision it is written in. Without it a version skew arrives as a missing
# field at the point of use, which reads as the peer being broken rather than as a peer from a
# different build.
if grep -rql 'pub struct Reply' "$ROOT" 2>/dev/null; then
  if ! grep -rql 'pub family: u16' "$ROOT" 2>/dev/null; then
    echo "gate-wire: a Reply carries no \`family\`: a skew would arrive as a missing field" >&2
    echo "gate-wire: failed" >&2
    exit 1
  fi
fi

FRAMING="$ROOT/framing.rs"
PLACES="$ROOT/places.rs"
CLIENT="${GATE_CLIENT:-lua/melchior.lua}"
CORPUS="${GATE_CORPUS:-tests/wire-corpus/cases.json}"
fail=0
bad() { echo "gate-wire: $1" >&2; fail=1; }

# Code with the comments taken out; a file's code before its tests; one item's code, from the
# line matching a pattern to the first line closing at column zero.
code() { awk '{ sub(/\/\/.*$/, ""); print }' "$1"; }
shipped() { code "$1" | sed '/#\[cfg(test)\]/,$d'; }
body() {
  code "$1" | awk -v pat="$2" '
    !inside && $0 ~ pat { inside = 1 }
    inside { print }
    inside && /^}/ { exit }
  '
}

# Every line of shipped code outside `$1` that matches `$2`, as `path:line: text`.
elsewhere() {
  find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort | while IFS= read -r file; do
    [ "$file" = "$1" ] && continue
    shipped "$file" | grep -n "$2" | sed "s|^|$file:|" || true
  done
}

# ---- one frame cap, the family's, on read and on write --------------------------------------
shipped "$FRAMING" | grep -q 'pub const MOST: usize = 1 << 24;' ||
  bad "the frame cap is not the family's 16 MiB"
body "$FRAMING" '^pub fn framed[<(]' | grep -q 'if body\.len() > MOST {' ||
  bad "a frame is written without checking it against the cap"
body "$FRAMING" '^fn expecting[(]' | grep -q 'if len > MOST {' ||
  bad "a frame is read without checking its length against the cap"
body "$FRAMING" '^fn expecting[(]' | grep -q 'if len == 0 {' ||
  bad "a frame of no bytes is read as a frame"
for reader in 'pub fn read_body' 'pub async fn read_wire<' 'pub async fn read_wire_within<'; do
  body "$FRAMING" "^$reader" | grep -q 'expecting(' ||
    bad "$reader reads a frame without checking its length"
done
body "$FRAMING" '^pub fn read_from<' | grep -q 'read_body(' ||
  bad "read_from reads a frame of its own"
for pair in mind/lua/stream.rs:framing::framed mind/lua/stream.rs:framing::read_body \
            asking.rs:framing::write_to asking.rs:framing::read_from \
            serving/talking.rs:framing::read_wire_within serving/talking.rs:framing::framed; do
  file=${pair%%:*}
  call=${pair#*:}
  shipped "$ROOT/$file" | grep -qF "$call(" || bad "$file no longer frames through $call"
done
caps=$(elsewhere "$FRAMING" '1 << 24\|16_777_216\|16777216\|16 \* 1024 \* 1024')
[ -z "$caps" ] || bad "a frame cap is written down outside $FRAMING:
$caps"
lua=$(awk '{ sub(/--.*$/, ""); print }' "$CLIENT")
if printf '%s\n' "$lua" | grep -q ':send(\|:recv(\|string\.char(\|16777216'; then
  bad "$CLIENT frames by hand; the stream's call does that"
fi
printf '%s\n' "$lua" | grep -q 'handle:call(request' || bad "$CLIENT no longer hands a call to the stream"
grep -q '"cap": 16777216' "$CORPUS" || bad "the wire corpus names another cap"

# ---- runtime directories, by the private-dir rule ----------------------------------------------
body "$PLACES" '^pub fn runtime[(]' | grep -q 'is_empty()' ||
  bad "an empty XDG_RUNTIME_DIR is no longer read as unset"
body "$PLACES" '^pub fn runtime[(]' | grep -q 'trusted(' ||
  bad "XDG_RUNTIME_DIR is used without asking whether it is private"
for wanted in 'is_absolute()' '== owner' '0o077'; do
  body "$PLACES" '^pub fn trusted[(]' | grep -qF "$wanted" ||
    bad "a runtime directory is trusted without $wanted"
done
body "$PLACES" '^pub fn base[(]' | grep -qF 'tmp().join(format!("{program}-{}", uid()))' ||
  bad "the fallback is no longer <tmp>/<program>-<uid>"
body "$PLACES" '^pub fn tmp_from[(]' | grep -q 'is_absolute()' ||
  bad "a relative TMPDIR is taken for the system temporary directory"
body "$PLACES" '^pub fn ensure[(]' | grep -q 'mode(0o700)' ||
  bad "a directory is made without mode 0700"
body "$PLACES" '^pub fn ensure[(]' | grep -q 'held(' ||
  bad "a directory is made and not checked afterwards"
for wanted in 'symlink_metadata(' 'is_symlink()' '!= owner' '0o077'; do
  body "$PLACES" '^pub fn held[(]' | grep -qF "$wanted" ||
    bad "a directory is kept without $wanted"
done
read_runtime=$(elsewhere "$PLACES" 'XDG_RUNTIME_DIR')
[ -z "$read_runtime" ] || bad "XDG_RUNTIME_DIR is read outside $PLACES:
$read_runtime"
# Directories made anywhere else, less the two that are not runtime directories: the acknowledged
# manifest beside the config, and the model cache under XDG_CACHE_HOME.
made=$(elsewhere "$PLACES" 'create_dir\|DirBuilder' |
  grep -v "^$ROOT/mind/acknowledged.rs:\|^$ROOT/mind/describing.rs:\|^$ROOT/mind/discovering.rs:" || true)
[ -z "$made" ] || bad "a directory is made outside places::ensure:
$made"

[ "$fail" -eq 0 ] || { echo "gate-wire: failed" >&2; exit 1; }
echo "gate-wire: ok"
