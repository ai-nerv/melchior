#!/bin/sh
# A Lua VM in melchior is a sandboxed one, and there is only one place that builds it.
#
# melchior runs declarations it did not write. `apis.lua` and `providers.lua` are shipped, the
# config directory's copies are the owner's own, and `site/pack/*/start/*/plugin/` is code that
# arrived by being fetched. `Lua::full()` hands a VM the whole standard library — `os.execute`,
# `io.popen`, `dofile` — and a provider description with those can spawn, which would make the
# process boundary a stylistic preference rather than the only way out.
#
# **The defence is one line in one constructor, and that is the risk.** `src/mind/lua/sandbox.rs`
# has tests that prove the removals work; what they cannot prove is that every VM went through
# the constructor that applies them. A second `Lua::full()` written somewhere reasonable — a
# helper, a benchmark, a `configure` path that wanted its own VM — is a full standard library
# with no test anywhere going red. So the count is held at one, and its file is named.
#
# **Comments are stripped before anything is matched, and a call is matched with its paren.**
# Both are bugs this gate had in the sibling it was ported from: the sandbox module's own
# documentation opens by naming `Lua::full()` — it is explaining what it takes away — so the
# count read two with one constructor in the tree; and `grep sandbox::apply` is satisfied by
# `sandbox::apply_REMOVED`, which is how a defence gets deleted under a green gate.
#
# POSIX for the same reason the others are: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-src}"

ENGINE="$ROOT/mind/lua/engine.rs"
SANDBOX="$ROOT/mind/lua/sandbox.rs"
STREAM="$ROOT/mind/lua/stream.rs"
FS="$ROOT/mind/lua/fs.rs"
CONVERT="$ROOT/mind/lua/convert.rs"
PLACES="$ROOT/places.rs"
SETUP="$ROOT/mind/setup.rs"
POLICY="$ROOT/policy.rs"
PLUGINS="$ROOT/mind/plugins.rs"
CATALOG="$ROOT/mind/catalog.rs"

# Code with the comments taken out, which is the only thing any match here looks at.
code() {
  awk '{ sub(/\/\/.*$/, ""); print }' "$1"
}

# A file's code before its tests.
shipped() {
  code "$1" | sed '/#\[cfg(test)\]/,$d'
}

# One item's code, from the line matching a pattern to the first line closing at column zero.
body() {
  code "$1" | awk -v pat="$2" '
    !inside && $0 ~ pat { inside = 1 }
    inside { print }
    inside && /^}/ { exit }
  '
}

# One declaration, from the line naming it to the line that ends it.
decl() {
  code "$1" | awk -v pat="$2" '
    !inside && $0 ~ pat { inside = 1 }
    inside { print }
    inside && /;[[:space:]]*$/ { exit }
  '
}

# The quoted names in what is read from stdin, sorted, on one line.
names() {
  grep -o '"[a-z_]*"' | tr -d '"' | sort | tr '\n' ' ' | sed 's/ $//'
}

fail=0

# ---- one VM, in one place ---------------------------------------------------------------------
built=$(
  find "$ROOT" tests -name '*.rs' -not -path '*/target/*' 2>/dev/null | sort \
  | while IFS= read -r file; do
      code "$file" | grep -n 'Lua::full()\|Lua::new()' | sed "s|^|$file:|" || true
    done
)
count=$(printf '%s' "$built" | grep -c . || true)
if [ "$count" -ne 1 ]; then
  echo "gate-sandbox: a Lua VM is built in $count places; there is one sandbox and it is applied once:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
elif ! printf '%s' "$built" | grep -q "^$ENGINE:"; then
  echo "gate-sandbox: the VM is no longer built in $ENGINE:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
fi

# ---- and that place trims it -------------------------------------------------------------------
if ! code "$ENGINE" | grep -q 'sandbox::apply('; then
  echo "gate-sandbox: $ENGINE builds a VM and does not apply the sandbox to it" >&2
  fail=1
fi

