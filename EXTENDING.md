# Extending any of them

One page. It is the same shape in all four programs, so what you learn adding a tool to magi is
what you already know when you add a wire protocol to melchior.

The rule underneath everything here: **the last declaration of a name wins.** Every registrar
replaces by identity, so the load order *is* the precedence, and `after/plugin/` is how you
override something you did not write.

### Model prices

Model `cost` fields are dollars per million tokens. Omitted input/output rates are
unknown; explicit zero rates are free. Omitted cache read/write rates fall back to
the effective input rate. An explicit zero cache rate does not use that fallback.

```lua
cost = {
  input = 3, output = 15, cache_read = 0.3,
  tiers = {
    { input_tokens_above = 200000, input = 6, output = 22.5, cache_read = 0.6 },
  },
}
```

The highest threshold strictly below input plus cached-read plus cached-write
tokens selects rates for the entire request attempt. Missing tier rates inherit
the base schedule, not a preceding tier. Rates must be finite and nonnegative;
thresholds must be unique unsigned integers, with at most 64 tiers. Each retry is
priced independently. A positive provider-reported cost replaces that attempt's
estimate. One-hour cache-write pricing is not separately represented yet.

Cached/generated costs carry versioned rate-presence metadata; do not infer free
service from legacy zero-filled fields. Unknown input/output rates remain absent
from cards and usage estimates, but the existing spend ledger/admission fallback
is unchanged. This is not a new strict unknown-price admission policy.

### Model discovery

Provider declarations can select `discovery = { kind = "bundled" }`,
`{ kind = "compatible", path = "models", auth = "bearer" }`,
`{ kind = "openrouter" }`, or `{ kind = "ollama" }`. Compatible listings use
`{ "data": [...] }`; their header choices are `bearer`, `api-key`, `x-api-key`
and `google-api-key`. An explicit strategy takes precedence over the older
`discover`/`details` fields. Provider headers and credential-origin restrictions
also apply to discovery. A missing required key does not start a network request.
Discovery uses fresh stored OAuth access tokens without logging in or refreshing
tokens. Cloud credential discovery remains unavailable until its auth strategy is
implemented.

OpenRouter uses the public popularity-sorted catalog without a key and the
account-filtered `models/user` catalog with one. The account-filtered order is not
advertised as a popularity ranking. A failed request keeps last-good data or the
declared models; a successful empty listing removes those models. Ollama metadata
requests share their provider's deadline; a metadata timeout keeps the listing.

Refresh runs at most four provider requests concurrently, with five seconds per
provider and a twenty-second overall network budget. Responses and cache entries
are capped at 8 MiB and 4,096 models. Malformed or duplicate model identities reject
the whole listing. Normal loads still fetch missing or expired catalogs; use the
bundled strategy for a strictly offline provider.

Caches live under `$XDG_CACHE_HOME/melchior/models/v2` (or `~/.cache`), partitioned
by a SHA-256 digest of provider, endpoint, strategy, headers and credential identity.
No credential values are written into filenames or normalized model data. Cache
files are private, bounded and atomically replaced; symlinks, hard links, special
files and unsafe directories are refused. Older provider-ID-only caches are not
imported because their account binding cannot be established. Public and
authenticated catalogs never share an entry.

---

## 1. Where the file goes

```
~/.config/<program>/plugin/yours.lua                          alphabetical, each on its own
~/.local/share/<program>/site/pack/*/start/*/plugin/*.lua      installed packages
~/.config/<program>/after/plugin/yours.lua                     the last word
```

`<program>` is `magi`, `casper`, `melchior` or `balthasar`. Make the directory if it is not
there. Nothing else needs telling: there is no registration and no entry point to edit.

**A file under `site/pack/` is one exception.** Your own `plugin/` files run on sight; a package
you fetched runs once you have said it may:

```
<program> acknowledge
```

That records the SHA-256 of every installed file in `<config>/installed.json`. One that is not in
the manifest, or whose digest no longer matches, does not run -- and the program says which and
what to type. Run it after installing or updating anything.

That last part is the point. Requiring an edit to your own `init.lua` to enable somebody's
package makes every package a merge conflict with your configuration.

Working examples live in each repository under `examples/plugin/`. They are run by that
repository's test suite, so an example that does not load fails the build rather than wasting
your afternoon.

**casper's thirteen live in `~/.config/casper/tools.lua`**, like every other sibling's
declarations. `oslo make install` puts them there and overwrites them on every install, so keep
your own in `plugin/` or `after/plugin/`, which it never touches. Editing `tools.lua` changes the
tools on the next call — no rebuild.

To switch one off without writing any Lua, tell casper from magi's config:

```lua
magi.casper = { tools = { dino = { off = true }, birdy = { hidden = true } } }
```

`off` removes it entirely; `hidden` keeps it runnable and takes it out of what the model is shown.
magi passes this on every casper spawn, because casper is one process per call — see `FAMILY.md`.

---

## 2. What you may do in one

A plugin runs in the program's own Lua VM, with the sandbox on. `os.execute` and `io.popen` are
gone; so is `io` entirely, along with `require`, `dofile` and `loadfile`. That is not "a config
must never run anything" — it is that spawning outside the seam happens with nothing checking the
path, nothing asking the person, and nothing bounding what comes back.

What you use instead:

| program | to run something | to touch a file |
|---|---|---|
| magi | `magi.shell(command)` — gated, and only inside a tool's `run` | `magi.fs.write(path, text)`, `magi.fs.ls(dir)` |
| casper | `casper.exec(program, { args })` — a list, so there is no shell in between | its declared tools |
| melchior | — | `melchior.fs` |
| balthasar | — | `balthasar.fs` |

`magi.shell` and `magi.fs.write` go through the same permission gate the built-in tools do: the
person is asked, the answer is remembered, and `magi.confine` applies. Both answer `nil, why`
rather than raising, so a refusal is something to handle rather than something that breaks a turn.

casper, melchior and balthasar refuse a few more things. `debug` is gone, because it reaches past
every other refusal. A precompiled chunk is refused, as a file or as a string handed to `load`:
only source is loaded. `setmetatable` refuses a metatable carrying `__gc`, and `getmetatable`
answers `nil` for a userdata, because no finalizer ever runs. A chunk is stopped once the VM holds
more memory than its ceiling, the way it is stopped once it runs too long. A value handed back as
JSON may not contain itself, nest too deeply or grow past a size limit, and each of those is an
error that says which. balthasar builds its VM from a list of what it keeps rather than taking
things away, so `print`, `warn` and `collectgarbage` are gone there as well (`balthasar.log` is
where a message goes).

