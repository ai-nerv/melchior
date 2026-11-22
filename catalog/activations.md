# Reviewed provider batches

## Mistral native adapter — focused foundation, 2026-11-21

All 34 pinned Mistral chat records are selected through `MISTRAL_API_KEY` and bundled
discovery. `mistral-conversations` names the native `/v1/chat/completions` dialect, not
Mistral's managed Conversations service or OpenAI-compatible endpoint. Provider access
stays in Melchior; Casper owns tools/MCP and Balthasar owns memory. No executor or agent
loop was introduced.

The adapter translates native thinking/text chunks, tool declarations, paired call IDs
and results, image parts, finish reasons and six cached-token spellings. Calls use the
existing bounded stream assembler. History IDs are deterministically remapped to nine
alphanumeric characters with collision avoidance. Reasoning controls preserve pinned
null restrictions, clamp explicit selections among supported levels, and distinguish
omitted settings from off. Ordered System messages retain position; current tool
declarations use the existing neutral history projection. Strict tools remain opt-in.

Seven adapter, two compiler and four native tests pass: 272 in-process request bodies
and 342 loopback HTTP requests across all 34 models, including reasoning, image fallback,
call/result translation, usage, local cancellation and pre-authentication refusal.
Session/image/protocol and Chat Completions regressions pass, as do all 45 catalog tests,
exact generation, Clippy and seven architecture gates. Full native verification passes:
1,033 library and 165 readiness tests in default/all-feature runs, plus hermetic,
native family/model-role and dependency/security gates. Initial adapter-inventory
expectations were updated, and a reproduced `none`/off budget defect was fixed with a
regression. Actual Mistral host/full-family, packaging/UI and live qualification are not claimed.
The worktree now has 34 declarations, 307 selected imports and 1,183 complete candidate
shapes. No installed configuration changed. Logs are in Magi's
`target/provider-foundations/mistral-20261121T152137Z-n4kuGChT/`.

## Google tool-call fidelity — focused foundation, 2026-11-21

Both Google adapters preserve distinct supplied IDs, generate collision-free fallback
IDs and pair call/result IDs for pinned modern model families. Exact argument decoding
preserves nulls, empty containers and unsigned integers. Five adapter tests and one
native test pass; the native case checks 24 loopback requests across both APIs, legacy
and modern models, supplied and generated IDs, and fresh subprocesses. Streaming/usage
regressions, Clippy and all architecture gates pass. No new records are activated.
The subsequent replay foundation preserves signed/ordered parts in the existing opaque
journal carrier, scoped by provider/model/API/endpoint/credential and visible projection.
Seven adapter regressions and actual Magi-host execution pass (two APIs, eight scenarios,
30 loopback requests, eight Echo calls, JSON/CBOR restored history and account/model
isolation). The pending-signature byte-bound fix passes its regressions. Full native
verification (1,012 library / 157 readiness tests in default/all-feature runs) and
sequential full Magi verification pass. Retained debug/release native images pass the
Google host fixture and every previous host batch against extracted public sources.
A subsequent thought-token accounting fix passes six adapter and two native tests
(ten requests), usage/spending regressions, Clippy and architecture gates. The actual host
first reproduced the undercount against a retained pre-fix image; all eight scenarios /
30 requests now pass with exact thought-inclusive usage/prices and retry charges.
Full usage-source native verification passes (1,018 library / 159 readiness tests in
default/all-feature runs), followed by sequential full Magi verification. Both retained
debug/release images pass Google replay and Completions host checks. Discrete reasoning
controls and full CLI/memory restart remain separate work.
This work postdates the frozen
Ant Ling checkpoint. See `target/provider-foundations/google-calls-final-focused.log`
and `google-calls-native-exact.log`; the earlier failing logs remain beside them.

## Google reasoning controls — focused foundation, 2026-11-21

One resolver determines advertised controls and serialized GenerateContent settings for
both Google APIs. Typed `google_thinking` modes distinguish discrete levels from budgets;
automatic profiles retain per-model ranges, mandatory reasoning and real `xhigh` support.
Null/empty restrictions and explicit model overrides survive inheritance and retries.
Seven adapter, two compiler and two native tests pass, including all 53 pinned Google-API
records / 424 request bodies and 128 native loopback requests. All 43 catalog tests and
ten import tests pass. Six formerly pending shapes now convert, for **1,149 complete
shapes**, without changing **33 declarations / 273 selected imports**. Rounded thinking,
usage/replay/call tests, Clippy and all seven native architecture gates pass. Actual Magi
host execution passes 144 requests across two APIs / nine profiles, plus all eight Google
replay/usage scenarios; Magi Clippy and all eleven architecture gates pass. Full native
verification passes (1,025 library / 161 readiness tests in default/all-feature runs,
plus later gates). Sequential full Magi and both retained debug/release host profiles pass
(reasoning, replay and Completions recipes; unchanged retained host image). The first rounded
pipeline stopped at a test-only boolean lint; the first host fixture lost an empty array
through legacy JSON decoding. Both fixtures are repaired without weakening assertions.
Evidence: `../magi/target/provider-foundations/google-thinking-20261121T145151Z-pQ0vmNYU/`.

## Ant Ling — verified offline checkpoint, 2026-11-21

All three pinned Chat records are bundled with `ANT_LING_API_KEY` and exact routes.
Coverage is **33 declarations / 273 selected imports / 1,143 complete candidate shapes**.
The typed `ant-ling` dialect sends nested `reasoning.effort` only from explicit mappings;
Ring advertises `high`/`xhigh`, not a guessed `max`. Missing/null/unspecified controls and
nonreasoning requests emit no reasoning fields. Explicit capability lists restrict but
never supply mappings; top-level effort support remains disabled.

Fourteen focused tests pass (four adapter, two compiler, eight native), with 63 native
loopback requests, nine pre-auth refusals and one unknown-model refusal. Actual Magi-host
debug execution passes all three models / six scenarios / 18 requests / 12 Echo calls.
Both reasoning-field spellings, parallel tools, later users, text-only image fallback,
strict schemas, unknown cache-rate fallback, retries and cancellation are covered.
Retained debug/release host runs each pass this batch and all earlier host regressions.
Full Melchior verification passes with 1,000 library / 156 readiness tests in default
and all-feature profiles, plus hermetic/family/role/dependency gates. Full Magi verification
passes on a sequential rerun; the first shared-runtime hermetic failure is retained and
the gate was not weakened. Complete parity correctly fails: 58 generic passes / 393
pending / zero live requests. Evidence:
`../magi/target/provider-foundations/ant-ling-activation-20261121T131826Z-xxVlQzx3/`.
Subsequent Google ID/argument work is outside that source checkpoint.
No live requests, UI/durable restart or complete provider qualification are claimed.

