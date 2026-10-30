<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="misc/melchior-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="misc/melchior.svg">
    <img src="misc/melchior.svg" alt="melchior" width="180">
  </picture>
</p>

<p align="center"><em>One agent talking to another: naming, finding, reaching and refusing.</em></p>

<p align="center">
  <a href="https://claude.ai/code/artifact/cf3ff7f0-1c1d-472a-b01e-d08a854178b1"><strong>How the four fit together</strong></a> —
  a turn end to end, writing a tool, memory both directions, what may run
</p>

melchior is the layer a coding agent uses to know about the other agents on the machine — who is
running, how each stands to it, what it may say to them, and what it may not. It knows nothing
about turns, transcripts, models or screens. Those belong to whatever harness is using it.

It was lifted whole out of [magi](https://github.com/termworks/magi), which is why the one rule
worth stating is that **nothing here knows what a harness is**. No dependency on one, and no type
from one: the vocabulary a model can call goes out as *data*, and the harness turns that into
whatever a tool looks like on its side.

## Where sessions live

```
$XDG_RUNTIME_DIR/melchior/   or <tmp>/melchior-<uid>/ when that is unset, empty or open to others
  myproject/                 one directory per project, 0700
    alpha-rho                a socket, named by the id and nothing else
    alpha-rho.lock           held for as long as alpha-rho listens
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
  private. `melchior.talk` widens this to the rest of the run a session belongs to — siblings,
  and anything else started under the same root — or to everything in the project, and nothing
  widens it further.

## Provider response boundaries

Provider SSE is parsed from bytes: split UTF-8 and CRLF remain intact, including tool arguments.
Malformed UTF-8 fails the attempt through the existing bounded transport-retry policy. The parser
accepts a leading BOM and retains the provider client's EOF-tail delivery; it is not a browser
EventSource reconnection implementation.

Error bodies and OAuth control responses retain at most 64 KiB of decoded text and have a
10-second absolute deadline. OAuth's deadline includes request headers and body. Error-body
failure preserves the HTTP status's retry/refusal class; a complete bounded body can identify
context overflow. A refusal says the `message` of a JSON error body, cut to one line with anything
shaped like a key taken out; any other body, and credential-bearing URLs, are never repeated. A
`Retry-After` or `Retry-After-Ms` is waited out, spread by up to a tenth. No redirect is followed.

A stream is held to 4 MiB a line, 8 MiB an event and 256 MiB in all, and one past those is not
asked again. Successful generations have no total-duration cap; their 180-second inactivity
timeout remains. Cancellation closes the active request.
`oslo make test-streaming` runs parser partitions and local-HTTP boundary fixtures without
provider credentials.

```sh
melchior serve      # bind this session's socket and answer for it
melchior tool       # the vocabulary a model calls, one exec per request
melchior fork       # a name and a secret for a session this one is about to start
melchior lua-api    # the Lua client library, for redirecting into a config directory
melchior verbs      # what a session answers over its socket
```

`serve` is a **child, not a daemon**: it reads its parent's pipe and exits when that closes, so a
session's socket lives exactly as long as the session does. A layer that outlived its harness
would leave a name in the directory that answers and cannot act, and a sibling sending to it
would be told the message landed.

Two things cross the pipe, one JSON object per line:

```
->  {"event":"doing","busy":true,"working_for":7,"waiting":0}
<-  {"event":"listening","at":"…/melchior/demo/alpha-rho"}
<-  {"event":"message","who":"demo/main/beta-nu","sort":"attention","text":"…"}
```

## The wire

Three transports, two shapes, one encoding — written out because it was written out nowhere, and
five wires had grown five ways to say the same thing.

| Transport | When | Framing |
|---|---|---|
| **argv** | a question with an answer and nothing to hold open | one JSON object on stdout |
| **pipe** | a parent and the child it started | newline-delimited JSON, both directions |
| **socket** | anything may knock | four bytes of big-endian length, then JSON |

JSON is on all three. It is the *encoding*, not a transport.

A **call** is answered; an **event** is not:

```
->  {"call":"status","args":[]}
<-  {"ok":true,"family":1,"n":1,"result":[{"busy":false}]}

    {"event":"listening","at":"…"}
```

`result` is a **list** and `n` says how long it is: a sibling that unpacked a bare value would
read an answer as nothing at all. `family` says which revision the reply is written in — a reader
refuses a number it does not know and tolerates one it predates. A refused call is a *reply*, not
a dropped connection.

**The tag key is `event`, everywhere, in both directions**, and `gate-wire` refuses any other.
The failure it prevents is silent: two of these wires exist as byte-identical copies in two
repositories, so when two spellings drift nothing fails and no test goes red — the surface simply
stops being answered.

## Starting a session under another

```
$ melchior fork
{"project":"demo","role":"main","id":"iota-mu","parent":"alpha-rho","token":"…",
 "environment":{"MAGI_MELCHIOR_PROJECT":"demo","MAGI_MELCHIOR_ID":"iota-mu",
                "MAGI_MELCHIOR_PARENT":"alpha-rho"}}
```

**melchior names, the harness spawns.** A layer that started harnesses would have to know what
one is; this hands down a name and a secret, and whoever asked starts the process with the
environment it was given. The secret is `token`, never in that environment: the child's harness
hands it to its own `melchior serve` as the first line on serve's stdin,
`{"event":"token","token":"…"}`. A session started that way comes up a *child*: it writes the note that
makes the tree readable, it is inside the walls `policy::between` draws, and the session that
minted its secret is the only one that can end it.

Poll a spawned agent with `tool --verb=status --who=<id>`; its id is not a task handle.
`tool --verb=task --about=<id@message-id>` tracks a question submitted with `ask`. If `task`
receives a known child's bare id in `about`, with `who` omitted or matching it, it returns
that child's current agent status through the normal policy checks instead of rejecting the
id as a malformed handle. This does not create an `ask` task or infer completion from a report.

`tool --verb=report --message=<text>` stores the caller's report; the body comes only in the
message — no file and no standard input is read for it. One argument holds at most 128 KiB, so a
larger report goes through the socket's `tool` verb, whose frame holds 16 MiB. Submission atomically
replaces the agent's stored report and sends its parent a non-interrupting `report` notification
with a distinct submission revision. Repeating identical text on another turn is a new revision.
Read with `--who=<id>` for pages, or add `--about=json` for the whole body, revision and SHA-256
content digest. Older plain-text report files remain readable. A stored report is not evidence
that the child or its delegated work has finished; check current status separately.

## Talking to it from Lua

`melchior lua-api` prints a plain-Lua client — discovery and the verbs — with no dependencies of
its own. Siblings copy it rather than port it. The host's stream does the framing:
`handle:call(request_table, { deadline_ms = N })` sends one frame, under the 16 MiB cap, and
returns the decoded reply, or `nil` and the reason.

```lua
local melchior = load(src)(host.stream)
local them = melchior.connect("beta-nu")
print(them.status())
them.tell("the parser is done", "attention")
them:close()
```

## Model discovery

`melchior models --refresh --json` bypasses the 24-hour discovery cache for providers with
`discover = true`. Successful listings, including empty ones, replace their cached catalogs.
Failed requests keep the last cached listing. The reply includes `refreshed: true` and a
`failed` list of provider IDs; model cards remain in the ordinary `result` array. Without
`--refresh`, normal cache freshness applies. This lists models; it does not run inference.

A card carries what the provider publishes about its model, and leaves out what it does not:

| field | what it says |
|---|---|
| `vision` | it takes images |
| `pricing` | `input` and `output` in dollars per million tokens, as decimal strings; `"0"` is free, and an unreadable or negative price is left out rather than guessed. Without a listed price, a declared `cost` above zero stands in |
| `capabilities` | `input` and `output` modalities as listed (`text`, `image`, `audio`, …), and `thinking`, the provider's own names for its reasoning levels |
| `popularity_rank` | OpenRouter only: the position in its catalog sorted by weekly popularity, asked for with `sort=most-popular` |

ollama's listing names a model and nothing else, so its vision and thinking levels come from
`/api/show`.

## Images

An `ask` carries images as `image` content (`data` in base64, and `media_type`) and on a
`tool_result` as `images`, each the same pair. Every shipped protocol but `decisions` sends both
as its own image parts. OpenAI Completions takes no image in a tool message, so a tool's images
follow its results as one user message, labelled with the tool that made them. A model without
`vision` is sent a note where each image was; what was asked is not changed, so the images are
there again for the next model that can see them.

## Spending cap

A run — a root session and every session started under it — may spend `spend_cap_usd` US dollars
on models, 10 unless something sets it. `0` or `false` means no cap. Set it in a file of
melchior's own, `melchior.spend_cap_usd = 25`, or have a coordinator send it with `configure`.

Each ask adds what it cost to its run's ledger under `.spent/` in the runtime directory: what the
provider reports, else the model's own rates, and nothing for a model with neither. Attempts that
failed and were tried again are counted too. Once a run has reached its cap, the next ask is
refused with nothing sent, as `failed` with `why: "invalid"`, and the message gives the total, the
cap and how to raise it. The request that crosses the cap still finishes. The ledger goes when the
root session that began the run ends.

Cached reads and writes with no configured price use the ordinary input rate.
An explicit cached rate of `0` remains free. Generated model caches keep versioned
`cost.presence` metadata alongside the numeric rates; older cache arrays remain
readable offline, but their generated zero rates are unknown rather than proof of
free caching. Existing settled ledger entries are not recalculated. Positive
provider-reported cost takes precedence; a reported zero retains the existing
unspecified-cost sentinel and uses configured estimates.

Each `spent.usage` replaces the previous cumulative measurement, including failed
attempts. `cost_micros` contains provider-reported costs; optional
`estimated_micros` contains configured estimates for other, unreported-cost
attempts. Their sum is the resolved charge. A later positive report supersedes
only the same attempt's estimate. Settlement replaces that ask's in-flight ledger
contribution under the lock; duplicate events do not charge again, concurrent asks
remain counted, and settled history is not recalculated. Invalid ledgers refuse
new capped asks rather than appearing empty; saturated totals stay saturated.

Chat Completions recognizes nested `cached_tokens` and `prompt_cache_hit_tokens`
as duplicate counts, using the maximum valid value and bounding it by a supplied
inclusive prompt total. Missing or invalid metrics preserve other valid counts and
response text; invalid metrics are counted in stream state and noted in the log.
`oslo make test-usage` runs adapter, model/cache, retry, spending and real-binary
local-provider coverage; `test-streaming` separately covers transport boundaries.

## JSON in protocol adapters

Request tool arguments, tool parameters, and forced answer schemas preserve JSON
nulls and array/object identity. Unsigned integers above Lua's signed range remain
opaque values that `melchior.json.encode` writes exactly. Optional protocol
metadata and `json.decode` keep their nil-based control semantics.

`melchior.json.array()` and `melchior.json.object()` return explicitly shaped
tables. Passing a table marks and returns that same table; custom metatables are
refused. Use `melchior.json.null` for a stored null. Explicit arrays must contain
consecutive integer keys starting at 1, with nulls rather than holes. Encoding
malformed arrays, cycles, unrelated userdata, or oversized values returns nil.

`oslo make test-json-contract` checks the bridge and actual protocol requests;
`oslo make test-cache-contract` checks the request caching prerequisites.

## Prompt-cache capabilities

Provider or model `compat.prompt_cache` accepts `"none"` or
`"anthropic_ephemeral"`. An omitted field inherits the provider declaration;
otherwise it defaults to none. A model's explicit none disables its provider's
mode. Only the shipped direct Anthropic provider enables the mode by default.
Copilot and undeclared custom endpoints remain off, regardless of protocol or
provider name. Replacing a provider declaration also replaces its capability
declaration; a custom compatible endpoint requires explicit opt-in.

The encoder marks a nonempty system text block and the final selected tool
definition, including a forced answer-schema tool. After role merging, it also
marks the last eligible content block in each of the two latest eligible
messages, with no more than four markers total. Nonempty text, user images and
tool call/result blocks are eligible; thinking blocks and empty systems never
receive direct markers. Earlier content and signed state stay unchanged as the
boundaries advance. No TTL override or extra beta header is sent. Provider
rejection is reported without removing markers or retrying an invalid request.
Offline tests establish encoding and supplied usage accounting, not real cache
hits or savings. Root `oslo make test-cache-contract` captures consecutive actual
family requests, including same-model cache disable, image adaptation and exact
history restoration when capabilities are restored. Its resume case checks
signed native-tool history and images against actual Balthasar replay, with
legal marker boundaries and one declared resume invalidation.
Additional actual-family captures cover deferred-tool activation and Balthasar
rule/drop/summary transitions, preserving surviving tool IDs, arguments, outputs
and signatures against durable entries and the memory owner's selected slots.

Each HTTP attempt can emit an optional `request_frame` observation after request
construction and before sending. Version 1 carries fixed-size identities for the
registered adapter and resolved serialization policy, plus an omitted-image
count. The identities exclude prompts, messages, tools, endpoints and credentials.
They are transient transport metadata, not provider request fields, transcript
entries or usage. They must not release helpers waiting for the first response.

Magi reports reason-only `provider_frame` diagnostics for dialect, policy and
image-adaptation changes. Missing or unsupported observations clear its baseline,
including between retries. An empty reason list does not prove delivery, a cache
hit or arbitrary custom Lua determinism; shipped serializers have separate exact
request tests. Resume discards the in-memory observation baseline.

`oslo make test-cache-contract` exercises actual Lua requests, capability
layering, current catalog declarations, cache reloads, policy observations and
captured local HTTP rejections, consecutive successful HTTP requests and supplied
cache counters. Wire tests cover JSON and CBOR observation bounds.

## Commands

`oslo make build-fake-provider` builds the offline HTTP fixture. Script turns
default to Chat Completions. An Anthropic turn sets
`"dialect":"anthropic-messages"` and uses turn-level `usage` fields `input`,
`output`, `cache_read`, and `cache_write`; its events support text, thinking,
signature, tool_call, finish, pause, drop, and split. Named block/message lifecycle
events are generated by the fixture. Tool arguments remain raw fragments,
including deliberately malformed fault fixtures; that does not imply those
arguments are accepted by a real provider. Chat-only event metrics/usage and
Anthropic-only signatures do not silently cross dialects.

`oslo make test-fake-provider` checks the actual decoder, HTTP split streaming,
malformed sequences, refusals and request bounds. Fixture scripts and request
bodies are capped at the family frame limit, with sixteen concurrent readers and
bounded request reads. Recorded JSONL writes are serialized and capture failures
are reported.

The build is `.make.lua`, read by [oslo](https://github.com/termworks/oslo). At an oslo prompt in
this directory `make` is enough; anywhere else it is `oslo make`.

```sh
make build
make test
make verify
```

## Nix

Each release tag publishes a static build for x86_64 and aarch64 Linux to the `bresilla` cache,
with `config/` and the Lua client under `share/melchior/`.

```sh
nix run github:ai-nerv/melchior -- verbs
nix build github:ai-nerv/melchior
```

The flake names the cache itself; pass `--accept-flake-config` or answer yes when Nix prompts. In
another flake, take `inputs.melchior.packages.${system}.default`, and configure the cache on the
consuming machine: an input's `nixConfig` is not inherited. Add to `/etc/nix/nix.conf`:

```ini
extra-substituters = https://cache.bresilla.com/bresilla
extra-trusted-public-keys = bresilla:zsecaZYIrjCJQS4AuGPmEVU5PRkiMmB/Mds3KaCzTvE=
```

Here, `make nix-build` builds it into `result/`, and `make nix-check` checks the flake.