A file that raises costs itself and nothing else. Your package failing is reported on stderr and
skipped; the program's own configuration failing is fatal, because a config that will not parse
has not expressed an intention.

---

## 3. The registrars

### magi — a tool

```lua
magi.tool("name", {
  description = "what it does, in the model's terms",
  parameters = { type = "object", properties = { … }, required = { … } },
  transport = { kind = "lua" },       -- the body is the `run` below, in this VM
  needs = "run",                      -- read | write | run | reach; omit if it touches nothing
  run = function(args)                -- return { content = … } or { content = …, is_error = true }
    …
  end,
})
```

`transport` is not optional and there is no default: a tool with a `run` and no transport is
refused at load with "missing field `transport`", because the registry has no way to guess that
the function is the point. The other kinds are declarations rather than code — `command` spawns
one program per call with the arguments in argv, `casper` hands the call to casper.

### magi — a watcher

```lua
magi.watch("name", { run = function(event) … end })
```

Told after the fact, answered with nothing: a watcher cannot change a result and cannot fail one.
Branch on `event.kind` and nothing else — a watcher written when there was one kind of event
assumed every event had a `tool` field, and that is now wrong.

| `kind` | carries |
|---|---|
| `session.opened` | `id`, `resumed` |
| `turn.began` | `model` |
| `turn.ended` | `model`, `took_ms`, `ok` |
| `tool.finished` | `tool`, `arguments`, `is_error` |
| `permission.asked` | `verb`, `about` |
| `permission.answered` | `verb`, `about`, `allowed` |
| `context.compacted` | `dropped`, `kept` |
| `provider.retried` | `mind`, `attempt`, `of`, `delay_ms` |

### casper — a tool

```lua
casper.tool("name", {
  description = "…",
  parameters = { … },
  needs = "read",
  run = function(args)                -- { said = … } or { said = …, failed = true }
    local done = casper.exec("jq", { args.filter, args.path })
    …
  end,
})
```

`said` is what the model reads and should be plain. `shown` is what the person sees and may carry
colour — and a tool never names a colour, it names what its output *means* (`added`, `keyword`,
`path`) and the harness resolves that against its own palette.

A result may also carry `images = { casper.image(path) }`. `casper.image` reads a PNG, JPEG, GIF,
WebP or BMP file through the jail and answers it resized for vision as `{ data, media_type }`, or
`nil` for a file that is not one of those. The base64 stays out of `said`.

### melchior — a provider or a wire protocol

```lua
melchior.provider("id", { name = …, api = …, base_url = …, auth = …, models = { … } })
melchior.api("id", { endpoint = …, headers = …, request = …, on_event = … })
```

`api` names a protocol, not a vendor. `melchior.apis` is the registry and is readable, so a
dialect that differs from a shipped one in one function borrows the other three rather than
forking eight hundred lines.

Dispatch calls `endpoint(base_url, model, routing)` with the prepared request's `tools` in
`routing.tools`; history, images and credentials are not included. Existing two-argument
functions remain valid. Endpoint-conversion failure refuses the request before credentials.
Final destinations still pass the credential-origin validation below.

Models inherit the provider's `api` unless they explicitly name another known protocol. A model's
`base_url` overrides the provider's base URL; the selected protocol still constructs the final
request path. Provider and model `headers` are string-keyed tables, merged case-insensitively in
this order: protocol defaults, provider headers, model headers. Duplicate spellings within one
table are refused. Authentication, signing and HTTP framing headers cannot be overridden this
way. Each configured header table and the final header set are limited to 128 entries and 32 KiB
of names and values. Resolved headers never appear in model cards.

The provider's base-URL origin is the default credential destination. Any different origin,
including one returned by a protocol's `endpoint` function, needs an explicit provider-level
`credential_origins` entry. An origin contains a scheme, host and optional port, not a path,
query, fragment, user information or wildcard. HTTPS is required except for local HTTP. With
no provider base URL, model endpoints also need explicit origin bindings. Validation precedes
credential lookup or refresh, and redirects remain disabled. Discovered or cached models cannot
introduce endpoint overrides or headers.

```lua
melchior.provider("example", {
  name = "Example",
  api = "openai-completions",
  base_url = "https://api.example.com/v1",
  credential_origins = { "https://responses.example.com" },
  auth = { kind = "api-key", vars = { "EXAMPLE_API_KEY" } },
  headers = { ["X-Client"] = "melchior" },
  models = {
    { id = "chat", name = "Chat", context_window = 8192, max_tokens = 1024 },
    { id = "responses", name = "Responses", api = "openai-responses",
      base_url = "https://responses.example.com/v1",
      headers = { ["X-Client"] = "melchior-responses" },
      context_window = 8192, max_tokens = 1024 },
  },
})
```

Omit unused arrays or use `melchior.json.array()` for an explicitly empty array; an untyped empty
Lua table represents an object. Readiness describes local configuration and credential
availability, not account entitlement. An unbound model endpoint is configuration-incomplete.

`melchior.json.decode(text)` retains control-value semantics: null becomes nil and empty
containers are untyped. `melchior.json.decode_value(text)` is the data-preserving alternative:
JSON null uses `melchior.json.null`, arrays and objects retain their shapes, and unsigned
64-bit integers round-trip exactly through `json.encode`. Malformed, nonstring and over-16-MiB
inputs return nil. Existing VM conversion and memory limits still apply. Registrations and
configuration delivery are unchanged.

The Responses family sends `store=false` and requests `reasoning.encrypted_content` for
reasoning models, including requests without an explicit effort. Complete terminal output
can be carried in a versioned opaque turn signature (maximum 1 MiB and 256 output items).
Replay preserves ordered reasoning, assistant text items and function calls, including empty
summaries and text phases. It requires the same provider, model, protocol, resolved endpoint and
credential binding, and unchanged visible text/tool calls. Other shipped protocols do not forward
this signature. Missing bindings in older envelopes retain visible history but do not replay opaque
provider items.

