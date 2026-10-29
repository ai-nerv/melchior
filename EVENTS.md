# Local operational events

Phase11's approved initial policy is implemented as an independent owner-local
module with instrumented provider attempts, retry waits and cumulative usage, not an enabled
telemetry service.

`events::emit` skips payload preparation until explicit `enable_local` creates
the process's single receiver. Ordinary runtime configuration does not activate it.
Explicit native bootstrap can instead bind that receiver to a joined forwarder;
the default path creates no worker, disk spool or network connection. Startup obtains an
OS-random128bit producer identity. Local ask/attempt IDs derive from that identity
and a nonwrapping atomic counter through domain-separated SHA256 truncated to
128bits. They require no per-call entropy I/O and contain no native identifiers or
user content. Identity exhaustion skips correlated observations and increments
the explicit exhaustion counter; it does not wrap or change provider behavior.

The strict revision1 envelope contains `program: melchior`, a numeric package
version triple, opaque producer identity, nonzero sequence and nullable opaque
run/agent/turn/request/ask/attempt/job/tool-call correlation. Unknown revisions,
owners, kinds, fields, invalid numeric states and noncanonical IDs are refused.
The decoder checks4096bytes before parsing and returns fixed error classes.

Owner facts are typed provider-attempt outcomes, retry transitions and cumulative
usage snapshots. Snapshots have nonzero revision and separate nullable input,
output, cache-read/cache-write counts, provider-reported cost, estimated cost and
ledger-persistence outcome. Null is unknown; zero is a measured zero. A snapshot
is not an increment to sum, and observed usage is not proof the ledger was saved.
The native accounting owner emits snapshots under the same local ask identity as
its provider attempts. Bare Client streams remain attempt-only.

Usage evidence is preserved separately from legacy scalar `Usage` values through
the Lua adapter and internal traced stream. Builtin dialects annotate measurements
before default filling; custom adapters without annotations remain unknown. Evidence
must match the retained native scalar, so invalid/decreasing candidates cannot
override accounting. A measured zero is retained as zero, while absent or invalid
data stays null. Inclusive input is unknown without a measured cache partition;
clamped contradictory cache counts are not presented as exact measurements.
Public stream deltas and serialized Usage remain unchanged.

The native meter exposes replaced/unchanged/untracked/failed settlement outcomes
without changing charging behavior. An unchanged zero charge need not create a
ledger, and replacement success means write+rename, not crash-durable fsync.
Cumulative snapshots replace the current attempt's evidence and add completed
attempts with checked arithmetic. A dispatched but unreported attempt makes its
contribution unknown; waiting to retry does not invent another attempt. Cancellation
folds only dispatched work. Unknown or overflowing fields remain null. Revisions
advance even when delivery drops, and exhaustion counts loss without wrapping.

Settlement snapshots distinguish persisted, unchanged, untracked and failed native
meter outcomes. Lifecycle-only snapshots use unknown ledger status, not a claim of
another write. Reported charge and estimated charge stay separate; estimates require
known token counts and defensible native rates. Missing default prices on nonzero
counts are unknown, not measured free use. Queue loss and incomplete provider evidence
can prevent reconciliation; these events are neither a ledger nor a complete-run receipt.

One provider stream has one local ask ID; each attempt has a distinct attempt ID.
Retry scheduled/dispatched/cancelled/failed events reference the next attempt ID.
Retry dispatched means entering that attempt, not network transmission.
Provider-attempt dispatched occurs after request construction and URL admission,
immediately before handing the request to the HTTP executor. It does not prove
remote receipt. Preparation refusals have no dispatched event. Completion follows
the streaming result; dropped futures emit cancellation, while callback panic
unwinding emits failure. Unpolled futures emit nothing. Terminal attempt duration
uses a monotonic clock and includes credential preparation; dispatch has no duration.
No run/agent/turn or cross-process correlation is inferred from these local IDs.

Explicit native bootstrap2 instead supplies a strict immutable run/agent/turn/ask
binding in at most512bytes. Run and agent must be paired; turn/ask require that
root. Binding overrides local candidates during encoding without changing the
native ledger or attempt IDs. The collector checks bound fields before accepting
or deduplicating events. Event schema1 and4096byte limits remain unchanged.
Capabilities advertise bootstrap2; parent identity and descriptor checks are
unchanged. `lineage-native-fixture.log` verifies owner tests and fully bound
selected-native interoperability. Daemon request and helper attribution are not
established by a process-level binding.

Enqueue never waits for capacity: a fixed4096byte encoder feeds a256slot try-send
queue. Queued payload is at most1MiB, excluding metadata, concurrent encoders and
consumer-owned data. Full, closed, encoding and exhausted losses are counted with
saturation; maximum means at least that many. Receiver shutdown reports successful
enqueues not handed to the consumer as abandoned; send/close accounting settles
after producers return. Counter snapshots are not atomic. Event identity is the
producer/sequence pair, not enqueue order; dropped attempts consume sequences.

`oslo make test-events` runs golden, hostile-field, disabled, saturation,
concurrency, shutdown-race and usage-snapshot contracts. An isolated integration
process drives actual Lua adaptation and loopback HTTP through success, retry,
pre-dispatch refusal, header/body/backoff cancellation, socket closure, callback
failure, privacy canaries and disabled/full/closed queues. Accounted-stream integration
also checks revised and duplicate quotes, paid/unreported retries, cancellation,
failed ledger-write recovery, absent ledgers, measured zero, estimates and full queues
against actual native ledger state. Revision exhaustion has a separate boundary test.
These tests are not
collector/export, cross-process attribution, billing reconciliation or performance
acceptance. Local collection policy is approved; network export is not implemented.

## Explicit native forwarding

Only `ask --event-bootstrap` and `serve --event-bootstrap` accept the private
parent handoff. The strict bounded packet carries an event descriptor and the
original stdin descriptor. Parent credentials, owner, fresh producer identity,
socket type, flags and buffer bounds are checked before stdin is restored.
Malformed handoffs fail before useful work starts. Forwarding activation failure
after restoration leaves useful work available with unknown event loss.

Serve completes its identity-setting self-exec before receiving the packet.
Received descriptors remain close-on-exec; bind and announcement errors return
through the forwarder guard. The sole queue is drained by one joined worker with
one nonblocking send per event and no retries, disk spool or second event queue.
Status sends are rate-limited, including the final attempt; missing final status
is unknown, not zero loss. Shutdown does not wait for in-flight payload creation.

Magi's selected-native integration checks actual executable and restored stdin,
loopback provider results, self-exec, serving status, bind failure and reaping.
`sibling-bootstrap-observed.log` passes these checks and this owner's event,
accounting, serving and Clippy suites. `event-capabilities` returns fixed family
JSON before configuration or useful work. Opted-in Magi now negotiates normal
serve and provider ask startup and retains event lifetimes through child reaping;
standalone defaults remain disabled. Native broker tests cover streaming completion,
cancellation, older-owner fallback and negotiation-worker expiry without orphaning
or prematurely ending the selected provider. Complete containment,
cross-process lineage, overhead and full-family acceptance remain outstanding.
No shared Rust runtime crate or dependency upgrade is added.