## Hugging Face — verified offline checkpoint, 2026-11-21

All 87 pinned Chat records convert after resolving reviewed implicit PI defaults before
capability derivation. Three compiler tests pass, including 696 in-process requests and
explicit override/provider/URL/API scoping. Six `xhigh` records become complete, for
**1,140 complete shapes**. All 87 records are explicitly selected, bringing coverage to
**32 declarations / 270 selected imports**. Eight passing native tests cover every model's
requests, history, tools, images, usage, refusals and cancellation, plus representative
retries: 1,659 loopback requests, 261 pre-auth refusals and one unknown-model refusal.
Actual Magi-host debug and release checks each pass all 87 models / 174 scenarios /
522 requests / 348 Echo calls / 35 vision models, plus every earlier host batch. Corrected
fixture expectations preserve exact selection membership, unknown-price semantics and
one-retry success. Global runtime defaults and raw pinned records are unchanged.
Full Magi verification passes. Full Melchior verification passes 996 library / 148 readiness
tests in default/all-feature profiles plus hermetic cleanup and all later gates. Sources
and selected executables match retained snapshots after verification. Complete parity
correctly fails: 56 generic smoke passes / 395 pending / zero live requests. Evidence in
Magi: `target/provider-foundations/huggingface-activation-20261121T125618Z-p4EzwCBH/`.
Full provider, selected-family/UI/static and live qualification remain open.

## Together — verified offline checkpoint, 2026-11-21

All 16 pinned models are bundled with `TOGETHER_API_KEY`, exact IDs and routes, model-local
compatibility and immutable metadata. Coverage is **31 declarations / 183 selected imports /
1,134 complete candidate shapes**. Fifteen models use the new typed Together hybrid toggle;
GPT-OSS remains effort-only. Absence/null controls are preserved; exact PI default behavior
and current remote effort aliases remain separate qualification items.

Fourteen focused tests pass: four adapter, two compiler, eight native. Native coverage is
310 loopback requests (128 efforts, 96 history/images, 32 tool changes, 32 failures, six
retries, 16 cancellations), 48 pre-auth refusals and one unknown-model refusal. History
tests preserve both reasoning field names and parallel tool arguments. Shared bounded
cancellation fixtures retain the existing 66 Qwen cases. Pricing fixtures enforce unknown
cache-rate fallback and whole-micro rounding without changing production accounting.

Magi's actual-host debug run passes 16 models / 32 scenarios / 96 requests / 64 Echo calls
and nine vision models. Full Magi verification and full Melchior verification pass; the
latter runs 996 library / 140 readiness tests in both default/all-feature profiles plus
all later gates. Retained debug/release images pass Together and all earlier host batches.
Complete parity remains incomplete: 54 generic cases pass, 397 pending, zero live requests.
Evidence in Magi: `target/provider-foundations/together-activation-20261121T122802Z-6la1djtL/`.
Hugging Face work postdates this source snapshot. No live inference has been run.

## Qwen plans — verified offline activation, 2026-11-21

All 66 pinned models are bundled under four distinct providers: team/global and
team/China have 20 each; individual/global and individual/China have 13 each.
Coverage is **30 declarations / 167 selected imports / 1,119 complete candidate shapes**.
Exact keys, regional fallback precedence, routes, nullable efforts and unknown prices
remain intact. The reviewed `max_completion_tokens` default matches PI's detected
Qwen behavior; the earlier import fixture's `max_tokens` assumption was corrected.

Fifteen focused tests pass: four adapter, two compiler and nine native. They exercise
1,092 loopback requests, 198 pre-authentication refusals and four unknown-model refusals,
including all default/effort settings, schemas, streamed parallel tools, images and later
users, dynamic tool updates, failures, retries and all-model process cancellation.
Evidence: `target/provider-foundations/qwen-activation-rounded-2.log`. Per-plan retry
fixtures now run concurrently with unchanged policy/assertions; the final focused rerun passes.

Magi's actual-host debug run passes 66 models / 198 requests / 132 Echo calls, with 36
vision models: `target/provider-foundations/qwen-host-debug.log` in Magi. Full Magi
verification passes; an inherited nine-case editor/visual skip count was not measured by
that run (see the additive erratum next to its Magi receipt). Final Melchior verification passes
992 library / 132 readiness tests in both default and all-feature runs, plus all later gates.
Retained debug and release images each pass all 66 models / 198 requests / 132 Echo calls
and Fireworks, Moonshot/Kimi, Baseten and generic host regressions. The complete-parity gate
correctly fails: 52 generic cases pass, 399 remain pending, zero live requests. Evidence in
Magi: `target/provider-foundations/qwen-activation-20261121T120542Z-hyan2iJZ/`.
Together work starts after this verified source snapshot. Exact
PI default equivalence, current remote capabilities and full provider qualification
remain open; no live inference or full family/UI/static acceptance is claimed.

## Qwen plans — previous foundation, 2026-11-21

All 66 pinned records now convert: 20 team/global, 20 team/China, 13 individual/global
and 13 individual/China. Coverage is **1,119 complete candidate shapes**, still **26
declarations / 101 imports**. No Qwen provider is bundled by this foundation.

The request adapter now sends model-supported `reasoning_effort` alongside
`enable_thinking`; explicitly supported `xhigh` is no longer silently filtered.
Binary-only controls, null mappings and unknown plan prices remain intact. Unspecified
levels preserve NERV's existing omission of controls; exact equivalence with PI's
default-disabled Chat helper is not claimed and remains a qualification item.

Nine focused tests pass: four adapter, two compiler and three native. The native cases
exercise **540 loopback requests** and **66 missing-credential refusals**, including all
models/defaults/efforts, schema/tool JSON, usage, offline listing, primary/fallback key
precedence and regional isolation. All 34 catalog tests, exact generation, lint and
architecture gates pass. Evidence: `target/provider-foundations/qwen-native-foundation.log`
and `qwen-foundation-check-2.log`; initial failures and primary sources are retained.

