-- melchior's client library: what another program requires to talk to a running session.
-- Plain Lua, so siblings copy it rather than port it. The transport arrives as the chunk's
-- argument; inside a host that is its own stream primitive, found automatically when nothing is passed.
--
--   local melchior = load(src)(my_transport)
--   local them = melchior.connect("beta-nu")
--   print(them.status())
--
-- The transport does the framing, so nothing here writes a length prefix or reads a frame:
--
--   transport.connect(path, timeout_ms)               -> handle | nil, why
--   handle:call(request, { deadline_ms = N })          -> reply  | nil, why
--   handle:close()
--
-- What is on the other end is one *session* — one agent, one conversation, one name. There is no
-- server for the layer as a whole: every session binds its own socket and answers for itself, so
-- "connect to melchior" is always "connect to somebody".
--
--   $XDG_RUNTIME_DIR/melchior/<project>/<id>          the socket a session listens on
--   $XDG_RUNTIME_DIR/melchior/<project>/<id>.parent   who started it, when somebody did
--
-- A name is `project/role/id`, and the role is what a session says it is *for* — it never
-- decides which socket is meant. The id does. So `review/iota-mu` and `scratch/iota-mu` are the
-- same session described two ways, and `melchior.connect` reaches it by the last part either way.

local transport = ...

-- Where the socket primitive comes from when the caller did not say. Inside a host the whole
-- library is already there; elsewhere a host that named its own `__stream` is honoured too.
if not transport then
  transport = (_G.melchior and _G.melchior.stream) or (_G.magi and _G.magi.stream) or _G.__stream
end

local M = { _NAME = "melchior", _VERSION = 1 }

-- Every global this family answers to. The file is copied between siblings, so a lookup that knew
-- only its own name would fail on exactly the hosts it is meant to run in.
local FAMILY = { "melchior", "magi", "aeon", "oslo", "hexe" }

-- The revision of the reply shape this library reads. A newer one is refused by name.
local FAMILY_REVISION = 1

-- ---------------------------------------------------------------- the session

local Session = {}
Session.__index = Session

