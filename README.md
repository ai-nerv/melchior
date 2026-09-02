# melchior

One agent talking to another: naming, finding, reaching and refusing.

melchior is the layer a coding agent uses to know about the other agents on the machine — who is
running, how each stands to it, what it may say to them, and what it may not. It knows nothing
about turns, transcripts, models or screens. Those belong to whatever harness is using it.

It was lifted whole out of [magi](https://github.com/termworks/magi), which is why the one rule
worth stating is that **nothing here knows what a harness is**. No dependency on one, and no type
from one: the vocabulary a model can call goes out as *data*, and the harness turns that into
whatever a tool looks like on its side.

## Where sessions live

```
$XDG_RUNTIME_DIR/melchior/
  myproject/                 one directory per project
    alpha-rho                a socket, named by the id and nothing else
    iota-mu
    iota-mu.parent           "alpha-rho": who started it
```

There is no server for the layer as a whole. Every session binds its own socket and answers for
itself, so the directory *is* the registry — a process that died did not get to remove itself
from a list, and a socket nobody answers is discovered on the first call rather than trusted.

Two walls, and no setting opens either past what it says:

- **the project wall** — a session sees only its own project's directory. Not "should not":
  another project's sessions are not refused, they are somewhere this one never lists.
- **the instance wall** — a main is its instance's front door; the subagents behind it are
  private. `melchior.talk` widens this to siblings, or to everything in the project, and nothing
  widens it further.

## Commands

```sh
melchior serve      # bind this session's socket and answer for it
melchior tool       # the vocabulary a model calls, one exec per request
melchior lua-api    # the Lua client library, for redirecting into a config directory
melchior verbs      # what a session answers over its socket
```

`serve` is a **child, not a daemon**: it reads its parent's pipe and exits when that closes, so a
session's socket lives exactly as long as the session does. A layer that outlived its harness
would leave a name in the directory that answers and cannot act, and a sibling sending to it
would be told the message landed.

Two things cross the pipe, one JSON object per line:

```
->  {"say":"doing","busy":true,"working_for":7,"waiting":0}
<-  {"heard":"listening","at":"…/melchior/demo/alpha-rho"}
<-  {"heard":"message","who":"demo/main/beta-nu","sort":"attention","text":"…"}
```

## The wire

Four-byte big-endian length, then a JSON body — the same shape oslo, hexe and aeon speak.

```
->  {"call":"status","from":"demo/main/beta-nu"}
<-  {"ok":true,"n":1,"result":[{"busy":false,"working_for":0,"waiting":0}]}
```

`result` is a **list** and `n` is its length. Settled before anything shipped, because two tools
in one family disagreeing here fail *silently*: a client that unpacks a list reads a bare-value
server as having returned nothing at all, and an empty answer looks like an empty session.

A refusal is a reply, not a dropped connection. A connection serves more than one call. Hanging
up is not a mistake. `verbs` is answered from the first version and before any permission check,
and `client` beside it hands over the library that speaks all this — enough for a sandboxed VM
that cannot shell out to run `melchior lua-api`.

Every call says who is making it. That claim is taken at face value, because everything here is
one user in one directory and a check that cannot be enforced reads like security to whoever
comes along next. What it buys is a *relation*, read off the directory rather than from the
frame. `stop` is the exception: it carries the secret the session was started with, which only
whoever started it ever held.

## Talking to it from Lua

`melchior lua-api` prints a plain-Lua client — framing, encoding, discovery and the verbs — with no
dependencies of its own. Siblings copy it rather than port it.

```lua
local melchior = load(src)(host.stream)
local them = melchior.connect("beta-nu")
print(them.status())
them.tell("the parser is done", "attention")
them:close()
```

## Commands

The build is `.make.lua`, read by [oslo](https://github.com/termworks/oslo). At an oslo prompt in
this directory `make` is enough; anywhere else it is `oslo make`.

```sh
make build
make test
make verify
```
