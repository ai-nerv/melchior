# melchior and A2A

**Decision: melchior does not speak A2A, and will not until something outside this family asks
it to.** What it gains instead is a written reason, and one property it did not have — a version
on the wire — so that changing this decision later is a change and not a rewrite.

Written 2026-09-06. Revisit when the first of the triggers below fires.

## What A2A is, and where it stands

A2A (Agent2Agent) was Google's, went to the Linux Foundation in June 2025, absorbed IBM's ACP in
August 2025, and reached v1.0 in April 2026. It is now the one standard in this space rather
than one of three, which is the fact that makes this worth deciding rather than ignoring.

It is a JSON-RPC protocol over HTTP with an Agent Card for discovery, tasks with a lifecycle, and
streaming over SSE. It is designed for agents that are *services*: different vendors, different
machines, different trust domains, discovered over a network.

## What melchior is

A layer for sessions that are on one machine, started by one person, under one uid.

Every design decision in it follows from that:

- **Unix sockets, not HTTP.** `SO_PEERCRED` answers "is this the same user" from the kernel,
  which is not a claim the caller can make. A2A's transport has no equivalent; over HTTP that
  question becomes a token, and a token is something to mint, hold, rotate and leak.
- **The directory is the discovery.** A socket in a known place *is* the registry, and a session
  that died did not get to deregister itself — which is why `children` reads notes off disk and
  why a stale socket is a dial that refuses. An Agent Card is a document a service publishes
  about itself; here the thing that publishes it is the thing being asked about.
- **The walls are relations, not scopes.** `policy::between` decides what one session may do to
  another from *how they are related* — parent, child, sibling, cousin, elsewhere — read off the
  directory, never from what the caller says. A2A has no notion of the caller's relationship to
  the callee; authorisation is a bearer token and whatever the receiving service decides.
- **A person is in the loop by construction.** `Then::Ask` carries a question up to a human and
  holds the call until they answer. A2A's task lifecycle has `input-required`, which is the same
  shape — but for a *remote* user, over a stream, with no way to say "the person at this
  keyboard".

Speaking A2A would mean either losing those properties at the boundary or writing an adapter
that refuses most of what A2A permits — which is a worse standard-compliance story than not
claiming compliance.

## The honest cost of not doing it

1. **An A2A client cannot drive a magi session.** Somebody wanting to put magi behind an
   orchestrator has to write the socket client. The Lua stub is shipped and `melchior lua-api`
   prints it, which makes that a small job, but it is a job.
2. **magi sessions cannot call an A2A agent as a tool.** This is the one that will actually
   matter, and it is *not* melchior's to fix: a remote agent is a tool, and tools are a registry
   question. MCP is the answer there and MCP is the one being built.
3. **"Does it do A2A" is a question that gets asked.** The answer is this file.

## What was done instead

`FAMILY = 1` on every reply (`wire::FAMILY`, duplicated in each sibling because there is no
shared crate and cannot be one). Four implementations of this wire existed with no version in any
of them, already disagreeing about whether `n` is optional and whether `fault` exists — so a
version skew presented as a missing field at the point of use, which reads as the peer being
broken rather than as the peer being a different build.

That is the property A2A would have brought that this family actually lacked. It cost twenty
lines rather than a protocol.

## What would change this

Any one of these, and this decision is reopened:

- **Something outside this family wants to drive a session.** One real consumer, not a
  hypothetical one. The adapter is then written against what it actually needs.
- **A session needs to live on another machine.** The moment `SO_PEERCRED` stops being able to
  answer the question, the whole argument above changes, and A2A's answers to authentication and
  discovery become the right ones rather than the heavy ones.
- **A2A grows a local transport with a peer-credential story.** Then the objection is only to the
  weight, and the weight is worth paying for a standard.

Until then: a private protocol, versioned, with the reason written down.
