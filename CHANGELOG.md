# Changelog

## [0.7.2] - 2026-10-15

### <!-- 5 -->🎨 Styling

- What clippy 1.95 asks for

### Build

- Move to rustc 1.95

## [0.7.1] - 2026-10-14

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Reproducible across build dirs

## [0.7.0] - 2026-10-14

### <!-- 0 -->⛰️  Features

- Images for every vision protocol
- Prices, ranks and capabilities

### <!-- 3 -->📚 Documentation

- Image as a tools extension
- Model cards and images

### Build

- Fake provider and metered proxy

## [0.6.1] - 2026-10-14

### <!-- 0 -->⛰️  Features

- Cap what one run may spend

### <!-- 1 -->🐛 Bug Fixes

- Send plain http to this machine only

### <!-- 3 -->📚 Documentation

- The spending cap
- The model role's spend cap

### <!-- 6 -->🧪 Testing

- The cap against the real binary

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Spend cap
- Https except loopback

## [0.6.0] - 2026-10-14

### <!-- 0 -->⛰️  Features

- Frame calls in the stream, not in Lua

### <!-- 1 -->🐛 Bug Fixes

- Hide a userdata's metatable
- Bound Lua-to-JSON by nodes, bytes, cycles
- Refuse __gc metatables and drop debug
- Refuse precompiled chunks everywhere
- Hold every VM to a memory ceiling
- Refuse a call from a newer family
- Take TMPDIR only when absolute
- Take rustls and quinn-proto fixes
- Take the uid only from the host
- A reply that will not decode closes
- Name an unknown verb to any caller
- Never name a child after its parent
- Withdraw an own asking only on accept
- One runtime rule, flat names, no shell
- Meter every usage shape, settle always
- Count what every attempt cost
- Bound streams, say refusals, honour waits
- Cap tells, one inbox, steady roster
- Reserve names, drop stale parent notes
- Start no outsider with family capability
- Runtime dirs 0700 and owner-checked
- Keep given config per session only
- Follow no redirect with a key on it
- Answer off the loop, under deadlines
- Mint secrets without a panic
- Take the body only from the message
- Kernel kin, stdin token, adopt nonce
- Read a report only across agent_talk
- Cap frames and refuse a newer family
- Check the server uid and its family
- One 16 MiB cap, answered not dropped
- Stop a VM that runs past its budget
- Dial only the family's own sockets

### <!-- 2 -->🚜 Refactor

- No panics on input
- Page lines without slicing
- No panics on input
- Pick names without indexing
- Mark a cut row without indexing

### <!-- 3 -->📚 Documentation

- The three VMs and getmetatable
- The memory key and socket families
- What a VM refuses
- One contract in all four
- Drop a duplicated heading
- One text across the family
- Install from the ai-nerv Nix cache
- Say where callers and secrets come from
- Runtime dir, lock, stream bounds
- Name hexe and oslo sockets
- Frames, sockets, deadlines, callers

### <!-- 5 -->🎨 Styling

- Collapse the guard ifs clippy flags

### <!-- 6 -->🧪 Testing

- Hold a one-slice chunk to the ceiling
- One gate-family in all four
- Hold the VM surface to an allowlist
- Probe a live socket for refusals
- Hold the wire to one cap and dir rule
- Hold the plugin VM to its narrow surface
- Run the family conformance corpus
- Find the busy reply without racing

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Pin actions by commit
- Publish tagged builds to ai-nerv

### Build

- Enable flakes for the dev shell
- Run cargo audit and deny in verify
- Check overflow in release
- Read the version from Cargo.toml
- Nix-build, nix-check, download check
- A static package with config and client
- Deny expect and indexing outside tests

## [0.5.5] - 2026-09-29

## [0.5.4] - 2026-09-29

## [0.5.3] - 2026-09-29

### <!-- 1 -->🐛 Bug Fixes

- Enforce reports and refresh models

## [0.5.2] - 2026-09-23

## [0.5.1] - 2026-09-23

### <!-- 1 -->🐛 Bug Fixes

- Give up at once on no route to host
- Name the provider and why it was unreachable

## [0.5.0] - 2026-09-23

### <!-- 0 -->⛰️  Features

- Read windows and thinking from api/show

### <!-- 1 -->🐛 Bug Fixes

- Honour OLLAMA_HOST

## [0.4.0] - 2026-09-23

## [0.3.0] - 2026-09-21

### <!-- 0 -->⛰️  Features

