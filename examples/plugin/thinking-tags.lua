-- A wire protocol of your own, built out of one that already exists.
--
-- This is the claim P3 makes -- "adding one wire protocol to melchior is ten lines in a new
-- file" -- written out so it can be checked. Copy it into `~/.config/melchior/plugin/`.
--
-- It is `openai-completions` with one difference: some servers wrap reasoning in `<think>` tags
-- in the ordinary content stream instead of sending it as a separate field, and a session that
-- does not know that shows the model's scratchpad to the person as if it were the answer.
--
-- `melchior.apis` is the registry, readable as well as writable, so this borrows the four
-- functions a protocol owes and replaces one of them:
--
--   endpoint(base_url, model)   where to send it
--   headers(key)                how to prove who you are
--   request(model, ctx, opts)   the neutral conversation as this dialect's payload
--   on_event(state, event)      one server-sent event as `{ scratch, usage, deltas }`
--
-- Registering an id that already exists replaces it, so naming `openai-completions` here would
-- change the shipped protocol for everything that uses it. A new name is almost always what you
-- want; `after/plugin/` is where the exception goes.

local base = melchior.apis["openai-completions"]

local M = {
  endpoint = base.endpoint,
  headers = base.headers,
  request = base.request,
}

-- The one difference: a text delta that opens with `<think>` is reasoning, not answer.
function M.on_event(state, event)
  local out = base.on_event(state, event)
  for _, delta in ipairs(out.deltas or {}) do
    if delta.kind == "text" and type(delta.text) == "string" then
      local thought = delta.text:match("^<think>(.-)</think>")
      if thought then
        delta.kind = "reasoning"
        delta.text = thought
      end
    end
  end
  return out
end

melchior.api("openai-completions-thinking", M)
