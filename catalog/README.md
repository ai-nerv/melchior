# Provider catalog inputs

`reference/pi-1.6.0/data/` contains the unmodified provider JSON files from
`@earendil-works/pi-ai@1.6.0`. These are review/generation inputs, **not runtime
provider declarations**. Shipped providers remain in `config/providers.lua`.

Source: `https://registry.npmjs.org/@earendil-works/pi-ai/-/pi-ai-1.6.0.tgz`.
The release archive SHA-256 is
`d68312ec574bc959cb7fee26da4f4876d067aed9bbbe76e876359e60d2294747`;
its npm SHA-512 integrity is
`sha512-9hL+eygFDEiFp//cydeUfmFsKmEqpmvsNPb10Y1W8+m5K9O+sx1hMStfu6nukrBD5sl6mP4RykCYBBNEgsVLfw==`.
The archive was downloaded and verified without installing or executing it.
Per-file digests and counts are pinned in `tests/provider-parity/manifest.json`.

Run `oslo make catalog-generate` to convert the pinned local inputs into
`generated/pi-1.6.0.json`. Run `oslo make test-provider-catalog` to check input
integrity, mappings, counts, operation-qualified identities and exact deterministic
output. The tests run in normal all-target verification without network or upstream
code. The maintenance command uses the existing Rust dependencies, not Node or an
upstream executable, and normal startup does not execute it.

The generated file is a **candidate report, not an activated runtime catalog**.
It retains all 1,779 in-scope identities, API choices, source hashes and 74 explicitly
excluded image identities. Native-shaped candidates carry translated routes,
headers, prices, context tiers, known compatibility fields and thinking mappings.
Every unhandled field is listed in that record's `pending` array and counted in
the summary. The pinned source files retain the original values behind those paths.
All 43 providers remain `activation: "not-reviewed"`; an empty `pending` list
proves a model data shape only, not provider protocol defaults, authentication,
routing safety or runtime support. The shipped Groq, Cerebras and NVIDIA declarations select
four reviewed records explicitly; xAI selects its four pinned chat records through API-key
access. Meta selects seven more, and MiniMax global/China select M3 and M3.1 Flash each.
Xiaomi selects three PAYG records and two each for its China/Amsterdam/Singapore token plans.
DeepSeek adds two, ZAI global/China eleven, and Moonshot global/China plus Kimi Coding
eighteen. Baseten selects all fifteen pinned Chat records, including both GLM 5.2
template-controlled variants. Fireworks selects all 27 mixed Anthropic/Chat records.
Four Qwen token plans select all 66 pinned records with independent provider identities.
Together selects all 16 pinned records; its hybrid dialect adds 15 complete shapes, for
**1,134 complete shapes / 31 declarations / 183 selected imports**. Fourteen focused tests
and actual Magi-host debug/release execution pass, as do full Together local gates.
Hugging Face's later reviewed-default profile adds six complete shapes (**1,140 total**)
and selects all 87 records, for **32 declarations / 270 selected imports**. Eleven focused
compiler/native tests, actual-host debug/release batches and full local verification pass.
The subsequent Ant Ling batch adds three records and complete shapes, for
**1,143 complete shapes / 33 declarations / 273 selected imports**. Its 14 focused tests
and retained debug/release host checks pass, as do full local gates (Magi on a sequential
rerun after a concurrent-runtime check failure). The last complete parity report still
fails with 58 generic smoke passes and 393 pending cases. These selections
do not activate the remaining candidate data or establish full provider qualification.
See [activation batches](activations.md) for scope and qualification evidence.

The subsequent Google thinking resolver makes six more data shapes complete, for
**1,149/1,779 complete shapes**, while declarations/imports remain **33/273**.
All 53 pinned Google-API records pass control conversion and 424 in-process request checks;
native coverage adds 128 loopback requests. This does not activate Google, Vertex or
OpenCode catalog records. Broader host/full qualification is recorded separately in
the activation log.