--- Send one call and wait for its answer.
---
--- The handle frames it: the request is refused over the family's cap before a byte is sent, and
--- any failure after the first byte leaves the handle closed, so the next call meets "this
--- connection is closed" rather than the abandoned call's reply. See FAMILY.md, "A caller that
--- abandons a call abandons the connection".
---
--- Every call carries `from`, for a reader of the frame. The far end never believes it: who is
--- calling is read once when the connection is accepted, from the kernel and the calling
--- process's environment, and placed in the tree off the project directory.
function Session:call(name, ...)
  if not self.handle then return nil, "this connection is closed" end
  local request = { call = name, from = self.from, token = self.token }
  if select("#", ...) > 0 then request.args = { ... } end
  local reply, why = self.handle:call(request, { deadline_ms = self.timeout_ms })
  if not reply then return nil, why end
  if type(reply.family) == "number" and reply.family > FAMILY_REVISION then
    return nil, "the reply is written in family " .. reply.family .. ", newer than this library's "
      .. FAMILY_REVISION
  end
  -- A refusal is a reply, not a dropped connection: "only the session that started one may stop
  -- it" says what to fix where "connection reset" does not.
  if not reply.ok then return nil, reply.error or "the session refused the call" end
  -- `result` is a list of return values, so one Lua call answers with what the remote one did.
  local result = reply.result or {}
  return table.unpack(result, 1, reply.n or #result)
end

function Session:close()
  if self.handle then
    self.handle:close()
    self.handle = nil
  end
  return true
end

-- The verbs a session answers. Small on purpose and not a mirror of anything: most of what a
-- session knows is meaningless to a peer, and the parts that are dangerous — anything that runs
-- a command — are not here and are not coming.
--
-- Ask `verbs()` rather than trusting this list. It is here so `them.status()` reads as a call
-- and not as a string, and a session that answers something newer will say so.
local SURFACE = {
  "verbs", "client",
  "identity", "kin", "status", "inbox", "needs",
  "tell",
  -- What a session is for. Answered for the session itself and for the one that started it,
  -- and for nobody else -- and it is worth nothing to either of them: no wall, no reach and no
  -- verb reads a role, which is the only reason a session may name itself.
  "role",
  -- The adoption handshake, both halves. `adopt` puts the question and a person answers it;
  -- `adopted` is that answer coming back, and is believed only when it quotes the nonce the
  -- asking session left for it.
  "adopt", "adopted",
  -- A session's own: every other caller is refused. Attached anyway, because the caller that
  -- may ask is the harness holding this very session, and it reaches it through this library.
  "mint", "minted",
  -- The one act the far end cannot decline, so the one that has to prove itself: it carries the
  -- secret the session was started with, and a session nobody started holds none.
  "stop",
  -- The model's coordination vocabulary, the same surface `melchior tool` runs on the command
  -- line: `them.tool({ verb = "…", … })`. A session's own only, and answered as it because the far
  -- end takes who is calling from the kernel. This is how a harness reaches it through the library.
  "tool",
}

-- The verbs that answer one row per thing rather than one row that is the answer.
--
-- `result` is the rows and `n` is how many, so a listing arrives here as N return values. Gathered
-- back into a table, because `#them.inbox()` is what a caller asking for an inbox wants and is
-- what `ipairs` reads. Every other verb answers one value and is handed over untouched.
local LISTINGS = {
  verbs = true, needs = true, inbox = true,
}

--- Gather a listing verb's rows into one table, leaving every other verb alone.
local function gathered(verb, ...)
  if not LISTINGS[verb] then return ... end
  local out = table.pack(...)
  -- A refusal is `nil, why` and passes through as it stands.
  if out.n > 0 and out[1] == nil then return ... end
  out.n = nil
  return out
end

local function attach(session)
  for _, verb in ipairs(SURFACE) do
    session[verb] = function(...) return gathered(verb, session:call(verb, ...)) end
  end
  return session
end

-- ---------------------------------------------------------------- who is where

--- An environment variable, or `nil` when it is unset, empty, or the host lends no `os.getenv`.
local function env(name)
  local getenv = type(os) == "table" and os.getenv or nil
  local value = getenv and getenv(name) or nil
  if value == "" then return nil end
  return value
end

--- This user's uid, as the host says it: `<host>.uid`, a whole number or a function answering
--- one. `nil` when no host says. `$UID` is never read.
local function uid()
  for _, name in ipairs(FAMILY) do
    local host = _G[name]
    local said = type(host) == "table" and host.uid or nil
    if type(said) == "function" then said = said() end
    if type(said) == "number" and said >= 0 and said == math.floor(said) then
      return math.floor(said)
    end
  end
  return nil
end

--- The first host's `fs.stat`, or `nil` when no host lends one.
local function host_stat()
  for _, name in ipairs(FAMILY) do
    local host = _G[name]
    local fs = type(host) == "table" and host.fs or nil
    if type(fs) == "table" and type(fs.stat) == "function" then return fs.stat end
  end
  return nil
end

--- Whether the host's `stat` says `path` is `id`'s directory with no group or other bits. A host
--- that names no type is taken at its word on owner and mode alone.
local function private_dir(stat, path, id)
  if not stat then return false end
  local ok, s = pcall(stat, path)
  if not ok or type(s) ~= "table" or type(s.mode) ~= "number" or s.uid ~= id then return false end
  if s.type ~= nil and s.type ~= "directory" and s.type ~= "dir" then return false end
  return math.floor(s.mode) % 64 == 0
end

--- The runtime directory sessions live under, by the family's rule: `$XDG_RUNTIME_DIR/melchior`
--- when it is absolute and the host's `fs.stat` says it is this uid's and closed, else
--- `<tmp>/melchior-<uid>`. `nil` when no host says the uid.
local function runtime()
  local id = uid()
  if not id then return nil end
  local dir = env("XDG_RUNTIME_DIR")
  if dir and dir:sub(1, 1) == "/" and private_dir(host_stat(), dir, id) then
    return (dir:gsub("/+$", "")) .. "/melchior"
  end
  local tmp = env("TMPDIR")
  if not tmp or tmp:sub(1, 1) ~= "/" then tmp = "/tmp" end
  return (tmp:gsub("/+$", "")) .. "/melchior-" .. string.format("%d", id)
end

--- One name as one path segment, the way the session that bound it flattened it: anything that is
--- not a letter, a digit, `-` or `_` becomes `-`, so a name cannot climb out of its directory.
local function safe(name)
  local flat = tostring(name):gsub("[^%w_-]", "-")
  if flat == "" then return "-" end
  return flat
end

--- Who this process belongs to, if it belongs to a session at all.
---
--- Set by the session that started the process, and inherited from there by everything it
--- starts. It is the one thing a separate process cannot work out for itself: names are made
--- when a session starts, and nothing on disk says which of several a given process came from.
function M.me()
  local project = os.getenv("MAGI_MELCHIOR_PROJECT") or os.getenv("MAGI_PROJECT")
  local id = os.getenv("MAGI_MELCHIOR_ID") or os.getenv("MAGI_ID")
  if not project or project == "" or not id or id == "" then return nil end
  local role = os.getenv("MAGI_MELCHIOR_ROLE") or os.getenv("MAGI_ROLE")
  if not role or role == "" then role = "main" end
  return { project = project, role = role, id = id, full = project .. "/" .. role .. "/" .. id }
end

--- List a directory through the host, or not at all: plain Lua cannot, and running a shell to
--- do it is not this library's to do.
local function entries(dir)
  for _, name in ipairs(FAMILY) do
    local host = _G[name]
    if type(host) == "table" and host.fs and host.fs.ls then
      local out = {}
      for _, entry in ipairs(host.fs.ls(dir) or {}) do out[#out + 1] = entry.name end
      return out
    end
  end
  return {}
end

--- Every session listening in `project`, by id.
---
--- Read from the directory rather than from a registry somebody has to keep up to date: a
--- process that died did not get to remove itself from a list. A socket file outlives its
--- process, so a name here is not a promise that anything is behind it — [`M.answers`] is the
--- cheapest way to find out, and `list` on a session is the considered one.
---
--- An id is two Greek words and a dash, so the `.parent`, `.host`, `.pid` and `.log` files that
--- sit beside the sockets are told apart by the dot they contain.
function M.instances(project)
  project = project or (M.me() or {}).project
  if not project then return nil, "no project: pass one, or set MAGI_MELCHIOR_PROJECT" end
  local base = runtime()
  if not base then return nil, "no runtime directory: this host does not say whose uid it runs as" end
  local out = {}
  for _, name in ipairs(entries(base .. "/" .. safe(project))) do
    if not name:find("%.") and name ~= "" then out[#out + 1] = name end
  end
  table.sort(out)
  return out
end

--- Where a session's socket is.
---
--- The id is the part that places it. A role in the path would be a second key for the same
--- door, and a session that changed what it was for would move.
local function socket_of(project, id)
  local base = runtime()
  if not base then return nil end
  return base .. "/" .. safe(project) .. "/" .. safe(id)
end

--- Read a name the way a session would: `id`, `role/id`, or `project/role/id`.
---
--- From the right, because the id is the part that is always there and the rest fills in from
--- whoever is asking. `a/b/c/d` is not a deeper name, it is a typo.
local function read(name, mine)
  local parts = {}
  for part in tostring(name):gsub("^%$", ""):gmatch("[^/]+") do parts[#parts + 1] = part end
  mine = mine or M.me() or {}
  if #parts == 1 then return mine.project, parts[1] end
  if #parts == 2 then return mine.project, parts[2] end
  if #parts == 3 then return parts[1], parts[3] end
  return nil, nil
end

-- ---------------------------------------------------------------- connecting

--- Open a connection to a running session.
---
--- `where` is a name — `"iota-mu"`, `"review/iota-mu"`, `"myproject/review/iota-mu"` — or a table
--- with a `path`. Nothing means this process's own session, which is how a peer asks itself
--- what has arrived.
---
--- The connection is held. Closing after one call is the obvious way to write a client and it
--- dies on its *second* call with a broken pipe, so this hands back something with a `close`.
function M.connect(where)
  if not transport then
    return nil, "no transport: pass one to the chunk, as load(src)(host.stream)"
  end
  local mine = M.me()
  local path, named
  if type(where) == "table" and where.path then
    path = where.path
  else
    local project, id
    if where == nil then
      if not mine then return nil, "no session: name one, or set MAGI_MELCHIOR_PROJECT and MAGI_MELCHIOR_ID" end
      project, id = mine.project, mine.id
    else
      project, id = read(where, mine)
    end
    if not project or not id then
      return nil, "`" .. tostring(where) .. "` is not a name a session can have"
    end
    named = project .. "/" .. id
    path = socket_of(project, id)
    if not path then
      return nil, "no runtime directory to find " .. named
        .. " in: this host does not say whose uid it runs as"
    end
  end

  local timeout = type(where) == "table" and where.timeout_ms or nil
  local handle, why = transport.connect(path, timeout)
  if not handle then
    return nil, "nothing is listening as " .. (named or path) .. (why and (" (" .. why .. ")") or "")
  end
  if type(handle.call) ~= "function" then
    handle:close()
    return nil, "this host's stream has no `call`, and the framing is the stream's to do"
  end
  return attach(setmetatable({
    handle = handle,
    path = path,
    timeout_ms = timeout,
    -- Put on every call for a reader of the frame. The session takes who is calling from the
    -- connecting process, never from this.
    from = mine and mine.full or nil,
    -- No secret is read from the environment: one never travels there. A caller that holds one
    -- sets `session.token` itself.
    token = nil,
  }, Session))
end

--- Whether anything is actually listening as `where`.
---
--- A socket file outlives the process that made it, so the directory says who *was* there. This
--- is the cheapest question that tells a running session from a crash's leftovers.
function M.answers(where)
  local session = M.connect(where)
  if not session then return false end
  session:close()
  return true
end

-- ---------------------------------------------------------------- one question

--- Ask one thing and keep nothing.
---
--- The counterpart to `connect`: that is a channel you hold and close, this is a question. The
--- verb says what the *caller* wanted, so a call site written today still reads correctly when
--- the layer grows something to hold a connection open for.
function M.fetch(where, verb, ...)
  local session, why = M.connect(where)
  if not session then return nil, why end
  local answers = table.pack(session:call(verb, ...))
  session:close()
  return gathered(verb, table.unpack(answers, 1, answers.n))
end

--- This file's own source, as the session it is talking to has it.
---
--- `melchior lua-api` prints it, which is enough for a host that can shell out and useless to one
--- that cannot: a sandboxed VM that may not start a program has no way to run it. So a session hands the
--- library out over the wire too, and a sibling that speaks the framing can fetch the right
--- vocabulary using the wrong one, in code, with nothing written to disk.
function M.client(where)
  return M.fetch(where, "client")
end

return M