Full Melchior foundation verification passed before activation: 992 library / 126
readiness tests in default/all-feature runs, with all later gates passing. Evidence:
`target/provider-foundations/qwen-foundation-verify.log` (job 86800).
Actual Magi-host regressions pass against the
new debug image for Fireworks, Moonshot/Kimi, Baseten and the generic host scenarios;
Magi's log is `target/provider-foundations/qwen-foundation-host-regressions.log`.
Activation, images/tool-history/error/retry/cancellation, actual Qwen host execution
and provider qualification remain open. The Fireworks frozen
receipt below predates these changes; it is not a passing full-suite receipt for Qwen.

## Fireworks — frozen offline checkpoint, 2026-11-21

All 27 pinned models are bundled under one API-key provider: 16 Anthropic Messages and
11 Chat records. Explicit model APIs and base URLs win over the provider default.
The working tree has **26 declarations**, **101 selected imports** and **1,107 complete
candidate shapes**. This is not full provider qualification or live acceptance.

Seventeen focused tests pass: seven adapter, two compiler and eight native, with
**366 loopback requests** and **124 pre-authentication refusals**. In addition to the
foundation cases, all models exercise image adaptation, two streamed tools with exact
JSON, unsigned reasoning/history across later users, and positional/folded tool updates.
One additional unknown-model refusal and lint/catalog/exact-generation/import checks
also pass. Evidence: `target/provider-foundations/fireworks-activation-qualified.log`.

The actual Magi-host batch passes against retained debug and release images, each: **27 models / 81
requests / 54 actual Echo executions**, with images and later-user history. Its first
image request exposed missing image-worker dispatch in the test example; the example
now invokes the same bounded worker as the CLI, without bypassing validation. Evidence
in Magi: `target/provider-foundations/fireworks-host-debug-2.log`. Existing host,
Moonshot/Kimi and Baseten regressions pass against both images. Full local verification
passes in both checkouts, including Melchior's 988 library / 123 readiness tests in
default and all-feature runs; Magi retains nine editor/visual skips. The complete-parity
gate still correctly fails with 44 generic smoke passes / 407 pending / zero live requests.
The frozen receipt, source archives, final docs, images and logs are retained in Magi at
`target/provider-foundations/fireworks-20261121T112521Z-xFYlVWg4/`. Extracted sources were
hash-checked, not independently rebuilt. No full CLI/family/UI/static/live acceptance
is claimed.

## Fireworks — previous foundation checkpoint, 2026-11-21

All 27 pinned records (16 Anthropic Messages, 11 Chat) convert and are exercised through
isolated import fixtures. At that checkpoint the generated catalog had **1,107 complete
shapes**, with **25 declarations / 74 selected imports**. Fireworks was not bundled yet.

The foundation adds protocol-specific Anthropic affinity, fieldwise tool-cache/eager
controls, native JSON-schema answers alongside reasoning and ordinary tools, and final
input/cache usage updates. Explicit null efforts remain unsupported; Off aliases `none`
and `off` disable adaptive thinking without changing aliases to a real minimum effort.
Native answer schemas do not confer strict-tool guarantees.

Fifteen focused tests pass: seven adapter, two compiler and six native. They cover
**231 loopback requests** and **124 pre-authentication refusals**, including all 27 models
at eight effort/default settings, native schema fidelity, final cache/input usage,
credential/session/endpoint partitioning, retries and malformed terminal streams.
Session (16 tests), catalog (32 tests/exact generation), Anthropic and strict regressions
pass. Evidence: `target/provider-foundations/fireworks-foundation-regressions-2.log`.
The first full run passed 988 library / 121 readiness tests in default and all-feature
runs, then stopped on a redundant struct update in the new test fixture. That lint-only
defect was fixed; `fmt`, `clippy` and the verification rerun completed successfully in
`target/provider-foundations/fireworks-foundation-verify-2.log` (job 87832). Activation
work began near that run's end, so it is not final activation acceptance.
Actual Magi-host regressions pass against debug Melchior: existing 14 scenarios plus
all 18 Moonshot/Kimi and 15 Baseten models, with actual Echo executions. Evidence in
Magi: `target/provider-foundations/fireworks-foundation-host-regressions.log`.
This is not Fireworks host acceptance or a new release/full-Magi-suite receipt.

Provider activation, per-model image/tool-history qualification, actual Magi host runs,
retained release receipts and live acceptance remain ahead. This foundation is not
provider qualification; the full 45-integration / 1,779-record goal remains unchanged.

## Baseten — frozen offline checkpoint, 2026-11-21

That frozen source snapshot has **25 declarations**, **74 selected imports** and
**1,082 complete candidate shapes**. Baseten imports all 15 pinned models, not only
the 13 that were already convertible. The full target remains 45 integrations and
1,779 scoped records; declarations are not qualified-provider counts.

`BASETEN_API_KEY` authenticates the exact pinned Chat endpoint. Bundled discovery is
offline. GLM 5.2 and Fast use typed boolean/`thinking.enabled` templates; explicit
false, empty overrides and off-to-none mapping are preserved. Unsupported types,
variables and protocols are refused/pending, not evaluated as a template language.
Kimi K3.1 exposes no selectable efforts under the pinned no-effort contract.