Credential bindings are domain-separated, length-framed SHA-256 digests of the authentication
configuration, credential identity and effective request headers. API-key changes conservatively
invalidate opaque replay, even when both keys belong to the same account. OAuth token exchanges
mint a local random login lineage; refresh preserves that lineage and normalizes the rotating access
header when calculating the binding. Fresh login replaces the lineage. Existing token files without
one use an access-derived identity, promoted on their next successful refresh without changing it.
Organization/project headers and issuer/client configuration remain part of the binding.

The binding is an internal, non-serialized request option, not a model-role field or caller-supplied
credential. Its digest may appear in opaque journal state; raw keys, access tokens, refresh tokens
and routing headers do not. It identifies a credential/login lineage, not a verified provider user
claim or a cryptographic authentication of the journal. Requests are validated without credentials
first, then with the selected binding before refresh traffic, again after the refresh lock is acquired,
and after renewal. Each retry resolves credentials again and rebuilds bound state. Old history is
not edited when replay is refused.
Indexed Responses streams keep separate output slots and content parts. The active slot streams
immediately; later slots wait until preceding items close. Argument/text done events supply only
missing suffixes and must agree with already received bytes. Complete terminal output or saved
item-done snapshots reconstruct missing output without duplicating it. Truncated streams retain
partial calls with a length stop and do not create replay signatures. Conflicting identities,
orphaned fragments and unsupported indexed tool namespaces stop with error, retaining usage.

Reassembly allows 256 output slots and 256 content parts per stream, with separate 1 MiB bounds
on reconstructed text/arguments/identities and encoded done-item snapshots. The final opaque
envelope also has its own 1 MiB bound. Fully unindexed legacy streams retain their old decoding;
they cannot be mixed with indexed fragments. Client tests cover single-byte HTTP chunks, retry
state reset and cancellation/connection closure. The selected-family image scenario additionally
checks terminal-only encrypted reasoning backfill, exact opaque state in journals and next-turn
requests, and same-host session resume across OpenAI/Azure/Codex hostings. Its mutation checks
reject lost items, signatures, origin fields and stateless policy. Native loopback tests additionally
cover changed keys and organization headers, legacy envelopes, OAuth renewal and replacement,
logout, credential replacement between retry attempts and pre-refresh body limits. Provider-specific
account claims and selected-family restart/cancellation qualification remain follow-up work.
No new authentication flow or provider entitlement is implied.

#### Pinned catalog selections

An existing provider registration can select models from the embedded PI 1.6.0 candidate
report. This does not fetch metadata or add other providers:

```lua
melchior.provider("my-groq", {
  name = "My Groq",
  api = "openai-completions",
  base_url = "https://api.groq.com/openai/v1",
  auth = { kind = "api-key", vars = { "GROQ_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = {
    source = "pi-1.6.0",
    provider = "groq",
    models = { "chat:llama-3.3-70b-versatile" },
  },
  models = { { id = "llama-3.3-70b-versatile", name = "My Llama" } },
})
```

Select between 1 and 4,096 operation-qualified identities explicitly; wildcards are not
supported. Unknown sources, providers or models, duplicate selections, classifier selections
and records with unresolved mappings refuse the declaration before discovery or credential
refresh. Review `catalog/generated/pi-1.6.0.json` for source identities and `pending` mappings.
An empty pending list establishes a mapped data shape, not remote availability or entitlement.

Local `models` entries match imported models by raw `id`. Each supplied top-level field replaces
that field; nested tables and arrays are replaced whole, not recursively merged. Distinct local
models append after the selected records, up to 4,096 combined entries. Duplicate local IDs
are refused. Imported API, endpoint, prices and compatibility remain unless explicitly replaced;
capabilities are recomputed after overrides unless locally supplied. The containing registration
owns each model's provider identity. Registering that provider ID again replaces the entire
declaration, including its selection, using the normal plugin and configuration precedence.

Imported endpoint and header overrides still require the usual credential-origin validation.
Changing only the provider base URL does not redirect an imported model's explicit endpoint.
Discovery remains a separate policy: `bundled` makes that provider's listing offline, not other
providers' listings. The generated report is embedded at build time and never downloaded during
startup. Updating its version or digest is an explicit maintenance operation.

The bundled Groq preset currently selects `openai/gpt-oss-120b` and `openai/gpt-oss-20b`;
Cerebras selects `gpt-oss-120b`. These use `GROQ_API_KEY` and `CEREBRAS_API_KEY`, respectively.
NVIDIA selects `stepfun-ai/step-5-preview`, using `NVIDIA_API_KEY`, bundled-only discovery
and the pinned NVCF header. It enforces at most eight images across the request and advertises
no selectable thinking levels: reasoning remains enabled. Unknown prices remain absent;
the spending meter cannot establish a complete dollar cap from unknown rates. A missing
estimate is not free service, and a positive partial estimate is not proof of a complete bill.
Other pinned records are not silently imported. See `catalog/activations.md` for remaining scope.

The bundled xAI preset selects `grok-4.3`, `grok-4.5`, `grok-4.6` and `grok-4.7` over
Responses, with `XAI_API_KEY` and offline discovery. Subscription login is not implemented.
Grok 4.3 maps `off` to `none`; the other three do not advertise an off setting. Grok 4.5's
`xhigh` selection is an alias for `high`. Strict tools are not advertised by this preset.
The pinned context/output limits, image resize profile and tiered token estimates remain
model-specific. Catalog presence and key readiness do not establish service entitlement.