The repository ignore rules explicitly retain `reference/pi-1.6.0/data/*.json`, including
when a developer has a global `**/data` ignore rule. All 43 source files must be included in
source archives and future commits; the generated candidate file does not replace these inputs.

The Qwen effort foundation brings complete native data shapes to **1,119**. Its subsequent
activation bundles all 66 plan records: **30 declarations / 167 selected imports**.
Fifteen focused tests include 1,092 native loopback requests, 198 pre-auth refusals and
four unknown-model refusals. Actual Magi-host retained debug/release testing passes all
models; full Melchior activation verification also passes. Together work postdates that
source snapshot and is not covered by its full-suite receipt. See
[activation batches](activations.md).

The frozen Fireworks checkpoint had 1,107 complete shapes. The Anthropic
affinity/tool-cache/eager/disabled-long-retention mappings add 25 complete shapes,
including all 16 Anthropic Fireworks records. All 27 Fireworks records are explicitly
bundled: the working tree has 26 declarations and 101 selected imports. Native tests
pass, as do actual Magi-host runs against retained debug/release images and both full
local verification suites. The frozen receipt is in Magi at
`target/provider-foundations/fireworks-20261121T112521Z-xFYlVWg4/`; full provider
qualification, static packaging and live acceptance remain open.
Baseten's two template-bearing Chat records map the typed `thinkingFormat: "baseten"` and
`chatTemplateArgs.enable_thinking` contract. The argument is a boolean or an object
with `$var: "thinking.enabled"` and optional boolean `omitWhenOff`. Other keys,
variables, nulls, sequences and protocols remain pending. Literal false and empty
object overrides survive; raw records are not rewritten. The runtime-only Kimi K3.1
empty effort menu does not change its pinned metadata.

Chat Completions and Anthropic Messages
map the boolean `sendSessionAffinityHeaders` and typed `sessionAffinityFormat`
(`generic`, `openai`, `openrouter`) to explicit runtime controls. Enabled records without
an explicit format preserve PI's detected default: `openrouter` for that provider/URL,
otherwise `openai` for Chat or `generic` for Anthropic. The initial Chat mapping resolved
463 affinity markers and 443 previously incomplete shapes. All 31 Anthropic affinity
flags now map with protocol-specific headers; 38 Responses format flags remain pending.
Unknown or malformed values also remain pending. Native credential-rotation/retry fixtures
verify the shared Chat and Anthropic mechanisms, not every provider's remote routing or
cache behavior. No provider declaration or selected import is added by this conversion.