[Current vendor reasoning controls](https://docs.baseten.co/inference/model-apis/reasoning.md)
differ from the pin for GLM 5.2 Fast and Kimi K3.1, and GPT OSS may still reason at
`none`. We do not silently rewrite the pin or claim remote acceptance. The
[documented affinity header](https://docs.baseten.co/inference/model-apis/pricing-and-limits.md#session-affinity)
uses the bounded scoped session mechanism rather than leaking the host identity.
The documented [function format](https://docs.baseten.co/inference/function-calling.md)
includes strict tools; [structured answers](https://docs.baseten.co/inference/structured-outputs.md)
use JSON schemas. Local validation is not proof of remote enforcement or entitlement.

Thirteen focused tests pass: three adapter, two compiler and eight native. Their
**276 loopback requests**, **75 pre-auth refusals** and one unknown-model refusal
cover every effort/default on every model, images, strictness/schema fidelity,
parallel tools, later-user reasoning/history, dynamic tools, accounting and retry/error
behavior. All 30 catalog tests and exact generation, plus seven importer units and
three native importer tests pass. Evidence:
`target/provider-foundations/baseten-native-qualified.log`. Actual Magi-host execution
against debug and retained-release Melchior passes all 15 models / 45 requests / 30 Echo
executions and the Moonshot/Kimi plus existing host regressions. Both full local
verification suites pass: Melchior has 980 library / 115 readiness tests in default and
all-feature runs; Magi retains nine existing editor/visual skips. The explicit complete
gate correctly exits 1 with 42 generic smoke passes / 409 pending / zero live requests.
Frozen evidence under Magi: `target/provider-foundations/baseten-20261121T103958Z-2erq7QOK/`.
Archives were hash-checked, not rebuilt; images are dynamic. Full provider qualification,
provider-specific cancellation, full-family/UI/static/production-restart/live acceptance
remain open. No live requests or installed configuration changes occurred.

## Session/cache identity foundation — frozen local checkpoint, 2026-11-21

This pre-Baseten candidate report contains **1,080 complete shapes**, with **24 declarations**
and **59 selected imports** unchanged. Chat affinity conversion preserves all 463 enabled
flags and PI's detected header defaults (397 OpenRouter, 66 OpenAI). Other-protocol
affinity and retention policy remain unfinished; this is not automatic activation.

The optional model-role session identity is bounded, negotiated by Magi and further
scoped to provider/model/endpoint/credential lineage by Melchior. Moonshot/Kimi now enable
stable body cache keys. Fifteen focused tests, 28 catalog tests/exact generation and
seven unit/three native import tests pass. Full Melchior verification passes with
977 library and 107 readiness tests in default/all-feature runs and all later gates.
The native Magi host proof passes all 18 Moonshot/Kimi models with stable keys and the
existing regression suite against retained debug and release images. Magi full verification
passes with nine existing editor/visual skips; its focused inventory is four identity units
and four integration tests. The complete-parity negative audit correctly exits 1 with
40 generic smoke passes / 411 pending / zero live requests.

Current evidence and exact limitations: Magi's `PROVIDERS.md`, section
"Session/cache identity foundation", and frozen receipt
`target/provider-foundations/session-20261121T100747Z-PfARstSj/` under Magi.
Source archives and extracted copies are hash-checked, not independently rebuilt. Images
are dynamic; full production restart, selected-family/UI/static/live proof remains open.
The older frozen batch below records its own source snapshot, not these newer changes.

## Moonshot global/China and Kimi Coding — offline API-key batch, 2026-11-21

The working tree now has **24 declarations** (22 PI IDs plus two extras), **59 selected
imports**, and **637 complete candidate shapes**. All 45 integrations and 1,779 scoped
records remain required. Three new declarations select six pinned models each; this does
not establish complete provider qualification or add subscription OAuth.

- [Pinned Moonshot global](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/moonshotai.ts),
  [China](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/moonshotai-cn.ts)
  and [Kimi Coding](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/kimi-coding.ts)
  retain their Chat endpoints and model IDs. Moonshot regions share the default
  `MOONSHOT_API_KEY`; Coding uses `KIMI_API_KEY`. No implicit region switching or key alias
  is added. Bundled discovery is offline; source prices, limits and null mappings survive.
- The [Chat API](https://platform.kimi.ai/docs/api/chat) documents K2.6's optional thinking
  and `keep="all"` for previous reasoning, K2.7's forced/preserved thinking, and K3's
  low/high/max effort without a `thinking` field. Runtime overrides enable preserved K2.6
  reasoning and expose honest binary menus. Unsupported controls are omitted. The same
  reference documents JSON-schema answers and the still-accepted `max_tokens` alias;
  pinned strict function capability stays false. It also calls for a stable Coding Plan
  session cache key, whose neutral host-to-provider wiring remains unfinished.
- [Coding model configuration](https://www.kimi.com/code/docs/en/kimi-code/models.html)
  warns that wrong IDs or disabled thinking can reroute to another model. Exact IDs and
  unsupported-off nulls are preserved; unknown local selections fail without dispatch.
  Subscription tiers determine real access and context capacity. Pinned token prices are
  estimates, not a quota or invoice guarantee.
- [Coding service guidance](https://www.kimi.com/code/docs/en/) permits third-party API-key
  integration, distinguishes China `.com` and overseas `.ai` endpoints and prohibits client
  identity tampering. The pinned `.com` base is retained and requests identify `melchior`.
  An explicit endpoint override must update both provider and imported-model URLs.

Focused verification: **11 tests** pass (two adapter, one compiler, eight native), with
**264 loopback requests**, **36 pre-authentication refusals** and three unknown-model
refusals. Coverage includes all eight effort/default choices on all 18 selections, images,
exact preferred/schema payloads, two parallel tools, later-user reasoning replay, dynamic
declarations/removal, regional accounting, auth/malformed failures and unchanged retries.
The actual Magi host proof passes all **18 models / 54 requests / 36 Echo executions**
against debug, selected release and retained-copy Melchior (counts per run). The retained
release also passes the existing 14-scenario / 28-request host regression plus strict/core
and truncation tests. Both full local verification suites pass; Melchior has 971 library /
101 readiness tests in each default/all-feature run, and 25 catalog tests/exact generation
pass. Magi retains nine existing editor/visual skips. Generic parity is still only 40 smoke
passes / 411 pending / zero live requests, not a count of qualified providers. Native
images are dynamic, not static-release proof. Evidence is retained in Magi at
`target/provider-foundations/moonshot-20261121T093425Z-1B6Vx0nm/`.

The two initial focused failures were fixture expectations: JSON integer versus floating
price representation, and the legitimate pre-request observation frame before an auth
failure. Tests now compare numeric rate bits and require one terminal auth failure while
allowing only the observation frame before it; production behavior was not weakened.

## ZAI global and Coding China — offline API-key batch, 2026-11-21

Current coverage is **21 declarations** (19 PI IDs plus two extras), **41 selected imports**
and **623 complete candidate shapes**. The full target remains 45 integrations and 1,779
records. ZAI already existed at baseline; only `zai-coding-cn` is a new declaration.

Seven global and four China records are imported from the unmodified pinned snapshot.
The global endpoint/key and tool-result-name behavior are retained. The old global
`glm-4.6` selection is replaced without an alias. Refresh is offline; regional keys are
independent. Limits, supported images, effort nulls and unknown highspeed prices survive.

- [ZAI thinking controls](https://docs.z.ai/guides/capabilities/thinking.md) and
  [China thinking controls](https://docs.bigmodel.cn/cn/guide/capabilities/thinking)
  distinguish GLM-5.2's native `none` from GLM-5.3's forced thinking. Binary models expose
  off/high, GLM-5.2 off/high/max, GLM-5.3 low/high/max. Unsupported choices are omitted;
  they do not falsely turn off a forced-thinking model.
- [Preserved thinking](https://docs.z.ai/guides/capabilities/thinking-mode.md) requires
  complete ordered reasoning across tool and later user turns. The ZAI dialect explicitly
  sends `clear_thinking=false` even when no effort was selected.
- [Global tool streaming](https://docs.z.ai/guides/capabilities/stream-tool.md) and
  [China tool streaming](https://docs.bigmodel.cn/cn/guide/capabilities/stream-tool) specify
  `tool_stream=true`. It is sent only alongside actual tools, and indexed calls retain
  exact arguments. The global guide's model wording is internally inconsistent (GLM-5
  restriction versus GLM-5.3 example); local fixtures do not resolve remote availability.
- The [Chat API schema](https://docs.z.ai/api-reference/llm/chat-completion.md) omits a
  strict function guarantee; [structured-output guidance](https://docs.z.ai/guides/capabilities/struct-output.md)
  describes JSON-object mode and client-side schema validation. PI's strict=true claim
  remains in raw/generated records, while reviewed runtime overrides refuse required
  strict tools and schema answers until the guarantee is established. This discrepancy
  remains open for full parity; preferred tools preserve the original schema.
- [China Coding Plan integration rules](https://docs.bigmodel.cn/cn/coding-plan/tool/others)
  give the pinned endpoint but restrict use to listed tools. NERV is not listed in the
  inspected page. Service approval/eligibility must be established before live use; no
  approved-tool impersonation or account request was performed.

`zai-inventory-fixed.log` passes **11 focused tests**: three adapter, two compiler and
six native tests. Native coverage makes **129 loopback requests**: 88 effort/no-tools,
33 parallel-tool/follow-up/later-user, four authentication/malformed-stream and four
retry requests. Listings, missing credentials and required-strict/schema refusals remain
zero-dispatch. Source override tests caught an initially incomplete replacement `compat`
object; all pinned fields now survive except the explicit strict guarantee override.
The exact-count recipe refuses changed/empty test filters. `zai-catalog.log` passes all
22 catalog tests and exact regeneration. `zai-verify.log` passes full Melchior verification:
956 library and 88 readiness tests in each default/all-feature run, plus remaining local gates.
Magi's full verification also passes with nine existing skips. Its selected-native and
retained-copy host proof passes ten scenarios / 20 requests / eight Echo executions,
including two ZAI scenarios retaining reasoning through tools, retry and later turns.
Full generic parity remains an expected failure: 34 generic passes / 417 pending / zero live
requests, after 50 native preset and seven shared tests pass. Dedicated native suites still
need requirement-specific aggregation. The source/binary/log receipt is
`../magi/target/provider-foundations/zai-20261121T081422Z-wE11nkx6/`; archives are not
independently rebuilt and retained binaries are dynamic. Service eligibility, strict-tool
guarantees and whole-family/UI/restart/static/live qualification remain open.

## DeepSeek Flash and Pro — API-key catalog, 2026-11-21

At this checkpoint coverage was **20 declarations**, **30 selected imported models** and **612 complete
candidate shapes**, not complete qualification of the 45 integrations / 1,779 records.
The `deepseek` preset selects `chat:deepseek-flash` and `chat:deepseek-v4-pro` from PI 1.6.0.
Its reviewed overlays enable effort control, disable unsupported answer JSON schemas and
select the explicit DeepSeek strict dialect. Source records, limits, cost presence and
effort nulls are unchanged. The legacy bundled Chat/Reasoner model IDs are not aliased.

The seven native cases make 32 loopback requests covering controls, user/tool-result images,
streamed reasoning/tool replay, later System/user turns, strict negotiation, beta routing,
retry and terminal cache accounting. Listings and invalid-before-credential cases dispatch
no requests. `deepseek-roundtrip-fixed.log` records these seven native tests plus 18 schema
and adapter tests; `deepseek-catalog.log` passes all 20 catalog tests and regeneration.
Full verification passes in `deepseek-verify-fixed.log`: 953 library tests and 82 native
readiness tests in each default/all-feature run, plus the other local gates. The complete
parity recipe still fails as intended at 32 generic passes / 419 pending / zero live requests;
its prerequisite preset suite passes 44 native and seven shared cases. Magi's selected
host proof passes eight scenarios / 13 requests, including a rerun with the retained binary.
The post-verification source/binary/log receipt is
`../magi/target/provider-foundations/deepseek-system-20261121T074927Z-9EcUwgUt/`.
Archives are not independently rebuilt, and binaries are dynamic. Full provider
qualification and selected-family/UI/restart/static/live evidence remain open. See Magi's
`PROVIDERS.md` for the strict subset, migration behavior and complete remaining scope.

## Xiaomi PAYG and regional token plans — API-key catalog, 2026-11-21

At this checkpoint the working tree had **20 declarations** (18 PI IDs plus Ollama and Decisions),
**28 selected imported models**, and **602 complete candidate shapes**. This is not
20 fully qualified integrations. The full 45-integration/1,779-record objective remains.

`xiaomi` selects `mimo-v2.6-flash`, `mimo-v2.6-pro`, and `mimo-v2.6-pro-ultraspeed`.
`xiaomi-token-plan-cn`, `xiaomi-token-plan-ams`, and `xiaomi-token-plan-sgp` select Flash
and Pro each. All nine pinned records are retained unchanged. Discovery is bundled/offline.

- The [Chat Completions reference](https://mimo.mi.com/static/docs/api/chat/openai-api-post.md)
  accepts Bearer authentication, `max_completion_tokens`, and strict function tools.
  The native adapter preserves system messages, exact tool arguments and streamed usage.
  It sends no store, forced-tool-choice, reasoning-effort or retention extension.
- [Deep-thinking requirements](https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/deep-thinking.md)
  require retaining reasoning on historical tool calls, including later user turns.
  All assistant messages on declared reasoning models carry `reasoning_content`;
  missing content becomes an empty string, while actual reasoning is preserved.
  Pinned controls expose off/high as disabled/enabled, not multiple reasoning depths.
  Other unsupported controls are omitted and the service's documented default applies.
- The [answer-format documentation](https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/structured-output.md)
  supports JSON-object mode, not schema enforcement. `supports_json_schema = false`
  rejects strict structured-answer requests before authentication instead of silently
  degrading to prompt-based JSON. This does not disable required-strict function tools.
- [Regional setup](https://mimo.mi.com/static/docs/tokenplan/Token%20Plan/quick-access.md)
  identifies the dedicated CN/AMS/SGP endpoints. Each uses its own pinned
  `XIAOMI_TOKEN_PLAN_{CN,AMS,SGP}_API_KEY`; PAYG uses `XIAOMI_API_KEY`. There is no generic
  `MIMO_API_KEY` alias or cross-region fallback. [Key guidance](https://mimo.mi.com/static/docs/quick-start/faq/api-integration.md)
  distinguishes PAYG and token-plan credentials; users must use their account's endpoint.
  Local readiness does not prove remote entitlement, subscription validity or region access.
- Snapshot PAYG rates are estimates, not current invoices. All token-plan rates remain
  unknown, not zero-cost; subscription quotas and billing are not inferred from token counts.

Focused evidence: `xiaomi-presets-rounded.log` passes 40 native preset tests plus seven
shared adapter tests. Seven Xiaomi native tests exercise 142 loopback requests: 72 control
selections, 27 tool/later-user turns, 27 output caps, eight typed failures and eight retry
requests. Key-isolated listing and pre-authentication refusals dispatch no HTTP.
`xiaomi-catalog-presence.log` passes 18 catalog tests and deterministic regeneration.
Initial fixture failures are retained: an untyped large integer, ceiling rather than
nearest-micro estimate expectations, and expecting an empty cost object instead of the
versioned all-false presence mask. Production accounting was not changed to fit fixtures.

`xiaomi-verify.log` passes the full local gate: 926 library tests in each default/all-feature
run, integration/examples, Clippy/rustdoc, architecture/hermetic/interface/role and dependency
checks. The complete parity gate remains an expected failure: 32 generic text/usage passes,
419 pending, zero live requests. Its receipt is
`target/provider-parity/run-1795243018488278262-1482726/`. Magi's model-refresh recipe passes
67 distinct tests and architecture gates pass. The retained batch snapshot is under Magi at
`target/provider-foundations/xiaomi-20261121T063712Z-boKp4vlJ/`; no separate archive rebuild
or static-package proof is claimed.

Shared Chat indexed tool-call interleaving and
`repetition_truncation` terminal classification need follow-up fixes; the current native
round trips prove single-call flows only. Selected-family/UI/static/live acceptance remains
open; no live inference is authorized.

## MiniMax global and China — selected API-key catalog, 2026-11-21

At this checkpoint the working tree had **16 declarations** (14 PI IDs plus Ollama and Decisions) and
**19 explicitly selected imported models**. This batch selects `MiniMax-M3` and
`MiniMax-M3.1-Flash` in each region. It does not qualify all 45 integrations or all
1,779 retained reference records.

- The [global compatibility documentation](https://platform.minimax.io/docs/api-reference/text-anthropic-api)
  specifies the Anthropic endpoint, automatic-only tool choice and complete assistant
  content replay. Local compatibility settings select adaptive on/off for M3 and mapped
  adaptive effort for Flash. Flash's unsupported off/minimal remain explicit nulls;
  M3's `minimal` control is its single enabled state, not a separate depth or budget.
- The [current China documentation](https://platform.minimax.cn/docs/api-reference/text-anthropic-api.md)
  uses `https://api.minimax.cn/anthropic`. Provider and model overrides use that endpoint,
  not the pinned `api.minimaxi.com` address. The immutable source is unchanged.
  `MINIMAX_API_KEY` and `MINIMAX_CN_API_KEY` are isolated; neither region accepts the
  other's variable or `ANTHROPIC_API_KEY` as an alias.
- Both current regional documents list only M3 and M3.1 Flash on this interface.
  The two M2.7 records per region remain in the pinned inputs/candidates, **not enabled**.
  Restoring or replacing their integration requires documented routing/availability
  evidence and explicit plan reconciliation; removing them from the target is not done.
- Complete replay uses bounded credential/endpoint/model-bound opaque state. It preserves
  interleaved unsigned thinking, text, tool input shapes and extra provider fields across
  Magi's neutral representation. No independent Magi vendor registry or wire change is added.
  Different credentials/model/endpoint/headers discard opaque state, not visible tools.
  Failed, unfinished, unknown-terminal and length-limited streams publish no replay envelope.
- Bundled listing is offline. Strict-required tools, forced structured answers and oversized
  image payloads refuse before credentials. Pinned prices and the M3 512,000-token tier
  boundary remain snapshot estimates; they are not current billing or entitlement promises.

Focused evidence: `minimax-transport.log` passes 30 tests (eight adapter, three transport,
nine compatibility, six thinking and four native). `minimax-catalog-fixed.log` passes
16 catalog tests and deterministic regeneration. `minimax-responses-regression.log`
passes the shared Responses/JSON/credential regressions. `minimax-presets-final.log` passes
33 native preset tests plus two shared tests. `minimax-verify-fixture.log` passes full local
verification, including 921 library tests in each default/all-feature run, integration/example
suites and Clippy/rustdoc/architecture/hermetic/interface/role/dependency gates. The first
full run exposed an invalid orphan-delta Anthropic parity fixture; corrected lifecycle frames
pass without weakening the adapter. The failure remains in `minimax-verify.log`.

The preset suite adds eight MiniMax tests covering 72 loopback inference requests:
32 control selections, eight tool round trips, eight typed failures, four tier boundaries,
12 output-cap checks and eight transient-retry requests. Listing/readiness and unsupported
requests do not dispatch. The first accounting run found a one-micro estimate error in the
test fixture (271110 instead of rounded 271111); the production calculation was correct.
Initial failing logs are retained. Full-family/UI/static/live and M2.7 availability remain open.

Explicit parity, streaming (176 tests), usage, overflow and cache recipes pass. The full-parity
gate remains an expected failure: 24 generic text/usage passes, 427 pending, zero live calls.
Negative receipt: `target/provider-parity/run-1795241635197889971-1373094/`.
The source/log snapshot is under Magi at
`target/provider-foundations/minimax-20261121T061428Z-8CdUKiEa/`; it is not a separately
rebuilt archive, selected-family, static-package or live-service acceptance receipt.

## Meta — pinned API-key catalog, 2026-11-21

At the Meta checkpoint the working tree had **14 provider declarations** and **15 explicitly selected imported
models**, not 14 fully qualified integrations. This batch selects all seven pinned chat
records: `muse-ember-1.0`, `muse-spark-1.1`, `muse-spark-1.2`,
`muse-spark-1.2-contributor`, `muse-spark-1.3`, `muse-spark-1.3-contributor` and
`muse-wonder-1.0`. The target remains 43 reference IDs plus Ollama and Decisions,
with all 1,779 reference records retained.

- The [pinned declaration](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/meta.ts)
  supplies `META_API_KEY`, Responses and `https://api.meta.ai/v1`. Bundled discovery
  keeps listing and explicit refresh offline. No subscription login is implemented.
- [Meta's Responses protocol](https://dev.meta.ai/docs/protocols/responses) supports
  stateless encrypted reasoning replay. Requests use `store=false`, include encrypted
  reasoning, and replay complete bound reasoning items alongside tool calls/results.
  No server-side `previous_response_id` or truncation override is sent. Switching models
  drops opaque continuation bound to the previous identity without dropping visible tools.
- [Contributor models](https://dev.meta.ai/docs/muse-spark) permit use of submitted content
  for training. Both variants have `training-enabled` display labels, distinct from their
  routable IDs. Standard-model retries retain the original model; there is no automatic
  Contributor fallback. Disabling response storage does not opt out of Contributor terms.
- Context/output limits, prices, cache rates, image resizing and effort maps come from
  the immutable snapshot. They are estimates, not current billing promises. `max` is
  exposed only on Ember, Standard Spark 1.3 and Wonder; unsupported `off` stays unsupported.
  Required-strict tools and oversized images refuse before credentials or dispatch.

`oslo make test-provider-presets` passes two shared adapter tests and 25 native tests,
seven specific to Meta. Meta's seven models account for **109 loopback requests**:
56 effort selections, nine accounting/tier checks, 21 continuation/tool/model-switch
requests, 21 nonretryable-error requests and a two-request transient retry.
Listing, wrong/missing/blank key, strict and image-limit checks make no inference calls.
The initial label regression and subsequent compilation failure are retained in
`target/provider-foundations/meta-presets*.log`; the passing rerun is
`meta-presets-fixed.log`. Magi label propagation and broader gates are tracked in
`../../magi/PROVIDERS.md`, not inferred from the native preset result.

Remaining: Muse subscription authentication, provider-specific cancellation and account
claims, selected-family/UI/static/live acceptance and all other unfinished integrations.
MiniMax is not enabled by this batch: adaptive thinking, complete assistant-content replay
and restricted tool choice need separate implementation/qualification.

## xAI — pinned API-key catalog, 2026-11-21

The working tree has **13 provider declarations**, not 13 fully qualified integrations.
This batch selects all four pinned xAI chat records: `grok-4.3`, `grok-4.5`, `grok-4.6`
and `grok-4.7`. It does not implement the reference provider's Grok/X subscription login.
The overall target remains 43 pinned PI IDs plus Ollama and Decisions, with all 1,779
reference records retained. There are now eight explicitly selected imported models.

### Declaration and model contract

- The pinned [xAI declaration](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/xai.ts)
  supplies `XAI_API_KEY`, the Responses protocol and `https://api.x.ai/v1`. Listing and
  `--refresh` use the bundled catalog without HTTP. Readiness means a usable local key
  was found, not that the service accepted it or granted access to these models.
- Context/output ceilings, text/image input, resize profile and tiered estimates come from
  the immutable snapshot. Cache-write pricing remains unspecified, not free. The native
  cards and accounting fixtures check each model, including the 200,000-token tier boundary
  using the full prompt count rather than only uncached input.
- [Grok 4.3's model details](https://docs.x.ai/developers/models/grok-4.3) list `none`,
  `low`, `medium`, `high` and `xhigh`, matching its pinned map; neutral `off` sends `none`.
  For the 4.5–4.7 series, [xAI's reasoning documentation](https://docs.x.ai/developers/model-capabilities/text/reasoning)
  prohibits disabling reasoning and describes 4.5's `xhigh` as equivalent to `high`.
  A local 4.5 override maps that selection to `high`; the other map entries, including
  explicit unsupported nulls, are preserved. This is an alias, not an additional depth.
- The [pinned Responses compatibility defaults](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/api/openai-responses.ts)
  leave strict tools disabled. This preset advertises false, omits strict for preferred
  tools and refuses required-strict tools before credentials or HTTP. Long cache-retention
  fields remain absent. Requests use `store=false` and request encrypted reasoning.

### Reported billing

[xAI reports per-request costs](https://docs.x.ai/developers/cost-tracking) in
`usage.cost_in_usd_ticks`, with 10,000,000,000 ticks per dollar. The Responses adapter reads
the lossless terminal JSON value, validates an unsigned integer, and rounds to the existing
microdollar wire unit (10,000 ticks) without passing the whole integer through floating point.
The rounding error is at most half a microdollar per request; this is not raw-tick accounting.
Tests include both sides of rounding boundaries above 2^53 and the maximum `u64`.

An explicitly present tick field takes precedence over an unrelated dollar `cost` field.
Malformed ticks produce an anomaly and unknown measured cost, not a fabricated zero or
dollar fallback. Missing ticks preserve existing dollar-cost and snapshot-estimate handling.
Failed and incomplete terminal responses retain their charge; duplicate terminal events do
not charge twice. A truncated billed attempt plus a successful retry adds both charges to
the run-wide ledger, and a reached cap prevents the next HTTP request.

The legacy usage/tally contract still treats zero reported microdollars as no positive charge
and retains a token estimate; separate measured accounting distinguishes explicit zero from
unknown. This batch does not redefine that cross-provider policy. Sub-microdollar raw charges
are not retained in the wire or ledger.

### Offline evidence and remaining work

`oslo make test-provider-presets` includes ten xAI native test functions (79 loopback inference
requests), alongside the eight prior native preset tests and two shared adapter tests. They
load the actual shipped declaration and test every pinned model's cards, request shape,
effort map, streamed calls, encrypted-state replay, user/tool images, pricing, refusals,
and selected retry/cap scenarios. Keys, HOME, XDG state and endpoints are disposable fixtures.
Four additional adapter tests exercise tick conversion across the three Responses hostings.

The common parity harness now reports 18 generic text/usage passes and 433 pending cases,
not full provider qualification. Subscription OAuth, xAI-specific cancellation/handoff and
selected-family/UI/static acceptance remain open. No live inference, real login, entitlement
check, installed configuration change or deployment was performed. Wider gate receipts are
recorded in `../../magi/PROVIDERS.md`.

## NVIDIA — first selected-model batch, 2026-11-21

That checkpoint had 12 provider declarations. This batch adds the `nvidia` ID
with `stepfun-ai/step-5-preview` only; the other 11 pinned NVIDIA records remain in scope
and unselected. No live inference, entitlement check or account configuration was performed.

- The pinned [NVIDIA declaration](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/nvidia.ts)
  supplies the Chat Completions protocol, `https://integrate.api.nvidia.com/v1`, and
  `NVIDIA_API_KEY`. The selected snapshot preserves its model ID, context/output limits,
  resize profile, unknown prices and `NVCF-POLL-SECONDS: 3600` header.
- NVIDIA's [Step 5 API reference](https://docs.api.nvidia.com/nim/reference/stepfun-ai-step-5-preview-infer)
  documents function tools, reasoning-content replay and up to eight images. The local
  override enforces eight request-wide images, including tool-result images; it retains
  the pinned resize bounds. Reasoning cannot be disabled on this checkpoint, so cards
  advertise an empty list of thinking controls rather than inventing effort levels.
- The snapshot's disabled long-retention setting matches the current Chat adapter's
  omission of cache-retention fields. The compiler qualifies only explicit false for
  Chat Completions and Responses. True, malformed and other-protocol values stay pending.
  The pinned [Chat implementation](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/api/openai-completions.ts)
  makes long-retention fields conditional on that capability. The source JSON remains intact.
- `discovery = {kind="bundled"}` never contacts NVIDIA for listing or refresh. Native
  fixtures check wrong/blank keys, missing-key refusal, exact endpoint/header/options,
  split tool arguments, reasoning replay, user and tool images, usage, 400/401/422 failures,
  strict-required refusal and an unchanged 503 retry. Fake keys and loopback HTTP only.

The native fixture exposed an unconditional zero estimate for unknown prices. The runtime
now leaves that estimate absent, while retaining explicit-free zero and every positive partial
estimate. Existing reported-versus-estimated retry settlement and context-tier charging are
preserved. Partial estimates are not complete bills; the operational accounting path retains
its separate unknown-rate representation. Do not rely on unknown NVIDIA prices for a complete
dollar cap or interpret an absent price as a free hosted service.

At that checkpoint, `oslo make test-provider-presets` ran two shared reasoning tests and eight
native preset tests, five specific to NVIDIA. Required wider gates and their receipts are recorded in
`../../magi/PROVIDERS.md`. The parity harness retains its limited text/usage meaning; no
authentication, tool-loop or full-provider row is declared complete from this model subset.
NVIDIA-wide catalog review, cancellation/handoff qualification, full-family/UI/static and
authorized live acceptance remain open.

## Groq and Cerebras — first selected-model batch, 2026-11-21

This batch adds two provider declarations, not full PI parity. The target remains all
43 pinned reference providers, 1,779 in-scope model records, and the retained Ollama and
Decisions integrations. That checkpoint had 11 shipped provider declarations. No live account
entitlement or paid inference has been tested.

| Provider | Selected pinned chat models | API-key variable | Bundled coverage |
| --- | --- | --- | --- |
| `groq` | `openai/gpt-oss-120b`, `openai/gpt-oss-20b` | `GROQ_API_KEY` | 2 of 7 records |
| `cerebras` | `gpt-oss-120b` | `CEREBRAS_API_KEY` | 1 of 2 records |

The remaining six records stay in the pinned inputs and candidate report. Their generic
image-history constraint is now qualified, but all six still need their own availability,
capability and dialect review before this batch expands. They are not removed from the target.

### Declarations and protocol review

- IDs, endpoints and environment variables follow the pinned
  [Groq declaration](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/groq.ts)
  and [Cerebras declaration](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/providers/cerebras.ts).
  Both select Chat Completions. Source-file digests are retained in the parity manifest.
- Compatibility defaults follow the pinned
  [Chat Completions implementation](https://github.com/earendil-works/pi/blob/5a7479a88390d947335a1a6609936897be0a7891/packages/ai/src/api/openai-completions.ts):
  system messages, `reasoning_effort`, `max_completion_tokens`, streaming usage, no tool-result
  name, Groq `store=false`, and omitted Cerebras storage. Per-model compatibility remains
  authoritative; Groq's selected records enable strict tools and Cerebras's does not.
- Groq documents [GPT-OSS reasoning and a strict function example](https://console.groq.com/docs/reasoning).
  Cerebras documents [bearer authentication, tool messages and GPT-OSS effort levels](https://inference-docs.cerebras.ai/api-reference/chat-completions).
  These primary documents inform the wire fixtures, not a claim of account entitlement.
- Context, output limits, prices and thinking maps come from the immutable PI 1.6.0
  snapshot. Prices remain snapshot estimates, not current billing quotes. Unspecified
  cache-write prices remain unknown. Explicit null effort mappings stay unsupported.
- `discovery = {kind="bundled"}` keeps listing and explicit refresh offline for these
  providers, with or without credentials. Provider-specific keys are resolved only for
  inference. New models are added through an explicit reviewed selection change.

### Qualification

`oslo make test-provider-presets` exercises both declarations and every selected model,
using the actual binary, isolated HOME/XDG/configuration, fake keys and loopback HTTP:

1. Model metadata, supported efforts, missing/whitespace/wrong-provider credentials,
   ready state and offline listing even with `--refresh`.
2. Missing-key refusal before dispatch; exact path, bearer header, model ID, system role,
   output-limit field, reasoning effort, storage, usage and strict-tool request shape.
3. Streamed reasoning, split tool arguments, call identity and tool-use termination;
   cached/input/output counts and model-specific estimated cost; a following request
   carrying the original call, reasoning field and matching tool result.
4. Single typed 401/400 failures without retry or success termination, and no credential
   values in output or request bodies.
5. Shared reasoning-field round-trip and arbitrary-signature rejection regressions.

`test-provider-parity` and `test-provider-parity-complete` depend on this recipe. The parity
worker now assembles declarations through the production catalog loader rather than
deserializing raw Lua values and silently skipping imports. Its separate generic text/usage
fixtures are still protocol smoke coverage, not full per-provider acceptance. Receipts include
the shipped catalog inventory and generated-catalog digest.

Cancellation, cross-provider handoff, the six remaining model records, full-family/UI/static
acceptance and authorized live receipts remain open. The complete parity gate must still fail
while any required case is pending. See `../../magi/PROVIDERS.md` for current verification logs.