The bundled Meta preset selects the seven pinned Muse Ember/Spark/Wonder chat records
over Responses using `META_API_KEY` and offline discovery. `MODEL_API_KEY` and
`OPENAI_API_KEY` are not aliases. Muse subscription OAuth is not implemented.
Contributor variants have explicit `training-enabled` display labels: Meta may use their
content for training. Choosing a Standard model never falls back to Contributor on retry.
`store=false` controls response storage, not the Contributor data-use terms.
See [Meta's mode documentation](https://dev.meta.ai/docs/muse-spark) before selecting one.

Model `name` is a display label in provider declarations. Model cards retain the bare
route in `name` and the qualified route in `id`; a distinct, nonempty declaration label
is carried as optional `display_name`. Consumers must not use display labels as aliases.

Responses usage can include unsigned `cost_in_usd_ticks` (10,000 ticks per microdollar).
The adapter converts the exact JSON integer to the nearest wire microdollar, including values
above 2^53. A present tick field takes precedence over `cost`; malformed ticks are anomalous
unknown measurements, and absent ticks preserve the existing dollar/estimate path. Positive
reported charges replace the corresponding estimate and survive retries. The legacy tally
still keeps an estimate for zero reported microdollars; measured accounting separately tracks
explicit zero. Raw sub-microdollar precision is not carried in the wire or spending ledger.

#### Input limits

Model declarations accept optional `input_limits` data:

```lua
input_limits = {
  max_request_bytes = 33554432,
  images = {
    max_per_message = 20,
    max_per_request = 100,
    resize = { max_width = 2000, max_height = 2000, max_bytes = 4718592, jpeg_quality = 80 },
  },
}
```

Limits are positive integers; JPEG quality is 1–100. Unknown nested fields refuse the
declaration with a field diagnostic that does not echo values. Omitted limits impose no
additional provider restriction; existing transport and host bounds still apply.

Before credential refresh or HTTP dispatch, Melchior checks the final compact JSON body size,
excluding internal budget/scratch fields. Image counts apply to converted provider messages,
including merged messages and lifted tool images, and to the whole request. The resize profile's
`max_bytes` limits base64 payload bytes, excluding a data-URL prefix. Remote references with no
known payload cannot satisfy that byte check; validation never fetches them. Refusals are
terminal `invalid` errors, with no retry or prompt/credential echo. The same checks apply to
rebuilt retry requests.

Image validation covers Chat Completions, Responses and its Azure/Codex variants, Anthropic,
Gemini/Vertex and PI Messages. Bedrock and Decisions image-limit validation remains unavailable
and fails explicitly when required; request-body byte limits are protocol-independent.

Model cards publish the enforced profile as optional `capabilities.input_limits` in JSON and
CBOR. The model's top-level `input_limits` is authoritative; an explicitly supplied capability
profile must match it. Magi preserves the metadata in choices, refreshes and session snapshots.
Legacy model roles may omit it, and old readers may ignore it.

Magi now normalizes new user/workflow and tool images before history storage, and validates
historical images without changing their bytes. Native tests cover cancellation/reaping during
preparation, tool normalization and history validation, plus refreshed-profile refusal. Selected-family
HTTP retry/tool/resume contracts pass for all eight implemented image adapters. Melchior never
resizes replayed history. The generic `inputLimits.images.resize:history` catalog marker is resolved;
Bedrock's adapter marker and provider-specific restrictions remain. This does not automatically
enable models or qualify cloud/subscription authentication. `oslo make test-provider-input-limits` covers the Melchior checks, including
actual-process refusal before OAuth refresh.

#### Chat Completions tool streams

The bundled adapter assembles function arguments by tool-call index. The first call streams
as it arrives; later calls wait for the response terminal, then emit in index order through
the sequential model-role wire. Arguments retain their original JSON bytes. Delayed identity
fields and identical repeats are accepted; conflicting identities, gaps and malformed final
arguments refuse executable completion. Indexless single calls and ID-addressed continuations
are accepted, but anonymous interleaving is ambiguous and refused. Custom/grammar calls remain
unsupported.

Each response allows 256 calls, 4 KiB per ID/name and 1 MiB of aggregate identity/argument
bytes. A limit violation ends with `error`, never `tool_use`. `length` and
`repetition_truncation` retain attempted calls with a non-executable `length` stop;
filter/error/unknown reasons stop with `error`. A valid `[DONE]` can close a legacy stream;
EOF alone cannot. Usage trailers remain chargeable after a terminal. Retries discard call
assembly but preserve charges from failed attempts.

`oslo make test-provider-completions` exercises the adapter, transport and native model role.
In Magi, the recipe of the same name requires `MELCHIOR_TEST_BINARY` and checks actual host
tool dispatch with isolated HOME/XDG/config and loopback HTTP. That fixture does not install
a durable store, run a UI or establish live-provider entitlement.

#### Reasoning controls

Chat Completions streams preserve the first nonempty reasoning field name (`reasoning`,
`reasoning_content` or `reasoning_text`) as the thinking signature. Replay uses that same
allowlisted field; unsigned legacy thinking keeps `reasoning_content`. Other opaque signatures
cannot create arbitrary JSON keys. Only one field-name signature is emitted per response.

Models may map neutral reasoning controls with `thinking = { low = "low", xhigh = "xhigh",
max = melchior.json.null }`. A null marks a control unsupported; an omitted ordinary level
retains legacy mapping. `xhigh` needs an explicit mapping or control declaration and a
supporting adapter, except reviewed Google model profiles with native `xhigh` support.
Budget-only adapters do not acquire native `xhigh` effort support from
the name; an explicit mapping to an existing supported budget remains possible.

`capabilities.thinking` preserves provider-native metadata. Optional
`capabilities.thinking_levels` restricts neutral controls, with
`melchior.json.array({})` meaning none. Model cards expose the effective control list derived
from these restrictions and the model's mappings. Discovery honors published effort lists;
mandatory reasoning maps `off` to the smallest recognized published effort, never an
invented level when an explicit list has no recognized effort. Unsupported request options
remain omitted, as with existing unsupported mappings; listing a control is not proof of
remote account entitlement.

Anthropic-compatible models may select `compat.anthropic_thinking = "budget"` (the
default), `"adaptive"` (on/off), or `"adaptive_effort"` (adaptive plus mapped
`output_config.effort`). Mode selection does not invent supported levels: use explicit
thinking maps and null entries. MiniMax M3 exposes `off` and `minimal` as its two states;
`minimal` means enabled, not a distinct token budget. Flash exposes five effort levels
and no disabled state. Unsupported request controls remain omitted.

`compat.anthropic_replay = true` retains complete ordered assistant blocks in a bounded
opaque thinking signature. Replay requires the same provider, model, resolved endpoint,
credential/header lineage and visible text/thinking/tool projection. Changed identities
drop only the opaque replay; visible text and tool calls remain. Successful complete
streams publish at most one envelope, capped at 256 blocks and 1 MiB. Incomplete,
malformed or length-limited streams publish none. This is credential-lineage binding,
not authentication of an imported journal or proof of remote account identity.
`compat.allow_empty_signature` defaults false and independently opts into unsigned
thinking; unrelated providers keep their existing handling. Replay scratch crosses the
Lua metadata bridge as encoded JSON to preserve null, empty containers and large integers.

`compat.anthropic_schema` defaults to `"forced_tool"`, preserving the synthetic answer
tool. `"output_config"` instead writes `output_config.format` with the JSON schema,
retains ordinary tools, and merges adaptive effort into the same output configuration.
`compat.supports_forced_tool_choice = false` refuses only the forced-tool schema path
before credentials or HTTP; it does not disable native schemas or automatic tools.
Native schemas do not imply strict-tool support. Required strict tools still need their
own supported dialect; the forced-tool answer path still cannot retain them.

Google Generative AI and Vertex retain ordered response parts in a versioned
`melchior.google.v1:` opaque turn signature. Text, thought, function-call and empty
signature carriers keep their individual signatures and order; signed parts are never
merged. The existing neutral signature field carries this state through the host and
journal without a Google-specific wire type. Replay requires the same provider, model,
API, resolved endpoint and credential/header lineage, plus unchanged visible text,
thinking and calls. Imported raw signatures without that binding are not forwarded;
visible thinking is retained as ordinary text when a complete bound replay is unavailable.

Google replay is capped at 8,192 parts and 1 MiB of encoded state. Exceeding a bound,
malformed/unsupported content or an unsuccessful finish produces an error stop rather
than truncated signed state. Length-limited streams keep a length stop without a replay
signature. One envelope is emitted on a successful terminal; later usage remains
accounted without a duplicate terminal. Google scratch crosses the Lua bridge as encoded
JSON, preserving nulls, container shapes and unsigned arguments. These limits are local
safety bounds, not a claim that the remote API restricts signature lengths to 1 MiB.

Google output usage sums the cumulative maxima of `candidatesTokenCount` and
`thoughtsTokenCount` per attempt, including late updates after a terminal. Both are
validated against their nonnegative int32 wire domain before adding. Invalid or stale
components increment anomalies without discarding valid paid counters. Missing optional
thoughts default to zero; a malformed, previously unknown thought count prevents an exact
output measurement until a valid observation arrives. Thought-only updates retain an
output lower bound without inventing an observed candidate count. Retries reset component
snapshots while attempt accounting retains their separate charges.

`compat.google_thinking` accepts `"auto"` (default), `"budget"` or `"level"` and inherits
fieldwise; an explicit model-local `"auto"` resets an inherited mode. Automatic profiles
select model-specific Gemini 2.5 budgets or discrete Gemini 3/4 and Gemma 4 controls.
Unknown model IDs retain legacy dynamic-budget behavior; custom deployments can select
an explicit mode and declare mappings/restrictions. Only the Google APIs use this field.

Google cards and requests use one resolver. Null mappings and empty control lists remain
unsupported. Valid uppercase maps are normalized; requests emit native uppercase levels.
An unavailable request level clamps to the next higher supported neutral control, then to
the next lower one. Unspecified controls use the same resolution as `off`. Mandatory
reasoning models therefore use their lowest available control rather than an invalid zero
budget; neither `off` nor an omitted selection requests thought summaries. Nonreasoning
models and explicit empty menus emit no thinking configuration. The output reservation
accounts for resolved active reasoning even when the original selection was absent/off.

Gemini 2.5 Pro uses budgets 128/2,048/8,192/32,768 for minimal/low/medium/high and cannot
disable reasoning. Flash uses 128/2,048/8,192/24,576; Flash-Lite uses
512/2,048/8,192/24,576. Both Flash profiles support zero for `off`. These neutral mappings
are selected budgets, not an exhaustive description of the API's accepted numeric range.
Gemma 4 exposes minimal/high; Hydrogen and Neon expose low/medium/high/xhigh. Other known
profiles retain their narrower menus. No request contains both `thinkingBudget` and
`thinkingLevel`. See [Google's control tables](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/thinking),
[Neon's accepted levels](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/guides/gemini-4-neon)
and [Gemma's binary behavior](https://gemma.ai/deployment/gemini-api/).

`oslo make test-provider-google-thinking` checks native menus, mappings, overrides and
retry bodies. Magi's recipe of the same name checks neutral model cards and actual host
requests with `MELCHIOR_TEST_BINARY`; neither recipe makes live service requests.

`compat.supports_cache_control_on_tools` defaults true and controls only the last-tool
cache marker. System/history markers follow the existing prompt-cache policy.
`compat.supports_eager_tool_input_streaming` defaults false; true emits
`eager_input_streaming=true` on tools. Both inherit fieldwise, including explicit false.
The ephemeral cache policy never emits a long-retention TTL. These fields are specific
to Anthropic Messages; Chat/Responses requests remain unchanged.

Fireworks documents native schema output, forbids forced tool selection with enabled
thinking, and reports authoritative usage on the final message delta. Both Anthropic
streaming paths now merge supplied final input/output/cache counters without erasing
omitted or malformed fields or relaxing monotonic paid-usage accounting.
[Fireworks compatibility](https://docs.fireworks.ai/tools-sdks/anthropic-compatibility.md)
`oslo make test-provider-fireworks` exercises all 27 bundled records in isolated local
fixtures, including images and tool-history replay; it does not prove live acceptance.
The earlier `test-provider-fireworks-foundation` recipe remains an alias.
`oslo make test-provider-anthropic` covers controls, replay, malformed/partial streams,
transport cancellation/retry and native credential/size boundaries.

`compat.requires_reasoning_content_on_assistant_messages = true` fills an absent
`reasoning_content` with an empty string on Chat Completions assistant messages for
reasoning models. Existing thinking fields remain intact, including across tool results
and later user turns. Other roles and nonreasoning models are unchanged. Inline thinking
and allowlisted alternative reasoning fields retain their existing encodings; opaque
signatures never become request keys. The default is false and model overrides inherit
fieldwise, including explicit false. This is visible reasoning, not opaque signed replay.

`compat.supports_json_schema = false` refuses structured-answer schemas on Chat Completions,
Anthropic Messages and the Responses family before credentials or HTTP. It is independent of strict function
tools: Xiaomi accepts strict tools but documents only text/JSON-object answer formats.
The default preserves the existing schema request path; it is not a newly verified claim
about undeclared services. Explicit true overrides false. These flags do not advertise
new wire capabilities. `oslo make test-provider-presets` includes their contract tests.

`compat.zai_tool_stream = true` adds `tool_stream=true` to Chat Completions requests
only when tools are present. Its default is false; model/provider overrides inherit
fieldwise, including false. Catalog imports map PI's `zaiToolStream` only for a capable
adapter (an explicit false is harmless elsewhere).

`compat.thinking_format = "baseten"` enables typed Chat template arguments for
reasoning models. `compat.chat_template_args` accepts only `enable_thinking`, whose
value is a boolean or `{ ["$var"] = "thinking.enabled", omitWhenOff = true }`.
`omitWhenOff` is optional and defaults to false. Unknown keys/variables, nulls and
sequence-shaped arguments are refused. A model's argument object replaces the inherited
object, including an empty object; explicit false survives. Use `melchior.json.object()`
for an explicit empty Lua object.

The variable uses the requested neutral level, not its provider alias: `off -> none`
is still disabled. An absent level resolves to disabled for this format; an unsupported
explicit level omits controls. Literal booleans stay literal. When enabled by compatibility,
`reasoning_effort` carries the mapped level or the model's off mapping. No empty template
object or internal toggle reaches the provider. Other formats/nonreasoning models do not
emit these arguments. The bundled Baseten selection preserves the pinned snapshot rather
than inferring today's service defaults; see `catalog/activations.md` for known drift.

`thinking_format = "qwen"` emits `enable_thinking=true` for supported non-off levels
and `false` for an effective `off`. With `supports_reasoning_effort=true`, enabled
requests also carry the mapped `reasoning_effort`; explicitly declared `xhigh` is
available only with that flag. Binary-only models never gain an effort field. Explicit
null mappings and an unspecified level omit both controls. An `off -> low` minimum
mapping enables reasoning at `low`, rather than disabling it. No thinking-token budget
is inferred. The four bundled Qwen token-plan presets explicitly use
`max_tokens_field="max_completion_tokens"` and preserve regional primary/fallback key
order. `oslo make test-provider-qwen` covers all 66 selections; the foundation recipe
remains an alias. Local fixtures do not establish live acceptance or exact PI defaults.

`thinking_format = "together"` emits `reasoning.enabled` for supported effective levels.
An effective `off` disables it; other levels enable it and send the mapped `reasoning_effort`
only with `supports_reasoning_effort=true`. Unspecified/null levels and nonreasoning models
emit neither field. Explicit `xhigh` also requires that support flag. The bundled Together
GPT-OSS record remains `openai`, without a hybrid toggle. `oslo make test-provider-together`
covers all 16 selections, both response reasoning fields, tool/image replay and lifecycle
contracts. No live acceptance or equivalence with PI's unspecified-level behavior is implied.

`thinking_format = "ant-ling"` emits `reasoning.effort` only when the selected level
has an explicit nonempty mapping and the model supports reasoning. Missing, null and
unspecified levels omit the control. Capability lists can restrict but cannot invent
mappings. An explicit `xhigh` mapping does not require top-level reasoning-effort support;
`reasoning_effort` is never emitted by this dialect. The three bundled records use
`ANT_LING_API_KEY`; only Ring advertises `high`/`xhigh`. The Ling models advertise no
effort choices. `oslo make test-provider-ant-ling` covers the adapter, compiler and all
three native models. Local checks do not establish remote strict-schema enforcement.

The bundled Hugging Face preset imports all 87 pinned Chat records using `HF_TOKEN`.
The catalog compiler applies reviewed PI defaults only to the `huggingface` /
`openai-completions` / `https://router.huggingface.co/v1` route: reasoning-effort and
store support, with `max_completion_tokens`. Explicit model compatibility values override
those defaults. Requests omit unspecified or explicitly null effort mappings; nonreasoning
models never gain an effort field. Unknown pinned zero prices stay unknown, not free.
`oslo make test-provider-huggingface` exercises all records and native lifecycle contracts.
These are pinned offline contracts, not proof of remote cap aliases, schema enforcement,
provider-suffix routing, entitlement or equivalence with PI's unspecified-level behavior.

`thinking_format = "zai"` enables preserved thinking (`clear_thinking=false`) for
reasoning models, including requests without a selected effort. A mapped `off` sends
`thinking.type=disabled`; other supported mappings also send `reasoning_effort` when
declared. GLM-5.2's `off -> none` is distinct from GLM-5.3's unsupported `off -> null`.
The latter never sends a disabled request. Historical reasoning and tools retain their
original order; this is visible reasoning, not opaque signed state.

The ZAI presets import seven global and four China models, with independent
`ZAI_API_KEY` / `ZAI_CODING_CN_API_KEY` credentials and bundled offline discovery.
The old bundled `zai/glm-4.6` entry is replaced without an alias; select a pinned ID or
declare a custom model explicitly. The existing global endpoint and key are unchanged.
Binary models expose off/high; GLM-5.2 exposes off/high/max and GLM-5.3 low/high/max.
Highspeed records with unknown rates do not advertise free usage.

PI's ZAI records claim strict tools, but the reviewed official API schema does not
establish that guarantee. The raw snapshot remains intact; the explicit runtime overlay
disables strict guarantees until verified. Preferred tools keep their original schema;
required tools refuse before credentials. JSON-object output is not JSON-schema answering.
Catalog `models[].compat` overrides replace the imported object, so these presets restate
all six compatibility fields rather than erasing unrelated pinned behavior.

`oslo make test-provider-zai` checks its exact test inventory, compiler mappings, isolated
native regional requests, history replay, controls, images, usage and pre-auth refusals.
Coding Plan eligibility and remote strict-tool acceptance remain unqualified; in particular,
the China service documents an approved-client restriction. Do not impersonate another
client or use a live account until permitted. Sources and remaining scope are recorded in
`catalog/activations.md` and Magi's `PROVIDERS.md`.

The Moonshot global/China and Kimi Coding presets select six pinned Chat models each.
Both Moonshot regions use `MOONSHOT_API_KEY`; the subscription service uses `KIMI_API_KEY`.
These variables are not interchangeable. Region choice is explicit, with no cross-region
retry. Imported model URLs override provider defaults: a regional endpoint override must
also change every selected model's `base_url`. The client identifies itself as `melchior`.

`compat.moonshot_keep_thinking = true` adds `thinking.keep="all"` only for reasoning
models using the DeepSeek format with thinking not disabled. It defaults to false and
inherits fieldwise, including explicit false. Only the K2.6 runtime overrides enable it;
K2.7 already preserves reasoning and K3 uses `reasoning_effort` without `thinking`.
K2.6 exposes off/high, forced binary models high, and mapped models low/high/max.
Unsupported effort selections are omitted; they never disable a forced-thinking model.
Raw pinned records are unchanged. Structured-answer JSON schemas retain the existing
request path; required strict function tools remain refused by the pinned capability.

`oslo make test-provider-moonshot` verifies an exact eleven-test inventory, including all
18 native selections. Magi's recipe of the same name requires `MELCHIOR_TEST_BINARY` and
`MELCHIOR_PROVIDER_SOURCE` (that checkout's `config/providers.lua`), and verifies all 18
through actual host tool execution. Subscription OAuth, account entitlement and live
endpoint acceptance remain unfinished requirements. Stable cache-key wiring is described below.
Sources and the distinction between native fixtures and remote qualification are recorded
in `catalog/activations.md` and Magi's `PROVIDERS.md`.

### Mistral native requests

`mistral-conversations` uses Mistral's native `/v1/chat/completions` endpoint. It is not
an alias for `openai-completions` or the managed Conversations API. The bundled provider
uses `MISTRAL_API_KEY` and 34 explicitly selected pinned models; discovery is offline.
Adapters only translate model requests and responses. Casper still owns tool execution
and MCP; Balthasar still owns memory.

Native assistant content retains text/thinking chunks. Historical tool calls and results
share deterministic nine-character alphanumeric IDs; already-valid IDs are preserved.
Images use native string-valued `image_url` parts, including tool results. Reasoning
models with mappings use `reasoning_effort`; unmapped reasoning models use `prompt_mode`
when requested. Explicit selections clamp among allowed mapped levels; an omitted
setting uses only the mapped off value, when present. Pinned strict-tool support is
disabled; explicit model `compat.supports_strict_mode=true` enables the existing schema
validator. Required strict tools fail before authentication unless enabled.

Pinned Mistral imports enable `supports_prompt_cache_key`, `send_session_affinity_headers`
and `supports_mid_convo_system_messages`. Model-level explicit false overrides disable
them. The cache key and native `x-affinity` header use the existing credential-scoped
opaque session identity, never the raw caller ID. An explicitly configured `x-affinity`
header takes precedence. `oslo make test-provider-mistral` checks the adapter, pinned
metadata and native subprocesses using isolated local servers, not live credentials.

### Session cache keys and routing affinity

Model cards advertise `capabilities.session_identity=true` to accept optional
`ask.wants.session_id`. It is bounded to 1–256 UTF-8 bytes without control characters;
invalid values fail before credential lookup, OAuth refresh or inference. `ask.about`
is independent and does not affect the key. Missing identity preserves existing requests.

Chat Completions providers may opt into `compat.supports_prompt_cache_key=true` and/or
`compat.send_session_affinity_headers=true`; both default to false and inherit fieldwise,
including explicit false overrides. The first adds `prompt_cache_key` to the body.
The second selects headers using `compat.session_affinity_format`:

| Format | Headers |
|---|---|
| `generic` (default) | `x-client-request-id`, `x-session-affinity` |
| `openai` | Both generic headers plus `session_id` |
| `openrouter` | `x-session-id` only |

Anthropic Messages also supports the affinity opt-in: `openrouter` emits only
`x-session-id`; `generic` and `openai` emit only `x-session-affinity`. Its body never
receives `prompt_cache_key`, even when that Chat-only flag is set. The emitted value is a
64-character SHA-256 digest scoped to the caller identity, provider, protocol, model,
resolved endpoint and credential/header binding. Raw caller IDs are removed before Lua
request conversion. Generated headers are sensitive and count toward merged header
limits. They replace configured values with the same name when enabled. Ordinary retries
and OAuth access/refresh rotation preserve the key; a new API key, login lineage, model,
provider, endpoint or conversation changes it. Logout refuses requests normally.

Moonshot global/China and Kimi Coding opt into the body cache key. Reviewed pinned Mistral
imports also enable the cache key and native `x-affinity` behavior described above.
Long cache-retention controls, other protocols' affinity,
cache-hit performance and remote account eligibility are separate requirements.
Pinned imports materialize PI's detected dialect: `openrouter` for its provider/URL,
otherwise `openai` for Chat or `generic` for Anthropic, unless an explicit format is
supplied. Runtime declarations do not guess from URLs; their default remains `generic`.
Imported records have native request-capture tests; this mechanism alone does not
activate a provider or qualify its remote behavior.
`oslo make test-provider-session` checks an exact 17-test inventory: eight client/adapter,
three compiler and six native cases, including two OAuth refreshes, retry stability and
pre-authentication refusals. Magi's matching recipe covers negotiation, transcript scoping
and nonempty history reconstructed in fresh host test processes; it does not establish
full CLI/Balthasar restart acceptance.

Dynamic tool history uses neutral `system` messages containing one
`{ type = "tool_changes", added = { tool, ... }, removed = { "name", ... } }` block.
`context.tools` supplies the initial state. A removal precedes additions in the same
block; replacement preserves order, while removal followed by addition appends the name.
Only Chat models with both `supports_mid_convo_system_messages = true` and
`supports_mid_convo_tool_additions = true` retain first-time additions at their original
positions. Defaults and explicit false values resolve the current set into top-level tools.
For the additions-only encoding, removals or any repeated declaration fold the tool state
through the last such message into top-level tools; later first-time additions stay inline.
Instruction text remains governed by the separate positional-system flag.

An inline declaration is a complete `system.tools` message with no `content`; text in
the neutral message becomes a separate wire message. Every ask resends its history.
These wire requirements follow the [Kimi dynamic-tool documentation](https://platform.kimi.ai/docs/guide/use-dynamic-tool-loading);
folding follows [PI's pinned transcript implementation](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/utils/transcript.ts).
This adapter capability does not establish availability for every Kimi endpoint or account.

Histories are bounded to 4,096 simultaneously active tools, 16,384 declarations/removals,
and 8 MiB of serialized declarations. Names contain 1–256 bytes without control characters.
Malformed blocks and required unsupported strict schemas refuse before credentials or
OAuth refresh. Strict negotiation includes both top-level and inline declarations; the
endpoint hook receives only their tool projection, never conversation text or credentials.
`oslo make test-provider-tool-history` checks exact inventories and exercises normalization,
compiler mappings, real local HTTP requests, retries and pre-authentication refusals.
Magi's `test-provider-completions` also exercises the independent wire through its broker.
Production session generation/durable replay, named instruction sections and native
Anthropic tool-change messages remain separate unfinished work.

Strict tool arguments are opt-in per tool: `strict = "prefer"` or `strict = "require"`.
Provider/model `compat.supports_strict_mode` enables the OpenAI Chat/Responses encodings;
`compat.supports_strict_tools` enables Anthropic's encoding. Both default to false and
inherit fieldwise, including explicit false overrides. Capability does not enable strict
sampling on an ordinary tool. Cards report the effective `capabilities.strict_tools`;
these flags do not control the separate structured-answer schema.

Required unsupported schemas or adapters fail during request preparation, before OAuth
refresh, credentials or inference dispatch. Preferred unsupported schemas retain their
original form. Strict conversion closes objects and represents optional properties as
required nullable values without modifying the original declaration. Magi's Lua/command
tool declarations carry the same policy and restore optional nulls to omissions/defaults
at validation; required values, explicitly nullable fields and transcript arguments are
preserved. No permission check is removed.

Chat models can opt into `compat.chat_strict_dialect = "deepseek"` alongside
`supports_strict_mode = true`. This dialect validates the documented numeric constraints,
string patterns/formats, arrays and primitive unions. Every object property must already be
required; optional/null schemas are refused rather than assuming undocumented null support.
References and structured unions remain outside the portable normalizer. Unsupported
constraints are never removed: required tools refuse, preferred tools retain their originals.
DeepSeek needs a whole strict tool set. Mixing strict and non-strict tools therefore returns
all preferred tools to their original schemas, or refuses if any tool requires strictness;
ordinary tools are never silently opted in. Effective strict requests use a same-origin
`/beta/chat/completions` path (a terminal `/v1` becomes `/beta`); non-strict requests retain
their configured route. Query-bearing strict base URLs are refused. The independent
`supports_json_schema = false` flag rejects unsupported structured answers. The
`test-provider-deepseek` recipe exercises this contract using isolated local HTTP servers.

The bundled DeepSeek preset selects pinned `deepseek-flash` and `deepseek-v4-pro` instead
of the old `deepseek-chat` and `deepseek-reasoner` entries. Stored selections of the old IDs
are not silently redirected; select a current ID or explicitly provide a custom model.

An Anthropic pre-stream HTTP 400 `invalid_request_error` with a recognized strict-tool-limit
message may retry once with only preferred tools returned to their original schemas.
Required tools remain strict. Other failures, or an error after streaming begins, do not
trigger this downgrade. Retries use the existing attempt/accounting machinery and ceiling.
Request-frame feedback exposes the effective `disable_preferred_strict` option. Magi
remembers it for the same model-role program and qualified model in the owning session,
then sends it in later asks. Required tools remain strict. The state is bounded to 4,096
identities, evicts the oldest when full, and resets on resume/restart; it is neither
journaled nor shared between sessions. Legacy requests omit the false option. Both host
cache observations and provider-policy fingerprints include the effective choice.
Selected-family qualification covers fallback across tool rounds and prompts, required
strictness, optional-null defaults and cache observations against a local HTTP fixture.
Grammar sampling is not implemented.

### balthasar — a source or a section

```lua
balthasar.source("id", { sessions = …, meta = …, line = … })
balthasar.section("id", { weight = …, order = …, tiers = …, where = …, render = … })
```

A source answering `nil` means "not one of ours", which is what keeps a glob that catches a
neighbouring program's file harmless.

---

## 4. What a project file may not do

A `.magi.lua` or `.balthasar.lua` arrives with a checkout, and cloning a repository must not be a
way to run code. Those files may *choose* among what your own configuration offers; they may not
declare a command to run, an endpoint to send text to, or how your transcripts are read.

`magi.trusted = { "/home/you/work" }` in your own configuration lifts that for directories you
vouch for, ancestors included. It is a decision that belongs to a person, made once, in the file
only they can edit.

---

## 5. Talking to another one of them

Every program prints its own client library:

```
balthasar client > /dev/null       # plain Lua, ready to load
melchior client
magi client                        # `lua-api` is the older name for the same thing
casper client                      # a refusal: casper is reached by spawning it with a call
```

Do not keep a copy. The library and the surface it talks to ship together, and a copy that has
fallen behind fails quietly — magi kept 631 lines of balthasar's, and a stale one removed every
memory tool from every session on a machine without saying anything.

Inside magi the siblings' libraries are already loaded and available to a tool as it declares
itself, so this is only for programs outside the family.

---

## 6. Checking it

```
<program> verbs            what it answers, on each of its doors
<program> needs            what a coordinator may set
<program> acknowledge      clear the installed packages, and list what it took
magi doctor                what a session actually composed, without starting one
```

`FAMILY.md` is the contract these implement, and `gate-family.sh` is what holds them to it.

## 7. What is stable

`<program> verbs` reports `surface`, which is the version of everything in section 3. It is **1**
in all four.

It goes up only when something already published stops working. Adding a registrar, a field or an
event does not move it — a file written against 1 keeps running. Renaming one, removing one, or
changing what a field means does. If your plugin cares, read it and refuse rather than failing
halfway:

```lua
-- at the top of your file
local surface = 1  -- what this was written against
```

Stable at surface 1:

| | |
|---|---|
| magi | `magi.tool`, `magi.watch`, `magi.load`, `magi.shell`, `magi.fs`, `magi.json`, `magi.stream`, and the eight event kinds |
| casper | `casper.tool`, `casper.exec`, `casper.paint`, `casper.theme`, and `said` / `shown` / `failed` |
| melchior | `melchior.provider`, `melchior.api`, `melchior.apis`, `melchior.json`, `melchior.fs`, and the four functions a protocol owes |
| balthasar | `balthasar.source`, `balthasar.section`, `balthasar.json`, `balthasar.fs`, `balthasar.load`, and the three functions a source owes |

Not stable, and will change without the number moving: anything not named above, the exact wording
of a description, and the contents of the shipped configuration.