# ---- the removals are still the removals --------------------------------------------------------
# Field by field, because losing one is losing a specific thing: `execute` is the spawn,
# `remove`/`rename`/`tmpname` are writes that go round the `Ops` seam where path checking lives,
# `exit` ends the daemon from inside a config file, and `io` goes wholesale because every
# remaining member of it opens a file — `io.popen` among them.
for gone in execute exit remove rename tmpname; do
  if ! code "$SANDBOX" | grep -q "\"$gone\""; then
    echo "gate-sandbox: os.$gone is no longer removed from the VM" >&2
    fail=1
  fi
done
for gone in io package dofile loadfile require debug; do
  if ! code "$SANDBOX" | grep -q "\"$gone\""; then
    echo "gate-sandbox: the global \`$gone\` is no longer removed from the VM" >&2
    fail=1
  fi
done

# ---- bytecode never enters ---------------------------------------------------------------------
body "$ENGINE" 'pub fn run_within[(]' | grep -q 'sandbox::precompiled(' || {
  echo "gate-sandbox: a chunk is no longer checked for bytecode before it is loaded" >&2
  fail=1
}
body "$SANDBOX" '^pub fn apply[(]' | grep -q 'guard_load(ctx)' || {
  echo "gate-sandbox: \`load\` no longer refuses bytecode" >&2
  fail=1
}
code "$SANDBOX" | grep -q '("string", "dump")' || {
  echo "gate-sandbox: string.dump is no longer removed from the VM" >&2
  fail=1
}

# ---- a value leaves the VM as JSON within bounds -------------------------------------------------
for wanted in 'self.open.insert(table)' 'self.nodes > NODES' 'self.bytes > BYTES' 'depth > DEPTH'; do
  shipped "$CONVERT" | grep -qF "$wanted" || {
    echo "gate-sandbox: converting a Lua value to JSON no longer checks $wanted" >&2
    fail=1
  }
done

# ---- and nothing is queued for a finalizer -----------------------------------------------------
body "$SANDBOX" '^pub fn apply[(]' | grep -q 'guard_setmetatable(ctx)' || {
  echo "gate-sandbox: setmetatable no longer refuses __gc, and nothing here runs finalizers" >&2
  fail=1
}
body "$SANDBOX" '^pub fn apply[(]' | grep -q 'hide_userdata_metatables(ctx)' || {
  echo "gate-sandbox: getmetatable hands out a userdata's metatable again" >&2
  fail=1
}

# ---- every chunk runs on a budget ---------------------------------------------------------------
body "$ENGINE" 'pub fn run_within[(]' | grep -q 'Fuel::with(' || {
  echo "gate-sandbox: chunks are no longer run a slice of fuel at a time" >&2
  fail=1
}
body "$ENGINE" 'pub fn run_within[(]' | grep -q 'spent >= budget' || {
  echo "gate-sandbox: nothing stops a chunk that has spent its budget" >&2
  fail=1
}
body "$ENGINE" 'pub fn new[(]' | grep -q 'set_memory_limit(Some(MEMORY))' || {
  echo "gate-sandbox: the VM is built with no memory ceiling" >&2
  fail=1
}
body "$ENGINE" 'pub fn run_within[(]' | grep -q 'over_memory()' || {
  echo "gate-sandbox: nothing stops a chunk that holds more than the ceiling" >&2
  fail=1
}

# ---- `stream`: the family's sockets, framed here ------------------------------------------------
# A path is dialable by the family's own directory names and a socket's owner, never by the
# runtime directory alone, and the handle carries whole calls rather than raw bytes.
body "$STREAM" '^fn dialable[(]' | grep -q 'family_socket(' || {
  echo "gate-sandbox: melchior.stream no longer asks whether a path is one of the family's sockets" >&2
  fail=1
}
if shipped "$STREAM" | grep -q 'temp_dir()\|XDG_RUNTIME_DIR'; then
  echo "gate-sandbox: melchior.stream decides what is dialable by directory again" >&2
  fail=1
fi
lent=$(shipped "$STREAM" | grep -o '[a-z]*\.set(ctx, "[a-z_]*"' | names)
if [ "$lent" != "call close connect" ]; then
  echo "gate-sandbox: melchior.stream lends [$lent]; it lends connect, call and close" >&2
  fail=1
fi
programs=$(decl "$PLACES" '^pub const FAMILY' | names)
if [ "$programs" != "balthasar casper magi melchior memory" ]; then
  echo "gate-sandbox: the family's socket directories are [$programs]" >&2
  fail=1