- A dialect for models that decide
- A proxy that meters live spend
- The fake serves many at once
- The fake can pause mid-answer
- The fake can split and hang up
- A report is handed in, not sent
- The fake provider can refuse
- The side edited last wins
- A fake provider for acceptance
- Coordinate credential persistence
- Never route through an avoided upstream
- Say which upstream answered
- Name the upstream that served each stream
- Name the finish each stream ended on
- Say how each stream ended
- One family log, every step stamped

### <!-- 1 -->🐛 Bug Fixes

- Off is the least a model that must reason will do
- A model that must reason is not told to stop
- Say off to every model, not only reasoners
- A failed stream is not charged
- An overflow said mid-stream is one
- An upstream's mid-stream failure is said
- A peer that vanishes is lost
- A provider that never answers is asked again
- An error finish is asked again
- A stream that goes quiet is asked again
- A tool call written as text is retried
- Keep each event's end, where the finish is
- A stream that never stopped is retried
- A child is on its own run's roster
- Discovered models get their provider's dialect
- Quiet the roster's status poll

### <!-- 3 -->📚 Documentation

- Plan as the layout fallback
- Layout, jobs and notes verbs

### <!-- 6 -->🧪 Testing

- Cover claim handover and permissions

## [0.2.0] - 2026-09-15

### <!-- 0 -->⛰️  Features

- Carry each session's spend
- Provider choice, prices and more facts
- Card verb and OpenRouter cost
- Show each agent's phase in crew
- Watch/unwatch verbs for cross-tree signals
- Phase in roster and edge signals
- The roster carries each agent's live status
- The pushed roster carries each agent's parent
- Full tree-merge adoption
- Reach the model vocabulary over the socket
- A session's children are capped in number
- A tree of agents has a depth limit
- What this program is for, as a contract
- Disband ends a branch
- The roster shows who started whom
- Advertise the tool door
- Unknown verbs reply; surface and fault
- A comment costs a fifth of its code
- No Rust of melchior's reaches a sibling
- One Lua VM, and it is the sandboxed one
- Publish each agent's screen beside its socket
- Claims, fan-out, task handles and caps
- Give an agent a role that grants nothing
- Name the run a session belongs to, and its crew
- Declare agent_talk, mint and adopt on the wire
- Discover, acknowledge and version the surface
- Implement the family contract
- Answer in the family shape, and say which door
- Answer a call in the encoding it arrived in

### <!-- 1 -->🐛 Bug Fixes

- Ask for 32k tokens, not the ceiling
- The caller's identity comes from the kernel
- Honour a short TMPDIR, name a full disk
- A listing is the rows, not one row
- A held connection goes with its call
- A failed read takes the connection
- A disbanded branch is counted by dialling
- An address wears the target's role
- A reply naming no revision predates it
- A half-made claim does not outlive its process
- The gate list is the directory
- The suite is watched in both its trees
- A skipped test item ends at its semicolon
- A signal takes the session's notes too
- A note that will not write is said out loud
- A turn does not outlive the magi that asked
- Die with magi even when magi is killed outright
- Stamp a run so a recycled id cannot reopen it
- The gate skip list was not valid sh
- One-shot replies name the family too

### <!-- 3 -->📚 Documentation

- Casper's socket door, and its jail
- How a tools program is configured
- Sessions is core to the memory role
- Both client shapes are correct
- The send half closes too
- Abandoning a call abandons the line
- The export corpus is not an answer
- The sandbox line is the whole header
- The sandbox facts, and rustfmt after
- The frame and the rung, not the story
- The wire contract, not the sermon
- The dot and the link, not the essay
- The vendor quirk, not the essay
- The invariant, not the argument
- Keep the wire facts, cut the essays
- Cut the argument from the socket
- Describe the program, drop the case for it
- The death signal watches a thread
- What a second signal costs, measured
- A private item is not a doc link
- Casper's declarations are an installed file
- A spawn-per-call sibling is told on every spawn
- A lua tool declares its transport
- Link the wiring walkthrough

### <!-- 5 -->🎨 Styling

- Drop a needless reborrow

### <!-- 6 -->🧪 Testing

- Openrouter provider order
- The fixture waits for the session
- The far end outlives the second call

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Merge develop to main
- Control push
- A run can be started by hand

## [0.1.0] - 2026-09-06

### <!-- 1 -->🐛 Bug Fixes

- The dev shell evaluates on aarch64