The boolean-only
`requiresReasoningContentOnAssistantMessages` mapping is qualified for Chat Completions;
other protocols and malformed values remain pending. It resolves 50 field markers and
15 complete shapes without implicitly activating any model. Nine Xiaomi records are
explicitly selected with reviewed provider defaults. Token-plan zero rates remain unknown,
not free; strict function tools and structured answers have independent capabilities.
Anthropic-only boolean
`forceAdaptiveThinking` maps to the tested adaptive-effort/budget modes and
`allowEmptySignature` maps to the unsigned-thinking capability. Other protocols or malformed
values remain pending. This does not enable complete replay or activate providers implicitly;
reviewed declarations opt into replay independently. Explicit
`supportsLongCacheRetention: false` on Chat Completions and Responses is qualified by
those adapters' omission of retention fields. This resolves 107 markers; true, malformed,
and other-protocol values remain pending (16 records). No long-retention implementation or
automatic provider activation is implied. The 1,290 generic
`inputLimits.images.resize:history` markers are resolved: all original limit values survive
conversion, eight implemented adapters pass selected-family HTTP retry/tool/resume contracts,
and Magi's native tests prove cancellation/reaping during preparation, tool normalization and
history validation. Historical images remain unchanged; refreshed profiles are fenced before
admission. Request byte counts, provider-message/request image counts and base64 payload limits
are enforced before authentication or dispatch. Another 163 records retain
`inputLimits.images:adapter` for Bedrock. Native cards and Magi choices/snapshots preserve
the profile as `capabilities.input_limits`. This resolves generic history normalization, not
provider-specific image/header compatibility, cloud credentials, activation or live entitlement.
Strict compatibility flags now survive conversion:
161 records retain `compat.supportsStrictMode:adapter`. The 29 former
`compat.supportsStrictTools:session_fallback` markers are resolved: actual selected-family
requests prove preferred-only fallback across tool rounds and prompts, required strictness,
optional-null validation and cache-policy observations. The retained receipt is
`target/acceptance/run-nqCJ2rDZ/` under the family root; this qualifies the shared behavior,
not every remote provider or account. Unsupported dialects do not advertise strict tool
support just because their input metadata enables it. Neutral
`xhigh` keys, including explicit nulls, are preserved; 143 records still need adapter
support for their mapped effort and retain `thinkingLevelMap.xhigh:adapter` in `pending`.
Generated selectable controls never claim those unsupported efforts. All 303 tier schedules
are converted. Unknown zero rates remain absent rather than being declared free;
an ambiguous tier zero that would overwrite a known base rate is explicitly pending.
Classifier operation selection and Mistral's native model/API shape remain pending.

The reviewed PI code reference is commit
`5a7479a88390d947335a1a6609936897be0a7891` of
`https://github.com/earendil-works/pi`. The npm metadata has no `gitHead`;
the release data is a separately identified snapshot, not proven byte-identical
to generated output at that source commit. `reference/pi-1.6.0/LICENSE` preserves
PI's MIT notice from that reference. PI's catalog aggregates models.dev and
provider metadata; see its `packages/ai/catalog/README.md` for source mappings.

The snapshot has 43 providers, 1,698 chat records, 81 classifier records and 74
image-generation records. Image records remain in the untouched source files
for digest reproducibility but are excluded from the NERV provider-access target.
Copying data does not qualify an endpoint, credential flow, capability or price.

## Import requirements

- Preserve operation identity: `openai/gpt-6-luna` occurs in both chat and
  classifier catalogs. Never overwrite one while flattening records.
- Preserve model-specific API, endpoint, headers, compatibility, thinking map,
  input constraints and prompt-cache semantics. Reject unreviewed mappings.
- Preserve missing prices as unknown. PI's models.dev normalization turns absent
  rates into zero; a zero in this snapshot alone cannot establish free service
  or a known cache rate. Positive rates and explicit `free` flags need distinct
  provenance from those zeros.
- Apply context-price tiers to the complete prompt-token count, including cached
  reads and writes. PI uses strictly greater-than thresholds, not greater-than-or-equal.
- Generate deterministic outputs from these local inputs. Ordinary builds,
  startup and verification must not download a catalog or execute upstream code.
- Keep full source checksums, operation counts and a review report. Updating the
  reference must be an explicit version/digest change with reviewed differences,
  never a fetch from an unpinned moving metadata endpoint.

## Explicit runtime selection

Provider registrations can use `catalog = { source = "pi-1.6.0", provider = "groq",
models = { "chat:llama-3.3-70b-versatile" } }`. See the pinned-selection section in
`../EXTENDING.md` for the complete registration, override and origin-binding rules.
The importer reads only the embedded generated report; it does not run the converter,
fetch upstream data or activate every candidate. Only explicitly selected chat records
with no pending mappings are accepted. Classifier selection remains unimplemented.

Run `oslo make test-provider-imports` for bounded/atomic selection, overlay validation,
actual-process offline listing, captured local HTTP inference, refusal before credential
refresh, and later-plugin replacement. These tests do not establish remote entitlement.
Remaining metadata mappings and provider activation still require separate reviewed
changes with protocol, credential and offline fixtures.