fi
for wanted in 'is_socket()' '== owner'; do
  body "$PLACES" '^pub fn family_socket[(]' | grep -qF "$wanted" || {
    echo "gate-sandbox: a dialable socket is no longer checked with $wanted" >&2
    fail=1
  }
done

# ---- `fs`: a listing, and nothing that writes ----------------------------------------------------
lent=$(shipped "$FS" | grep -o 'fs\.set(ctx, "[a-z_]*"' | names)
if [ "$lent" != "ls" ]; then
  echo "gate-sandbox: melchior.fs lends [$lent]; it lends ls alone" >&2
  fail=1
fi
if shipped "$FS" | grep -q 'fs::write\|File::create\|OpenOptions\|remove_\|create_dir\|rename('; then
  echo "gate-sandbox: melchior.fs can change the filesystem" >&2
  fail=1
fi

# ---- `env`: read through os.getenv, never written or lent ---------------------------------------
for file in "$ROOT"/mind/lua/*.rs; do
  if shipped "$file" | grep -q 'set_var(\|remove_var('; then
    echo "gate-sandbox: $file changes the process environment" >&2
    fail=1
  fi
done
if body "$ENGINE" 'fn install[(]' | grep -q '"env"\|"setenv"\|"environ"'; then
  echo "gate-sandbox: the VM is lent the environment as a table of its own" >&2
  fail=1
fi

# ---- `roles` and the privileged list: what a config may assign ----------------------------------
# An allowlist of exactly the settings that choose a model or name what a session is for. The
# reach comes from the environment a session is spawned with, and no decision reads a role.
allowed=$(decl "$SETUP" '^const SETTINGS' | names)
if [ "$allowed" != "discover max_tokens model role thinking" ]; then
  echo "gate-sandbox: a config may assign [$allowed]" >&2
  fail=1
fi
for setting in agent_talk talk reach env roles token secret parent project id confine allow \
               trusted isolation may_spawn mode ask deny agents tools helpers system memory \
               magi casper balthasar melchior; do
  case " $allowed " in
    *" $setting "*)
      echo "gate-sandbox: \`melchior.$setting\` is privileged and a config may set it" >&2
      fail=1 ;;
  esac
done
body "$SETUP" '^pub fn configure_into[(]' | grep -q '!SETTINGS.contains(' || {
  echo "gate-sandbox: a setting nobody declared is no longer refused by name" >&2
  fail=1
}
registrars=$(decl "$ENGINE" '^const REGISTRARS' | names)
if [ "$registrars" != "provider" ]; then
  echo "gate-sandbox: a config may call the registrars [$registrars]" >&2
  fail=1
fi
shipped "$POLICY" | grep -q 'inherited::TALK' || {
  echo "gate-sandbox: the reach is no longer read from the environment a session is spawned with" >&2
  fail=1
}
if shipped "$POLICY" | grep -qi 'role'; then
  echo "gate-sandbox: the reach reads a role, which any session may name for itself" >&2
  fail=1
fi

# ---- and fetched code still has to be let in ----------------------------------------------------
# The other half of the boundary. A file under `site/pack/` arrived from somewhere else and can
# change between one run and the next, so it runs once somebody has said it may.
#
# Both ends are checked: the rule that says which trust needs it, and the call site that asks.
# Either alone passes with the defence gone — a `needs_acknowledging` nobody consults is a
# function, not a gate.
if ! code "$PLUGINS" | grep -q 'Installed'; then
  echo "gate-sandbox: nothing marks an installed package as needing acknowledgement" >&2
  fail=1
fi
if ! code "$CATALOG" | grep -q 'needs_acknowledging()'; then
  echo "gate-sandbox: the loader no longer asks whether a file has been acknowledged" >&2
  echo "gate-sandbox: fetched declarations would then run on sight, which is the whole risk" >&2
  fail=1
fi
if ! code "$CATALOG" | grep -q 'acknowledged::cleared('; then
  echo "gate-sandbox: the loader asks, and nothing checks the answer against the manifest" >&2
  fail=1
fi

[ "$fail" -eq 0 ] || { echo "gate-sandbox: failed" >&2; exit 1; }
echo "gate-sandbox: ok"
