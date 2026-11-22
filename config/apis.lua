-- Every wire protocol magi speaks.
--
-- Each block is one dialect and stands alone: a `do ... end` so its locals stay its own,
-- ending in the `melchior.api` calls that register it. A vendor deviating inside a dialect says
-- so in its `compat` block in `providers.lua`, not here.

-- What a reply may run to when nothing says otherwise. Not the model's ceiling: a router holds
-- credit for every token asked for, and a million-token ceiling priced like Kimi's is a dozen
-- dollars held against a one-line answer. `max_tokens` in the config says otherwise.
local OUTPUT = 32000
local RESPONSES_STATE = "melchior.responses.v1:"
local ANTHROPIC_STATE = "melchior.anthropic.v1:"
local GOOGLE_STATE = "melchior.google.v1:"
local chat_event

local function opaque_state(signature)
  return type(signature) == "string" and (signature:sub(1, #RESPONSES_STATE) == RESPONSES_STATE
    or signature:sub(1, #ANTHROPIC_STATE) == ANTHROPIC_STATE
    or signature:sub(1, #GOOGLE_STATE) == GOOGLE_STATE)
end

local function same_state(a, b)
  if type(a) ~= type(b) then return false end
  if type(a) ~= "table" then return melchior.json.encode(a) == melchior.json.encode(b) end
  local count, other = 0, 0
  for key, value in pairs(a) do
    count = count + 1
    if not same_state(value, b[key]) then return false end
  end
  for _ in pairs(b) do other = other + 1 end
  if count == 0 then return melchior.json.encode(a) == melchior.json.encode(b) end
  return count == other
end

local function continuation_origin(model, opts)
  if not model.__endpoint or type(opts.__credential_binding) ~= "string" then return nil end
  return { provider = model.provider, model = model.id,
    api = model.__protocol, endpoint = model.__endpoint, credential = opts.__credential_binding }
end

local function output(model, opts)
  local explicit = not opts.max_tokens_automatic and opts.max_tokens or nil
  local chosen = explicit or melchior.max_tokens
  local limit = math.min(chosen or OUTPUT, model.max_tokens)
  if not chosen and opts.max_tokens then limit = math.min(limit, opts.max_tokens) end
  return limit, chosen == nil
end

local function prepared(body, model, opts, keys)
  local limit, automatic = output(model, opts)
  local thinking = (opts.thinking and opts.thinking ~= "off") or (opts.__google_control and opts.__google_control.active)
  local minimum = thinking and limit or 1
  local reduced = opts.__output_limit
  if automatic and reduced and reduced >= minimum and reduced < limit then limit = reduced end
  local target = body
  for i = 1, #keys - 1 do target = target[keys[i]] end
  target[keys[#keys]] = limit
  body.__budget = {
    version = 1, automatic = automatic, effective = limit, minimum = minimum,
    path = "/" .. table.concat(keys, "/"),
  }
  return body
end

local function token_count(value)
  if type(value) == "number" and value >= 0 and value < 18446744073709551616.0
      and value % 1 == 0 then return value end
end

local function measured_usage(raw, fields)
  local measured = {}
  if type(raw) ~= "table" then return measured end
  for field, key in pairs(fields) do measured[field] = token_count(raw[key]) end
  local cost = type(raw.cost) == "number" and raw.cost >= 0 and raw.cost * 1000000
  measured.cost_micros = cost and token_count(math.floor(cost + 0.5)) or nil
  return measured
end

local function inclusive_usage(previous, raw, total_key, output_key, details_key, cache_key, alternate, known, evidence)
  known = token_count(known)
  if type(raw) ~= "table" then return previous, 1, known, {} end
  local anomalies = 0
  local function count(value)
    local n = token_count(value)
    if value ~= nil and n == nil then anomalies = anomalies + 1 end
    return n
  end
  local details = raw
  if details_key then
    details = raw[details_key]
    if details ~= nil and type(details) ~= "table" then anomalies = anomalies + 1 end
    details = type(details) == "table" and details or {}
  end
  local total, output = count(raw[total_key]), count(raw[output_key])
  local nested, other = count(details[cache_key]), alternate and count(raw[alternate]) or nil
  local read = nested or other
  if nested and other then read = math.max(nested, other) end
  local reported_read = read
  local measured = measured_usage(raw, { output = output_key })
  local usage = {
    input = previous.input, output = output or previous.output,
    cache_read = previous.cache_read, cache_write = previous.cache_write,
    cost_micros = previous.cost_micros,
  }
  if total and known and total < known then
    anomalies = anomalies + 1
    read = previous.cache_read
  else known = total or known end
  if known then
    read = math.max(read or 0, previous.cache_read)
    if read > known then anomalies = anomalies + 1 end
    usage.cache_read = math.min(read, known)
    usage.input = known - usage.cache_read
  elseif read then usage.cache_read = read end
  if raw.cost ~= nil then
    local cost = type(raw.cost) == "number" and raw.cost >= 0 and raw.cost * 1000000
    local micros = cost and token_count(math.floor(cost + 0.5))
    if micros then usage.cost_micros = micros else anomalies = anomalies + 1 end
  end
  if reported_read == usage.cache_read then measured.cache_read = reported_read end
  local retained_read = evidence and token_count(evidence.cache_read)
  if known and (measured.cache_read == usage.cache_read or retained_read == usage.cache_read) then
    measured.input = usage.input
  end
  return usage, anomalies, known, measured
end

-- A schema as a provider that checks one will take it: without the `x-` hints a caller writes
-- for the `decisions` protocol, which a strict checker refuses as words it does not know.
local function plain(schema)
  -- An empty one is handed back as it came: a copy would not say whether it was a list.
  if type(schema) ~= "table" or next(schema) == nil then return schema end
  local out = {}
  for key, value in pairs(schema) do
    if not (type(key) == "string" and key:sub(1, 2) == "x-") then out[key] = plain(value) end
  end
  return out
end

do -- openai-completions
  local M = {}

  function M.endpoint(base_url, model, request)
    if model.compat.chat_strict_dialect == "deepseek" and request and type(request.tools) == "table" then
      for _, tool in ipairs(request.tools) do
        if tool["function"] and tool["function"].strict == true then
          if base_url:find("[?#]") then error("DeepSeek strict routing requires a plain base URL") end
          base_url = base_url:gsub("/v1$", ""):gsub("/beta$", "") .. "/beta"
          break
        end
      end
    end
    return base_url .. "/chat/completions"
  end

  function M.headers(key)
    if not key then return {} end
    return { authorization = "Bearer " .. key }
  end

  local function declared(tools)
    local out = melchior.json.array()
    for _, t in ipairs(tools) do
      out[#out + 1] = { type = "function", ["function"] = {
        name = t.name, description = t.description, parameters = t.parameters, strict = t.__strict,
      } }
    end
    return out
  end

  -- One neutral message as one or more Completions messages.
  local function convert(m, out, compat, attachments, reasons)
    local text, parts, tool_calls, results, reasoning = {}, {}, {}, {}, nil

    for _, c in ipairs(m.content or {}) do
      if c.type == "text" then
        if c.text ~= "" then
          text[#text + 1] = c.text
          parts[#parts + 1] = { type = "text", text = c.text }
        end
      elseif c.type == "thinking" then
        reasoning = (reasoning or "") .. (c.thinking or "")
      elseif c.type == "image" then
        parts[#parts + 1] = { type = "image_url", image_url = {
          url = "data:" .. c.media_type .. ";base64," .. c.data,
        } }
      elseif c.type == "tool_call" then
        tool_calls[#tool_calls + 1] = {
          id = c.id, type = "function",
          ["function"] = { name = c.name, arguments = melchior.json.encode(c.arguments) },
        }
      elseif c.type == "tool_result" then
        local r = { role = "tool", tool_call_id = c.id, content = c.content }
        -- Some dialects reject a result that does not repeat the tool's name.
        if compat.requires_tool_result_name then r.name = c.name end
        results[#results + 1] = r
        if c.images and #c.images > 0 then
          attachments[#attachments + 1] = { type = "text", text = "Images from tool " .. c.name .. " (" .. c.id .. "):" }
          for _, image in ipairs(c.images) do
            attachments[#attachments + 1] = { type = "image_url", image_url = {
              url = "data:" .. image.media_type .. ";base64," .. image.data,
            } }
          end
        end
      end
    end

    for _, r in ipairs(results) do out[#out + 1] = r end
    if #results > 0 then return end

    local role = m.role
    if role == "assistant" then
      local msg = { role = "assistant", content = table.concat(text, "") }
      if #tool_calls > 0 then msg.tool_calls = tool_calls end
      -- A dialect that cannot carry reasoning gets it inline, delimited, rather than losing it.
      if reasoning then
        if compat.requires_thinking_as_text then
          msg.content = "<thinking>" .. reasoning .. "</thinking>" .. msg.content
        else
          local field = "reasoning_content"
          for _, block in ipairs(m.content or {}) do
            local signature = block.signature
            if block.type == "thinking" and (signature == "reasoning" or signature == "reasoning_content" or signature == "reasoning_text") then
              field = signature
              break
            end
          end
          msg[field] = reasoning
        end
      end
      if reasons and compat.requires_reasoning_content_on_assistant_messages and msg.reasoning_content == nil then
        msg.reasoning_content = ""
      end
      out[#out + 1] = msg
    elseif role == "system" then
      for _, c in ipairs(m.content or {}) do
        if c.type == "tool_changes" and c.added and #c.added > 0 then
          out[#out + 1] = { role = "system", tools = declared(c.added) }
        end
      end
      if #text > 0 then
        out[#out + 1] = {
          role = compat.supports_developer_role and "developer" or "system", content = table.concat(text, ""),
        }
      end
    else
      local has_image = false
      for _, p in ipairs(parts) do if p.type == "image_url" then has_image = true end end
      out[#out + 1] = { role = "user", content = has_image and parts or table.concat(text, "") }
    end
  end

  function M.request(model, ctx, opts)
    -- Every field is answered before this file sees it: the host resolves the catalog's sparse
    -- `compat` against the conservative defaults. So `compat.x` is safe and `compat.x or <a
    -- default>` would be a second copy of a decision that already has a home.
    local compat = model.compat
    local messages = melchior.json.array()

    if ctx.system then
      -- `developer` is an OpenAI extension most copies reject, so it is opted into.
      messages[#messages + 1] = {
        role = compat.supports_developer_role and "developer" or "system",
        content = ctx.system,
      }
    end
    local attachments = {}
    local function flush()
      if #attachments > 0 then
        messages[#messages + 1] = { role = "user", content = attachments }
        attachments = {}
      end
    end
    for _, m in ipairs(ctx.messages or {}) do
      if m.role ~= "tool" then flush() end
      convert(m, messages, compat, attachments, model.reasoning)
    end
    flush()

    local body = { model = model.id, stream = true, messages = messages }

    -- Which field caps the response is the single most common way these dialects differ.
    local field = compat.max_tokens_field
    body[field] = output(model, opts)

    if compat.supports_store then body.store = false end
    -- Usage is not reported while streaming unless asked for, and some dialects cannot.
    if compat.supports_usage_in_streaming then
      body.stream_options = { include_usage = true }
    end
    -- OpenRouter says what a request cost when asked. Only it is asked: the field is its own.
    if model.provider == "openrouter" then
      body.usage = { include = true }
      -- A provider the person chose is asked first; the rest stay behind it as fallbacks.
      if opts.provider then body.provider = { order = { opts.provider } } end
      -- Upstreams this provider is never served by, fallbacks included.
      if opts.avoid and #opts.avoid > 0 then
        body.provider = body.provider or {}
        body.provider.ignore = opts.avoid
      end
    end

    if ctx.tools and #ctx.tools > 0 then
      body.tools = declared(ctx.tools)
      if compat.zai_tool_stream then body.tool_stream = true end
    end
    if compat.zai_tool_stream and not body.tool_stream then
      for _, message in ipairs(messages) do
        if message.tools and #message.tools > 0 then body.tool_stream = true; break end
      end
    end

    if opts.schema then
      body.response_format = {
        type = "json_schema",
        json_schema = { name = opts.schema.name, schema = plain(opts.schema.schema), strict = true },
      }
    end

    local level = opts.thinking
    if compat.thinking_format == "baseten" then
      if model.reasoning and opts.__thinking_enabled ~= nil then
        local enabled = opts.__thinking_enabled
        local toggle = compat.chat_template_args.enable_thinking
        if type(toggle) == "boolean" then
          body.chat_template_args = { enable_thinking = toggle }
        elseif type(toggle) == "table" and not (toggle.omitWhenOff and not enabled) then
          body.chat_template_args = { enable_thinking = enabled }
        end
        local effort = level or (not enabled and model.thinking and model.thinking.off)
        if compat.supports_reasoning_effort and type(effort) == "string" then body.reasoning_effort = effort end
      end
    elseif compat.thinking_format == "ant-ling" then
      if model.reasoning and level then body.reasoning = { effort = level } end
    elseif compat.thinking_format == "together" then
      if model.reasoning and level then
        body.reasoning = { enabled = level ~= "off" }
        if level ~= "off" and compat.supports_reasoning_effort then body.reasoning_effort = level end
      end
    elseif compat.thinking_format == "zai" then
      if model.reasoning then
        if level == "off" then
          body.thinking = { type = "disabled" }
        else
          body.thinking = { type = "enabled", clear_thinking = false }
          if level and compat.supports_reasoning_effort then body.reasoning_effort = level end
        end
      end
    elseif level == "off" then
      -- Said out loud: a model that reasons by default goes on reasoning unless told not to.
      local format = compat.thinking_format
      if format == "openrouter" then
        body.reasoning = { enabled = false }
      elseif format == "deepseek" then
        body.thinking = { type = "disabled" }
      elseif format == "qwen" then
        body.enable_thinking = false
      end
    elseif level then
      local format = compat.thinking_format
      if format == "openrouter" then
        body.reasoning = { effort = level }
      elseif format == "deepseek" then
        body.thinking = { type = "enabled" }
        if compat.supports_reasoning_effort then body.reasoning_effort = level end
      elseif format == "qwen" then
        body.enable_thinking = true
        if compat.supports_reasoning_effort then body.reasoning_effort = level end
      elseif compat.supports_reasoning_effort then
        body.reasoning_effort = level
      end
    end
    if model.reasoning and compat.thinking_format == "deepseek" and compat.moonshot_keep_thinking and level ~= "off" then
      body.thinking = { type = "enabled", keep = "all" }
    end
    if compat.supports_prompt_cache_key and opts.__session_id then body.prompt_cache_key = opts.__session_id end
    return prepared(body, model, opts, { field })
  end

  local FINISH = { tool_calls = "tool_use", length = "length", repetition_truncation = "length",
    stop = "end_turn", content_filter = "error", error = "error" }
  local MAX_CALLS, MAX_STATE, MAX_ID = 256, 1024 * 1024, 4096

  local function absent(value)
    return value == nil or value == melchior.json.null
  end

  local function shaped(value, first)
    return type(value) == "table" and melchior.json.encode(value):sub(1, 1) == first
  end

  local function index(value)
    return type(value) == "number" and value >= 0 and value < MAX_CALLS and value % 1 == 0
  end

  local function grow(s, slot, field, value, identity)
    if absent(value) or value == "" then return end
    if type(value) ~= "string" or (identity and (#value > MAX_ID or (slot[field] and slot[field] ~= value))) then
      s.failed = true; return
    end
    if identity and slot[field] then return end
    if s.bytes + #value > MAX_STATE then s.failed = true; return end
    s.bytes = s.bytes + #value
    slot[field] = identity and value or (slot[field] .. value)
  end

  local function collect(s, call)
    if s.failed then return end
    if not shaped(call, "{") or (not absent(call.type) and call.type ~= "function")
        or not absent(call.custom) then s.failed = true; return end
    if s.mistral then
      if call.id == "null" or absent(call.id) then call.id = nil end
      if absent(call.index) then call.index = nil end
      if type(call["function"]) == "table" and type(call["function"].arguments) == "table" then
        if not shaped(call["function"].arguments, "{") then s.failed = true; return end
        call["function"].arguments = melchior.json.encode(call["function"].arguments)
      end
    end
    local at = call.index
    if at == nil then
      if type(call.id) == "string" and call.id ~= "" then
        at = s.ids[call.id]
        if at == nil then
          at = s.max == 0 and not s.slots["0"].id and 0 or s.max + 1
        end
      elseif s.max <= 0 then at = 0
      else s.failed = true; return end
    end
    if not index(at) then s.failed = true; return end
    local key = tostring(at)
    local slot = s.slots[key]
    if not slot then
      slot = { arguments = "", sent = 0 }
      s.slots[key], s.max = slot, math.max(s.max, at)
    end
    grow(s, slot, "id", call.id, true)
    local f = call["function"]
    if not absent(f) then
      if not shaped(f, "{") then s.failed = true; return end
      grow(s, slot, "name", f.name, true)
      grow(s, slot, "arguments", f.arguments, false)
    end
    if slot.id then
      if s.ids[slot.id] ~= nil and s.ids[slot.id] ~= at then s.failed = true; return end
      s.ids[slot.id] = at
    end
  end

  local function drain(s, deltas, closed)
    if s.failed then return end
    while s.next <= s.max do
      local slot = s.slots[tostring(s.next)]
      if not slot or not slot.id or not slot.name then return end
      if not slot.started then
        deltas[#deltas + 1] = { kind = "tool_call_start", id = slot.id, name = slot.name }
        slot.started = true
      end
      if #slot.arguments > slot.sent then
        deltas[#deltas + 1] = { kind = "tool_call_args", arguments = slot.arguments:sub(slot.sent + 1) }
        slot.sent = #slot.arguments
      end
      if not closed then return end
      s.next = s.next + 1
    end
  end

  local function finish(scratch, s, deltas, reason)
    if scratch.stopped then return end
    if s.mistral then
      local next_id = 0
      for at = 0, s.max do
        local slot = s.slots[tostring(at)]
        if slot and not slot.id then
          local id
          repeat id = string.format("m%08d", next_id); next_id = next_id + 1 until not s.ids[id]
          slot.id, s.ids[id] = id, at
        end
      end
    end
    if reason == "tool_use" then
      if s.max < 0 then s.failed = true end
      for at = 0, s.max do
        local slot = s.slots[tostring(at)]
        if not slot or not slot.id or not slot.name then s.failed = true; break end
        local ok, value = pcall(melchior.json.decode_value, slot.arguments)
        if not ok or value == nil then s.failed = true; break end
      end
    elseif reason == "end_turn" and s.max >= 0 then s.failed = true end
    drain(s, deltas, true)
    scratch.stopped = true
    deltas[#deltas + 1] = { kind = "stop", reason = s.failed and "error" or reason }
    s.slots, s.ids = {}, {}
  end

  local function mistral_content(stream, deltas, content)
    local function emit(kind, text)
      if type(text) ~= "string" then stream.failed = true; return end
      if text ~= "" then deltas[#deltas + 1] = { kind = kind, [kind] = text } end
    end
    if absent(content) then return end
    if type(content) == "string" then emit("text", content); return end
    if not shaped(content, "[") then stream.failed = true; return end
    for _, part in ipairs(content) do
      if not shaped(part, "{") then stream.failed = true
      elseif part.type == "text" then emit("text", part.text)
      elseif part.type == "thinking" and shaped(part.thinking, "[") then
        for _, thought in ipairs(part.thinking) do
          if shaped(thought, "{") and thought.type == "text" then emit("thinking", thought.text)
          else stream.failed = true end
        end
      else stream.failed = true end
    end
  end

  local function mistral_usage(raw)
    if type(raw) ~= "table" then return raw end
    local cache
    for _, fields in ipairs({ {"promptTokensDetails", "cachedTokens"}, {"prompt_tokens_details", "cached_tokens"},
        {"promptTokenDetails", "cachedTokens"}, {"prompt_token_details", "cached_tokens"} }) do
      local details = raw[fields[1]]
      if type(details) == "table" and not absent(details[fields[2]]) then cache = details[fields[2]]; break end
    end
    if cache == nil and not absent(raw.numCachedTokens) then cache = raw.numCachedTokens end
    if cache == nil and not absent(raw.num_cached_tokens) then cache = raw.num_cached_tokens end
    raw.prompt_tokens_details = { cached_tokens = cache }
    return raw
  end

  function M.on_event(state, event, mistral)
    local scratch, usage, deltas, anomalies = state.scratch or {}, state.usage, {}, 0
    scratch.completions = scratch.completions or { slots = {}, ids = {}, next = 0, max = -1, bytes = 0 }
    local stream = scratch.completions
    stream.mistral = mistral
    if event.data == "[DONE]" then
      finish(scratch, stream, deltas, stream.max >= 0 and "tool_use" or "end_turn")
      return { scratch = scratch, usage = usage, deltas = deltas }
    end
    local ok, d = pcall(melchior.json.decode_value, event.data)
    if not ok or not shaped(d, "{") then
      stream.failed = true
      return { scratch = scratch, usage = usage }
    end

    if d.usage ~= nil then
      local measured
      if mistral then d.usage = mistral_usage(d.usage) end
      usage, anomalies, scratch.usage_total, measured = inclusive_usage(usage, d.usage, "prompt_tokens", "completion_tokens",
          "prompt_tokens_details", "cached_tokens", "prompt_cache_hit_tokens", scratch.usage_total, state.measured_usage)
      deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured }
    end

    if scratch.stopped then return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies } end
    if not absent(d.error) then stream.failed = true end
    local choices = d.choices
    if mistral and absent(choices) then stream.failed = true end
    if not absent(choices) and (not shaped(choices, "[") or #choices > 1) then stream.failed = true end
    if type(choices) == "table" and #choices > 0 then
      local choice = choices[1]
      if not shaped(choice, "{") or (not absent(choice.index) and choice.index ~= 0) then
        stream.failed = true; choice = {}
      end
      local delta = absent(choice.delta) and {} or choice.delta
      if type(delta) ~= "table" or (not absent(choice.delta) and not shaped(delta, "{")) then
        stream.failed = true; delta = {}
      end
      if not absent(choice.message) or not absent(delta.function_call)
          or (not absent(delta.role) and delta.role ~= "assistant") then stream.failed = true end
      local reasoned, field
      for _, name in ipairs({ "reasoning_content", "reasoning", "reasoning_text" }) do
        if not absent(delta[name]) and type(delta[name]) ~= "string" then stream.failed = true end
        if type(delta[name]) == "string" and delta[name] ~= "" then
          reasoned, field = delta[name], name
          break
        end
      end
      if reasoned and reasoned ~= "" then
        deltas[#deltas + 1] = { kind = "thinking", thinking = reasoned }
        if not scratch.reasoning_field then
          scratch.reasoning_field = field
          deltas[#deltas + 1] = { kind = "signature", signature = field }
        end
      end
      if mistral then mistral_content(stream, deltas, delta.content)
      elseif not absent(delta.content) and type(delta.content) ~= "string" then stream.failed = true
      elseif type(delta.content) == "string" and delta.content ~= "" then
        deltas[#deltas + 1] = { kind = "text", text = delta.content }
      end
      if not absent(delta.tool_calls) then
        if shaped(delta.tool_calls, "[") then
          for _, call in ipairs(delta.tool_calls) do collect(stream, call) end
        else stream.failed = true end
      end
      drain(stream, deltas, false)
      if not absent(choice.finish_reason) then
        finish(scratch, stream, deltas, mistral and choice.finish_reason == "model_length" and "length"
          or FINISH[choice.finish_reason] or "error")
      end
    end

    return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies }
  end

  melchior.api("openai-completions", M)
  chat_event = M.on_event
end

do -- mistral-conversations
  local M = {}

  function M.endpoint(base_url, _model)
    return base_url:gsub("/v1$", "") .. "/v1/chat/completions"
  end

  function M.headers(key)
    return { authorization = key and "Bearer " .. key or nil, accept = "text/event-stream" }
  end

  local function call_ids(messages)
    local mapped, used = {}, {}
    for _, message in ipairs(messages or {}) do
      for _, part in ipairs(message.content or {}) do
        if (part.type == "tool_call" or part.type == "tool_result") and type(part.id) == "string"
            and #part.id == 9 and part.id:match("^[A-Za-z0-9]+$") then
          mapped[part.id], used[part.id] = part.id, true
        end
      end
    end
    return function(id)
      if mapped[id] then return mapped[id] end
      local hash = 0
      for i = 1, #id do hash = (hash * 131 + id:byte(i)) % 2147483647 end
      local encoded = string.format("m%08x", hash)
      while used[encoded] do hash = (hash + 1) % 2147483647; encoded = string.format("m%08x", hash) end
      mapped[id], used[encoded] = encoded, true
      return encoded
    end
  end

  local function image(part)
    return { type = "image_url", image_url = "data:" .. part.media_type .. ";base64," .. part.data }
  end

  local function message(m, out, id)
    local parts, calls, results = melchior.json.array(), melchior.json.array(), {}
    for _, part in ipairs(m.content or {}) do
      if part.type == "text" and part.text ~= "" then
        parts[#parts + 1] = { type = "text", text = part.text }
      elseif part.type == "thinking" and part.thinking ~= "" then
        local last = parts[#parts]
        if last and last.type == "thinking" then
          last.thinking[1].text = last.thinking[1].text .. part.thinking
        else parts[#parts + 1] = { type = "thinking", thinking = { { type = "text", text = part.thinking } } } end
      elseif part.type == "image" then parts[#parts + 1] = image(part)
      elseif part.type == "tool_call" then
        calls[#calls + 1] = { id = id(part.id), type = "function", index = #calls,
          ["function"] = { name = part.name, arguments = melchior.json.encode(part.arguments) } }
      elseif part.type == "tool_result" then
        local text = part.content
        if text == "" then text = part.images and #part.images > 0 and "(see attached image)" or "(no tool output)" end
        if part.is_error then text = "[tool error] " .. text end
        local content = { { type = "text", text = text } }
        for _, attached in ipairs(part.images or {}) do content[#content + 1] = image(attached) end
        results[#results + 1] = { role = "tool", name = part.name, tool_call_id = id(part.id), content = content }
      end
    end
    if #parts > 0 or #calls > 0 then
      out[#out + 1] = { role = m.role, content = #parts > 0 and parts or nil,
        tool_calls = #calls > 0 and calls or nil }
      if m.role == "assistant" then out[#out].prefix = false end
    end
    for _, result in ipairs(results) do out[#out + 1] = result end
  end

  function M.request(model, ctx, opts)
    local messages = melchior.json.array()
    if ctx.system then messages[#messages + 1] = { role = "system", content = ctx.system } end
    local id = call_ids(ctx.messages)
    for _, m in ipairs(ctx.messages or {}) do message(m, messages, id) end
    local body = { model = model.id, stream = true, messages = messages }
    if ctx.tools and #ctx.tools > 0 then
      body.tools = {}
      for _, tool in ipairs(ctx.tools) do
        body.tools[#body.tools + 1] = { type = "function", ["function"] = {
          name = tool.name, description = tool.description, parameters = tool.parameters, strict = tool.__strict,
        } }
      end
    end
    if opts.schema then
      body.response_format = { type = "json_schema",
        json_schema = { name = opts.schema.name, schema = plain(opts.schema.schema), strict = true } }
    end
    if model.reasoning and opts.thinking and opts.thinking ~= "off" then
      if next(model.thinking or {}) then body.reasoning_effort = opts.thinking
      elseif opts.thinking ~= "none" then body.prompt_mode = "reasoning" end
    end
    if model.compat.supports_prompt_cache_key and opts.__session_id then body.prompt_cache_key = opts.__session_id end
    if opts.thinking == "none" then opts.thinking = "off" end
    return prepared(body, model, opts, { "max_tokens" })
  end

  function M.on_event(state, event) return chat_event(state, event, true) end

  melchior.api("mistral-conversations", M)
end

do -- openai-responses
  local M = {}

  local function tick_micros(value)
    local digits = melchior.json.encode(value)
    if not digits or not digits:match("^%d+$") or #digits > 20
        or (#digits == 20 and digits > "18446744073709551615") then return nil end
    local whole = #digits > 4 and tonumber(digits:sub(1, -5)) or 0
    local remainder = tonumber(digits:sub(-4))
    return whole + (remainder >= 5000 and 1 or 0)
  end
  local PREFIX, MAX_STATE, MAX_ITEMS = RESPONSES_STATE, 1048576, 256

  local equal, origin = same_state, continuation_origin

  local function projection(items)
    if type(items) ~= "table" or #items > MAX_ITEMS then return nil end
    local encoded = melchior.json.encode(items)
    if not encoded or encoded:sub(1, 1) ~= "[" then return nil end
    local text, calls = {}, melchior.json.array()
    for _, item in ipairs(items) do
      if type(item) ~= "table" then return nil end
      if item.type == "message" and item.role == "assistant" and type(item.content) == "table" then
        for _, part in ipairs(item.content) do
          if type(part) ~= "table" then return nil end
          local value = part.type == "output_text" and part.text or part.type == "refusal" and part.refusal
          if type(value) ~= "string" then return nil end
          text[#text + 1] = value
        end
      elseif item.type == "function_call" and type(item.call_id) == "string"
          and type(item.name) == "string" and type(item.arguments) == "string" then
        if item.namespace ~= nil and item.namespace ~= melchior.json.null then return nil end
        local arguments = melchior.json.decode_value(item.arguments)
        if arguments == nil then return nil end
        calls[#calls + 1] = { id = item.call_id, name = item.name, arguments = arguments }
      elseif item.type ~= "reasoning" or type(item.encrypted_content) ~= "string"
          or item.encrypted_content == "" then return nil end
    end
    return { text = table.concat(text), calls = calls }
  end

  local function continuation(message, bound)
    if message.role ~= "assistant" or not bound then return nil end
    local signature, text, calls = nil, {}, melchior.json.array()
    for _, block in ipairs(message.content or {}) do
      if block.type == "thinking" then
        if signature then return nil end
        signature = block.signature
      elseif block.type == "text" then text[#text + 1] = block.text
      elseif block.type == "tool_call" then
        calls[#calls + 1] = { id = block.id, name = block.name, arguments = block.arguments }
      else return nil end
    end
    if type(signature) ~= "string" or #signature > MAX_STATE
        or signature:sub(1, #PREFIX) ~= PREFIX then return nil end
    local saved = melchior.json.decode_value(signature:sub(#PREFIX + 1))
    if type(saved) ~= "table" or saved.version ~= 1
        or not equal(saved.origin, bound) then return nil end
    local expected = { text = table.concat(text), calls = calls }
    if not equal(projection(saved.items), expected) then return nil end
    return saved.items
  end

  -- Input items, which is Responses' name for the conversation. Tool calls and their results are
  -- items of their own rather than blocks inside a message, which is the shape difference from
  -- Completions that catches people out.
  local function inputs(messages, bound)
    local out = melchior.json.array()
    for _, m in ipairs(messages or {}) do
      local replay = continuation(m, bound)
      if replay then
        for _, item in ipairs(replay) do out[#out + 1] = item end
      else
      local text, images, calls, results = {}, {}, {}, {}
      for _, c in ipairs(m.content or {}) do
        if c.type == "text" then
          if c.text ~= "" then text[#text + 1] = c.text end
        elseif c.type == "image" then
          images[#images + 1] = { type = "input_image", detail = "auto",
            image_url = "data:" .. c.media_type .. ";base64," .. c.data }
        elseif c.type == "tool_call" then
          calls[#calls + 1] = {
            type = "function_call", call_id = c.id, name = c.name,
            arguments = melchior.json.encode(c.arguments),
          }
        elseif c.type == "tool_result" then
          local output = c.content
          if c.images and #c.images > 0 then
            output = { { type = "input_text", text = c.content } }
            for _, image in ipairs(c.images) do
              output[#output + 1] = { type = "input_image", detail = "auto",
                image_url = "data:" .. image.media_type .. ";base64," .. image.data }
            end
          end
          results[#results + 1] = {
            type = "function_call_output", call_id = c.id, output = output,
          }
        end
      end

      for _, r in ipairs(results) do out[#out + 1] = r end
      if #results == 0 and (#text > 0 or #images > 0) then
        local role = (m.role == "assistant") and "assistant" or "user"
        -- The content part is named for its direction: what the model produced is `output_text`
        -- and what it was given is `input_text`, and swapping them is rejected.
        local part = (role == "assistant") and "output_text" or "input_text"
        local content = { { type = part, text = table.concat(text, "") } }
        for _, image in ipairs(images) do content[#content + 1] = image end
        out[#out + 1] = {
          type = "message", role = role,
          content = content,
        }
      end
      for _, c in ipairs(calls) do out[#out + 1] = c end
      end
    end
    return out
  end

  function M.request(model, ctx, opts)
    local bound = origin(model, opts)
    local body = {
      model = model.id,
      stream = true,
      store = false,
      input = inputs(ctx.messages, bound),
      max_output_tokens = output(model, opts),
    }
    if bound then body.__scratch = { continuation_origin = bound } end
    if model.reasoning then body.include = { "reasoning.encrypted_content" } end
    if ctx.system then body.instructions = ctx.system end

    -- Responses moved it under `text.format`, and flattened the schema up a level.
    if opts.schema then
      body.text = {
        format = {
          type = "json_schema",
          name = opts.schema.name,
          schema = plain(opts.schema.schema),
          strict = true,
        },
      }
    end

    if ctx.tools and #ctx.tools > 0 then
      local tools = {}
      for _, t in ipairs(ctx.tools) do
        tools[#tools + 1] = {
          type = "function", name = t.name,
          description = t.description, parameters = t.parameters, strict = t.__strict,
        }
      end
      body.tools = tools
    end

    local level = opts.thinking
    if level and level ~= "off" then
      body.reasoning = { effort = level, summary = "auto" }
    end
    return prepared(body, model, opts, { "max_output_tokens" })
  end

  local INCOMPLETE = { max_output_tokens = "length" }

  local function index(value)
    return type(value) == "number" and value >= 0 and value < MAX_ITEMS and value % 1 == 0
  end

  local function spend(s, bytes)
    s.bytes = s.bytes + bytes
    if s.bytes > MAX_STATE then s.failed = true end
    return not s.failed
  end

  local function touch(s, at, item)
    if s.failed then return nil end
    if not index(at) or type(item) ~= "table" or s.legacy then s.failed = true; return nil end
    local kind = item.type
    if kind ~= "function_call" and kind ~= "message" and kind ~= "reasoning" then
      s.failed = true; return nil
    end
    if kind == "function_call" and item.namespace ~= nil and item.namespace ~= melchior.json.null then
      s.failed = true; return nil
    end
    if kind == "message" and item.role ~= "assistant" then s.failed = true; return nil end
    local key = tostring(at)
    local slot = s.slots[key]
    if slot and slot.kind ~= kind then s.failed = true; return nil end
    if not slot then
      slot = { kind = kind, groups = {}, order = {}, group = 1,
        arguments = { value = "", sent = 0 } }
      s.slots[key], s.max, s.indexed = slot, math.max(s.max, at), true
    end
    for _, field in ipairs({ "id", "call_id", "name" }) do
      local value = item[field]
      if value ~= nil then
        if type(value) ~= "string" or value == "" or (slot[field] and slot[field] ~= value) then
          s.failed = true; return nil
        end
        if not slot[field] then
          if not spend(s, #value) then return nil end
          slot[field] = value
        end
      end
    end
    for _, field in ipairs({ "id", "call_id" }) do
      local value = slot[field]
      local identity = value and (field .. ":" .. value)
      if identity then
        if s.ids[identity] and s.ids[identity] ~= key then s.failed = true; return nil end
        s.ids[identity] = key
      end
    end
    if kind == "function_call" and (not slot.call_id or not slot.name) then
      s.failed = true; return nil
    end
    return slot
  end

  local function grow(s, part, value, done)
    if s.failed then return end
    if type(value) ~= "string" then s.failed = true; return end
    local added = value
    if done then
      if value:sub(1, #part.value) ~= part.value then s.failed = true; return end
      added = value:sub(#part.value + 1)
    end
    if part.done and #added > 0 then s.failed = true; return end
    if not spend(s, #added) then return end
    part.value = part.value .. added
    part.done = part.done or done
  end

  local function part(s, slot, channel, at)
    if not index(at) then s.failed = true; return nil end
    local group = slot.groups[channel]
    if not group then
      group = { parts = {}, next = 0, max = -1 }
      slot.groups[channel] = group
      slot.order[#slot.order + 1] = channel
    end
    local key = tostring(at)
    if not group.parts[key] then
      if slot.done then s.failed = true; return nil end
      s.parts = s.parts + 1
      if s.parts > MAX_ITEMS then s.failed = true; return nil end
      group.parts[key] = { value = "", sent = 0 }
      group.max = math.max(group.max, at)
    end
    return group.parts[key]
  end

  local function full_parts(s, slot, channel, contents)
    if contents == nil then return end
    if type(contents) ~= "table" then s.failed = true; return end
    local encoded = melchior.json.encode(contents)
    if not encoded or encoded:sub(1, 1) ~= "[" then s.failed = true; return end
    for n, content in ipairs(contents) do
      if type(content) ~= "table" then s.failed = true; return end
      local accepted = channel == "text" and (content.type == "output_text" or content.type == "refusal")
        or channel == "summary" and content.type == "summary_text"
        or channel == "reasoning" and content.type == "reasoning_text"
      if not accepted then s.failed = true; return end
      local value = content.text
      if content.type == "refusal" then value = content.refusal end
      local p = part(s, slot, channel, n - 1)
      if not p then return end
      grow(s, p, value, true)
    end
  end

  local function close_item(s, at, item)
    local slot = touch(s, at, item)
    if not slot then return end
    if slot.kind == "function_call" then grow(s, slot.arguments, item.arguments, true)
    elseif slot.kind == "message" then full_parts(s, slot, "text", item.content)
    else
      full_parts(s, slot, "summary", item.summary)
      full_parts(s, slot, "reasoning", item.content)
    end
    local encoded = melchior.json.encode(item)
    if not encoded then s.failed = true; return end
    s.saved_bytes = s.saved_bytes - #(slot.saved or "") + #encoded
    if s.saved_bytes > MAX_STATE then s.failed = true; return end
    slot.saved, slot.done = encoded, true
  end

  local function drain(s, deltas, force)
    if s.failed then return end
    while s.next <= s.max do
      local slot = s.slots[tostring(s.next)]
      if not slot then
        if force then s.failed = true end
        return
      end
      local closed = slot.done or force
      if slot.kind == "function_call" then
        if not slot.started then
          deltas[#deltas + 1] = { kind = "tool_call_start", id = slot.call_id, name = slot.name }
          slot.started, s.has_calls = true, true
        end
        local p = slot.arguments
        if #p.value > p.sent then
          deltas[#deltas + 1] = { kind = "tool_call_args", arguments = p.value:sub(p.sent + 1) }
          p.sent = #p.value
        end
      else
        while slot.group <= #slot.order do
          local group = slot.groups[slot.order[slot.group]]
          while group.next <= group.max do
            local p = group.parts[tostring(group.next)]
            if not p then
              if closed then s.failed = true end
              return
            end
            if #p.value > p.sent then
              local value = p.value:sub(p.sent + 1)
              deltas[#deltas + 1] = slot.kind == "message" and { kind = "text", text = value }
                or { kind = "thinking", thinking = value }
              p.sent = #p.value
            end
            if not p.done and not closed then return end
            group.next = group.next + 1
          end
          if not closed then return end
          slot.group = slot.group + 1
        end
      end
      if not closed then return end
      s.next = s.next + 1
    end
  end

  local function locate(s, d)
    local at = d.output_index
    if at == nil and type(d.item_id) == "string" then
      at = tonumber(s.ids["id:" .. d.item_id])
    end
    if not index(at) then s.failed = true; return nil end
    local slot = s.slots[tostring(at)]
    if not slot or (d.item_id ~= nil and slot.id ~= d.item_id) then
      s.failed = true; return nil
    end
    return slot
  end

  local PART_EVENTS = {
    ["response.output_text.delta"] = { "message", "text", "content_index", false },
    ["response.output_text.done"] = { "message", "text", "content_index", true },
    ["response.refusal.delta"] = { "message", "text", "content_index", false },
    ["response.refusal.done"] = { "message", "text", "content_index", true, "refusal" },
    ["response.reasoning_summary_text.delta"] = { "reasoning", "summary", "summary_index", false },
    ["response.reasoning_summary_text.done"] = { "reasoning", "summary", "summary_index", true },
    ["response.reasoning_text.delta"] = { "reasoning", "reasoning", "content_index", false },
    ["response.reasoning_text.done"] = { "reasoning", "reasoning", "content_index", true },
  }

  local function track(s, d, kind, event, deltas)
    if s.failed then return end
    local indexed = d.output_index ~= nil or d.item_id ~= nil
    if kind == "response.output_item.added" or kind == "response.output_item.done" then
      if indexed then
        if kind == "response.output_item.done" then
          local exact = melchior.json.decode_value(event.data)
          close_item(s, d.output_index, exact and exact.item)
        else
          local slot = touch(s, d.output_index, d.item)
          if slot and slot.done then s.failed = true; return end
          if slot and slot.kind == "function_call" and d.item.arguments ~= nil then
            grow(s, slot.arguments, d.item.arguments, false)
          end
        end
      elseif s.indexed then s.failed = true
      elseif d.item and d.item.type == "function_call" and kind == "response.output_item.added" then
        if d.item.namespace ~= nil then s.failed = true; return end
        s.legacy, s.has_calls = true, true
        deltas[#deltas + 1] = { kind = "tool_call_start", id = d.item.call_id or d.item.id or "", name = d.item.name or "" }
      end
    elseif kind == "response.function_call_arguments.delta" or kind == "response.function_call_arguments.done" then
      local done = kind == "response.function_call_arguments.done"
      if indexed then
        local slot = locate(s, d)
        if slot and slot.kind == "function_call" then
          grow(s, slot.arguments, done and d.arguments or d.delta, done)
        else s.failed = true end
      elseif s.indexed then s.failed = true
      elseif not done then
        s.legacy = true
        deltas[#deltas + 1] = { kind = "tool_call_args", arguments = d.delta or "" }
      end
    elseif PART_EVENTS[kind] then
      local shape = PART_EVENTS[kind]
      if indexed then
        local slot = locate(s, d)
        if slot and slot.kind == shape[1] then
          local p = part(s, slot, shape[2], d[shape[3]] or 0)
          if p then grow(s, p, shape[4] and d[shape[5] or "text"] or d.delta, shape[4]) end
        else s.failed = true end
      elseif s.indexed then s.failed = true
      elseif not shape[4] then
        s.legacy = true
        deltas[#deltas + 1] = shape[1] == "message" and { kind = "text", text = d.delta or "" }
          or { kind = "thinking", thinking = d.delta or "" }
      end
    end
    drain(s, deltas, false)
  end

  local function finish(s, items, complete)
    if s.failed or s.legacy then return items end
    if items ~= nil and type(items) ~= "table" then s.failed = true; return nil end
    if items then
      local encoded = melchior.json.encode(items)
      if not encoded or encoded:sub(1, 1) ~= "[" then s.failed = true; return nil end
    end
    if items and #items > 0 then
      if #items > MAX_ITEMS or s.max >= #items then s.failed = true; return nil end
      for n, item in ipairs(items) do
        if complete and type(item) == "table" and item.status ~= nil
            and item.status ~= melchior.json.null and item.status ~= "completed" then
          s.failed = true; return nil
        end
        close_item(s, n - 1, item)
      end
      return items
    end
    if not s.indexed then return items end
    local out = melchior.json.array()
    for at = 0, s.max do
      local slot = s.slots[tostring(at)]
      if not slot or not slot.done or not slot.saved then
        if complete then s.failed = true end
        return nil
      end
      out[#out + 1] = melchior.json.decode_value(slot.saved)
    end
    return out
  end

  function M.on_event(state, event)
    local ok, d = pcall(function() return melchior.json.decode(event.data) end)
    if not ok or type(d) ~= "table" then return { scratch = state.scratch, usage = state.usage } end

    local deltas, usage = {}, state.usage
    local kind = d.type or event.name
    local anomalies = 0
    local scratch = state.scratch or {}
    if scratch.stopped then return { scratch = scratch, usage = usage } end
    scratch.responses = scratch.responses or { slots = {}, ids = {}, next = 0, max = -1,
      bytes = 0, saved_bytes = 0, parts = 0 }
    local stream = scratch.responses

    if kind == "response.completed" or kind == "response.incomplete"
        or kind == "response.failed" then
      local r = d.response or {}
      if type(r) ~= "table" then stream.failed = true; r = {} end
      local exact = melchior.json.decode_value(event.data)
      local response = type(exact) == "table" and exact.response
      if type(response) ~= "table" then stream.failed = true; response = {} end
      local items = finish(stream, response.output, kind == "response.completed")
      drain(stream, deltas, kind ~= "response.completed")
      local continuation_failed = stream.failed
      if kind == "response.completed" and scratch.continuation_origin and not stream.failed then
        if projection(items) then
          local encoded = melchior.json.encode({ version = 1,
            origin = scratch.continuation_origin, items = items })
          if not encoded or #PREFIX + #encoded > MAX_STATE then continuation_failed = true
          else
            deltas[#deltas + 1] = { kind = "signature", signature = PREFIX .. encoded }
          end
        end
      end
      local u = r.usage
      if u ~= nil then
        local measured
        local exact_usage = response.usage
        local has_ticks = type(exact_usage) == "table" and exact_usage.cost_in_usd_ticks ~= nil
        if has_ticks and type(u) == "table" then u.cost = nil end
        usage, anomalies, scratch.usage_total, measured = inclusive_usage(usage, u, "input_tokens", "output_tokens",
            "input_tokens_details", "cached_tokens", nil, scratch.usage_total, state.measured_usage)
        if has_ticks then
          local micros = tick_micros(exact_usage.cost_in_usd_ticks)
          if micros then usage.cost_micros, measured.cost_micros = micros, micros
          else anomalies = anomalies + 1 end
        end
        deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured }
      end

      local reason = "end_turn"
      if kind == "response.incomplete" then
        -- An incomplete response says why, and running out of room is the case that must poison
        -- the turn's tool calls rather than look like a clean finish.
        local why = r.incomplete_details and r.incomplete_details.reason
        reason = INCOMPLETE[why] or "error"
      elseif kind == "response.failed" then
        reason = "error"
      else
        -- A completed response that produced a call is waiting on it.
        if stream.has_calls then reason = "tool_use" end
        for _, item in ipairs(items or {}) do
          if item.type == "function_call" then reason = "tool_use" end
        end
      end
      if continuation_failed then reason = "error" end
      deltas[#deltas + 1] = { kind = "stop", reason = reason }
      scratch.stopped = true
    else track(stream, d, kind, event, deltas) end

    return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies }
  end

  -- ---------------------------------------------------------------- the three hostings

  local openai = {}
  for k, v in pairs(M) do openai[k] = v end
  function openai.endpoint(base_url, _model) return base_url .. "/responses" end
  function openai.headers(key)
    if not key then return {} end
    return { authorization = "Bearer " .. key }
  end
  melchior.api("openai-responses", openai)

  local azure = {}
  for k, v in pairs(M) do azure[k] = v end
  -- Azure routes by deployment and dates its API in the query string; the base URL carries the
  -- resource, so the model id is the deployment name.
  function azure.endpoint(base_url, model)
    return base_url .. "/openai/deployments/" .. model.id .. "/responses?api-version=2025-04-01-preview"
  end
  function azure.headers(key)
    if not key then return {} end
    return { ["api-key"] = key }
  end
  melchior.api("azure-openai-responses", azure)

  local codex = {}
  for k, v in pairs(M) do codex[k] = v end
  function codex.endpoint(base_url, _model) return base_url .. "/codex/responses" end
  function codex.headers(key)
    if not key then return {} end
    -- A subscription token, not an API key: the account is the credential, which is why this
    -- provider declares `oauth` rather than naming a variable to export.
    return { authorization = "Bearer " .. key, ["openai-beta"] = "responses=experimental" }
  end
  melchior.api("openai-codex-responses", codex)
end

do -- anthropic-messages
  local M = {}

  -- Anthropic dates its protocol rather than numbering it.
  local VERSION = "2023-06-01"

  -- Reasoning budgets in tokens, for each level a caller can ask for. `off` is absent rather than
  -- zero: the request omits thinking entirely, which is a different thing from asking for none.
  local BUDGET = {
    minimal = 1024, low = 4096, medium = 16384, high = 32768, max = 63999,
  }

  function M.endpoint(base_url, _model)
    return base_url .. "/v1/messages"
  end

  function M.headers(key)
    local h = { ["anthropic-version"] = VERSION }
    -- Omitted rather than sent empty: an empty credential reads as a malformed request, and the
    -- transport already refuses to call when none was resolved.
    if key then h["x-api-key"] = key end
    return h
  end

  local PREFIX, MAX_STATE, MAX_BLOCKS = ANTHROPIC_STATE, 1048576, 256

  local function projection(items)
    if type(items) ~= "table" or #items > MAX_BLOCKS then return nil end
    local encoded = melchior.json.encode(items)
    if not encoded or encoded:sub(1, 1) ~= "[" then return nil end
    local thinking, text, calls = {}, {}, melchior.json.array()
    for _, item in ipairs(items) do
      if type(item) ~= "table" then return nil end
      if item.type == "thinking" and type(item.thinking) == "string"
          and (item.signature == nil or type(item.signature) == "string") then
        thinking[#thinking + 1] = item.thinking
      elseif item.type == "text" and type(item.text) == "string" then
        text[#text + 1] = item.text
      elseif item.type == "tool_use" and type(item.id) == "string" and type(item.name) == "string"
          and type(item.input) == "table" and melchior.json.encode(item.input):sub(1, 1) == "{" then
        calls[#calls + 1] = { id = item.id, name = item.name, arguments = item.input }
      elseif item.type ~= "redacted_thinking" or type(item.data) ~= "string" then return nil end
    end
    return { thinking = table.concat(thinking), text = table.concat(text), calls = calls }
  end

  local function continuation(message, bound)
    if message.role ~= "assistant" or not bound then return nil end
    local signature, thinking, text, calls = nil, {}, {}, melchior.json.array()
    for _, item in ipairs(message.content or {}) do
      if item.type == "thinking" then
        if signature then return nil end
        signature = item.signature
        thinking[#thinking + 1] = item.thinking
      elseif item.type == "text" then text[#text + 1] = item.text
      elseif item.type == "tool_call" then
        calls[#calls + 1] = { id = item.id, name = item.name, arguments = item.arguments }
      else return nil end
    end
    if type(signature) ~= "string" or #signature > MAX_STATE or signature:sub(1, #PREFIX) ~= PREFIX then return nil end
    local saved = melchior.json.decode_value(signature:sub(#PREFIX + 1))
    if type(saved) ~= "table" or saved.version ~= 1 or not same_state(saved.origin, bound) then return nil end
    if not same_state(projection(saved.items), { thinking = table.concat(thinking), text = table.concat(text), calls = calls }) then return nil end
    return saved.items
  end

  -- Unsigned thinking is included only when the model permits it; opaque state stays local.
  local function block(c, compat)
    if c.type == "text" then
      if c.text == "" then return nil end
      return { type = "text", text = c.text }
    elseif c.type == "thinking" then
      if opaque_state(c.signature) or (not c.signature and not compat.allow_empty_signature) then return nil end
      return { type = "thinking", thinking = c.thinking, signature = c.signature or "" }
    elseif c.type == "image" then
      return { type = "image",
               source = { type = "base64", media_type = c.media_type, data = c.data } }
    elseif c.type == "tool_call" then
      return { type = "tool_use", id = c.id, name = c.name, input = c.arguments }
    elseif c.type == "tool_result" then
      local content = c.content
      if c.images and #c.images > 0 then
        content = { { type = "text", text = c.content } }
        for _, image in ipairs(c.images) do
          content[#content + 1] = { type = "image", source = {
            type = "base64", media_type = image.media_type, data = image.data,
          } }
        end
      end
      return { type = "tool_result", tool_use_id = c.id,
               content = content, is_error = c.is_error }
    end
  end

  -- Tool results are user-role blocks here rather than a role of their own, which is the largest
  -- shape difference from the Completions dialect. Consecutive same-role messages are merged
  -- because the API refuses two user turns in a row, and a result after a user message is that.
  local function messages(list, bound, compat)
    local out = melchior.json.array()
    for _, m in ipairs(list or {}) do
      local role = (m.role == "assistant") and "assistant" or "user"
      local blocks = continuation(m, bound)
      if not blocks then
        blocks = {}
        for _, c in ipairs(m.content or {}) do
          local b = block(c, compat)
          if b then blocks[#blocks + 1] = b end
        end
      end
      if #blocks > 0 then
        local last = out[#out]
        if last and last.role == role then
          for _, b in ipairs(blocks) do last.content[#last.content + 1] = b end
        else
          out[#out + 1] = { role = role, content = blocks }
        end
      end
    end
    return out
  end

  local function cacheable(message, content)
    if content.type == "text" then
      return type(content.text) == "string" and content.text ~= ""
    elseif content.type == "image" or content.type == "tool_result" then
      return message.role == "user"
    elseif content.type == "tool_use" then
      return message.role == "assistant"
    end
    return false
  end

  local function cache_conversation(list)
    local marked = 0
    for index = #list, 1, -1 do
      local message = list[index]
      for index = #message.content, 1, -1 do
        local content = message.content[index]
        if cacheable(message, content) then
          content.cache_control = { type = "ephemeral" }
          marked = marked + 1
          break
        end
      end
      if marked == 2 then break end
    end
  end

  function M.request(model, ctx, opts)
    local bound = model.compat.anthropic_replay and continuation_origin(model, opts) or nil
    local body = {
      model = model.id,
      stream = true,
      max_tokens = output(model, opts),
      messages = messages(ctx.messages, bound, model.compat),
    }
    if model.compat.anthropic_replay then
      body.__scratch = { anthropic_continuation = melchior.json.encode({
        origin = bound, items = melchior.json.array(), allow_empty = model.compat.allow_empty_signature,
      }) }
    end
    if ctx.system then body.system = ctx.system end

    local native_schema = model.compat.anthropic_schema == "output_config"
    if opts.schema and not native_schema then
      body.tools = {
        {
          name = opts.schema.name,
          description = "Answer by calling this with the requested value.",
          input_schema = plain(opts.schema.schema),
        },
      }
      body.tool_choice = { type = "tool", name = opts.schema.name }
    elseif ctx.tools and #ctx.tools > 0 then
      local tools = {}
      for _, t in ipairs(ctx.tools) do
        tools[#tools + 1] = { name = t.name, description = t.description, input_schema = t.parameters, strict = t.__strict }
      end
      body.tools = tools
    end
    if opts.schema and native_schema then
      body.output_config = { format = { type = "json_schema", schema = plain(opts.schema.schema) } }
    end
    if model.compat.supports_eager_tool_input_streaming then
      for _, tool in ipairs(body.tools or {}) do tool.eager_input_streaming = true end
    end

    -- A budget must leave room for a response: asking for the whole of max_tokens as reasoning
    -- yields a turn with nothing in it.
    local mode = model.compat.anthropic_thinking
    local budget = mode == "budget" and opts.thinking and BUDGET[opts.thinking]
    if mode ~= "budget" and opts.thinking then
      if opts.thinking == "off" then body.thinking = { type = "disabled" }
      else
        body.thinking = { type = "adaptive" }
        if mode == "adaptive_effort" then
          body.output_config = body.output_config or {}
          body.output_config.effort = opts.thinking
        end
      end
    elseif budget then
      local cap = math.max(body.max_tokens - 1024, 1024)
      body.thinking = { type = "enabled", budget_tokens = math.min(budget, cap) }
    end
    if model.compat.prompt_cache == "anthropic_ephemeral" then
      if body.system and body.system ~= "" then
        body.system = { { type = "text", text = body.system, cache_control = { type = "ephemeral" } } }
      end
      if body.tools and #body.tools > 0 and model.compat.supports_cache_control_on_tools then
        body.tools[#body.tools].cache_control = { type = "ephemeral" }
      end
      cache_conversation(body.messages)
    end
    return prepared(body, model, opts, { "max_tokens" })
  end

  local STOP = {
    tool_use = "tool_use", max_tokens = "length",
    end_turn = "end_turn", stop_sequence = "end_turn",
  }

  local function updated_usage(previous, raw)
    local usage, measured, anomalies = { cost_micros = previous.cost_micros }, {}, 0
    for field, key in pairs({ input = "input_tokens", output = "output_tokens",
        cache_read = "cache_read_input_tokens", cache_write = "cache_creation_input_tokens" }) do
      local value = token_count(raw[key])
      if raw[key] ~= nil and value == nil then anomalies = anomalies + 1 end
      usage[field] = value or previous[field]
      measured[field] = value
    end
    return usage, measured, anomalies
  end

  local function complete_stream(state, event)
    local scratch, usage, deltas, anomalies = state.scratch, state.usage, {}, 0
    local stream = melchior.json.decode_value(scratch.anthropic_continuation)
    if type(stream) ~= "table" then stream = { failed = true, items = melchior.json.array() } end
    if stream.finished then return { scratch = scratch, usage = usage } end
    local d = melchior.json.decode_value(event.data)
    if type(d) ~= "table" then stream.failed = true; d = {} end
    local name = event.name
    if name == "message_start" then
      if stream.started then stream.failed = true end
      stream.started = true
      local message = d.message
      if type(message) ~= "table" then stream.failed = true; message = {} end
      if message.role and message.role ~= "assistant" then stream.failed = true end
      if message.content and (type(message.content) ~= "table" or #message.content > 0) then stream.failed = true end
      local u = message.usage or {}
      if type(u) ~= "table" then stream.failed = true; u = {} end
      usage = { input = u.input_tokens or 0, output = u.output_tokens or 0,
        cache_read = u.cache_read_input_tokens or 0, cache_write = u.cache_creation_input_tokens or 0 }
      deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured_usage(u, {
        input = "input_tokens", output = "output_tokens", cache_read = "cache_read_input_tokens", cache_write = "cache_creation_input_tokens",
      }) }
    elseif not stream.failed and name == "content_block_start" then
      local b = d.content_block
      if not stream.started or stream.active or type(b) ~= "table" or type(d.index) ~= "number" or d.index ~= #stream.items
          or d.index >= MAX_BLOCKS or not projection(melchior.json.array({ b })) then stream.failed = true
      else
        stream.items[#stream.items + 1], stream.active, stream.args = b, d.index + 1, ""
        if b.type == "text" and b.text ~= "" then deltas[#deltas + 1] = { kind = "text", text = b.text }
        elseif b.type == "thinking" and b.thinking ~= "" then deltas[#deltas + 1] = { kind = "thinking", thinking = b.thinking }
        elseif b.type == "tool_use" then deltas[#deltas + 1] = { kind = "tool_call_start", id = b.id, name = b.name } end
      end
    elseif not stream.failed and name == "content_block_delta" then
      local b = stream.active and stream.items[stream.active]
      local delta = d.delta
      if type(delta) ~= "table" then stream.failed = true; delta = {} end
      if not b or d.index ~= stream.active - 1 then stream.failed = true
      elseif delta.type == "text_delta" and b.type == "text" and type(delta.text) == "string" then
        b.text = b.text .. delta.text
        deltas[#deltas + 1] = { kind = "text", text = delta.text }
      elseif delta.type == "thinking_delta" and b.type == "thinking" and type(delta.thinking) == "string" then
        b.thinking = b.thinking .. delta.thinking
        deltas[#deltas + 1] = { kind = "thinking", thinking = delta.thinking }
      elseif delta.type == "signature_delta" and b.type == "thinking" and type(delta.signature) == "string"
          and (b.signature == nil or type(b.signature) == "string") then
        b.signature = (b.signature or "") .. delta.signature
      elseif delta.type == "input_json_delta" and b.type == "tool_use" and type(delta.partial_json) == "string"
          and melchior.json.encode(b.input) == "{}" then stream.args = stream.args .. delta.partial_json
      else stream.failed = true end
    elseif not stream.failed and name == "content_block_stop" then
      local b = stream.active and stream.items[stream.active]
      if not b or d.index ~= stream.active - 1 then stream.failed = true
      else
        if b.type == "tool_use" then
          if stream.args ~= "" then b.input = melchior.json.decode_value(stream.args) end
          if not projection(melchior.json.array({ b })) then stream.failed = true
          else deltas[#deltas + 1] = { kind = "tool_call_args", arguments = melchior.json.encode(b.input) } end
        end
        if b.type == "thinking" and ((b.signature ~= nil and type(b.signature) ~= "string")
            or (not stream.allow_empty and (b.signature == nil or b.signature == ""))) then stream.failed = true end
        stream.active, stream.args = nil, nil
      end
    end
    if name == "message_delta" and d.usage ~= nil and type(d.usage) ~= "table" then stream.failed = true; d.usage = nil end
    if name == "message_delta" and d.delta ~= nil and type(d.delta) ~= "table" then stream.failed = true; d.delta = nil end
    if name == "message_delta" and d.usage then
      local measured
      usage, measured, anomalies = updated_usage(usage, d.usage)
      deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured }
    end
    local encoded = melchior.json.encode(stream)
    if not encoded or #encoded > MAX_STATE then stream.failed = true end
    local reason = name == "message_delta" and d.delta and d.delta.stop_reason
    if reason or name == "message_stop" or name == "error" then
      if type(reason) ~= "string" or not STOP[reason] then stream.failed = true end
      if not stream.started or stream.active or not projection(stream.items) then stream.failed = true end
      if not stream.failed and STOP[reason] ~= "length" and stream.origin and #stream.items > 0 then
        local saved = melchior.json.encode({ version = 1, origin = stream.origin, items = stream.items })
        if not saved or #PREFIX + #saved > MAX_STATE then stream.failed = true
        else deltas[#deltas + 1] = { kind = "signature", signature = PREFIX .. saved } end
      end
      deltas[#deltas + 1] = { kind = "stop", reason = stream.failed and "error" or STOP[reason] or "end_turn" }
      stream.finished = true
    end
    if stream.failed or stream.finished then stream.items, stream.active, stream.args = melchior.json.array(), nil, nil end
    scratch.anthropic_continuation = melchior.json.encode(stream)
    return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies }
  end

  function M.on_event(state, event)
    if type(state.scratch) == "table" and state.scratch.anthropic_continuation then
      return complete_stream(state, event)
    end
    local ok, d = pcall(function() return melchior.json.decode(event.data) end)
    if not ok or type(d) ~= "table" then return { scratch = state.scratch, usage = state.usage } end

    local deltas, anomalies = {}, 0
    local usage = state.usage

    if event.name == "message_start" then
      local u = d.message and d.message.usage or {}
      usage = {
        input = u.input_tokens or 0,
        output = u.output_tokens or 0,
        cache_read = u.cache_read_input_tokens or 0,
        cache_write = u.cache_creation_input_tokens or 0,
      }
      deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured_usage(u, {
        input = "input_tokens", output = "output_tokens", cache_read = "cache_read_input_tokens",
        cache_write = "cache_creation_input_tokens",
      }) }

    elseif event.name == "content_block_start" then
      local b = d.content_block or {}
      if b.type == "tool_use" then
        deltas[#deltas + 1] = { kind = "tool_call_start", id = b.id or "", name = b.name or "" }
      end

    elseif event.name == "content_block_delta" then
      local delta = d.delta or {}
      if delta.type == "text_delta" then
        deltas[#deltas + 1] = { kind = "text", text = delta.text or "" }
      elseif delta.type == "thinking_delta" then
        deltas[#deltas + 1] = { kind = "thinking", thinking = delta.thinking or "" }
      elseif delta.type == "signature_delta" then
        -- Arrives at the end of a thinking block and must be replayed verbatim for the model to
        -- accept its own reasoning back.
        deltas[#deltas + 1] = { kind = "signature", signature = delta.signature or "" }
      elseif delta.type == "input_json_delta" then
        deltas[#deltas + 1] = { kind = "tool_call_args", arguments = delta.partial_json or "" }
      end

    elseif event.name == "message_delta" then
      if type(d.usage) == "table" then
        local measured
        usage, measured, anomalies = updated_usage(usage, d.usage)
        deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured }
      end
      local reason = d.delta and d.delta.stop_reason
      if reason then
        deltas[#deltas + 1] = { kind = "stop", reason = STOP[reason] or "end_turn" }
      end
    end

    return { scratch = state.scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies }
  end

  melchior.api("anthropic-messages", M)
end

do -- google
  local M = {}
  local PREFIX, MAX_STATE, MAX_PARTS = GOOGLE_STATE, 1048576, 8192

  local function absent(value)
    return value == nil or value == melchior.json.null
  end

  local function shaped(value, opening)
    if type(value) ~= "table" then return false end
    local encoded = melchior.json.encode(value)
    return encoded and encoded:sub(1, 1) == opening
  end

  local function signature(value)
    return type(value) == "string" and #value > 0 and #value % 4 == 0
      and value:match("^[%w+/]+=?=?$") ~= nil
  end

  local function projection(parts)
    if not shaped(parts, "[") or #parts > MAX_PARTS then return nil end
    local text, thinking, calls = {}, {}, melchior.json.array()
    for _, p in ipairs(parts) do
      if not shaped(p, "{") then return nil end
      for key in pairs(p) do
        if key ~= "text" and key ~= "thought" and key ~= "thoughtSignature" and key ~= "functionCall" then return nil end
      end
      if not absent(p.thought) and type(p.thought) ~= "boolean" then return nil end
      if not absent(p.thoughtSignature) and not signature(p.thoughtSignature) then return nil end
      if not absent(p.functionCall) then
        local call = p.functionCall
        if not shaped(call, "{") or not absent(p.text) then return nil end
        for key in pairs(call) do
          if key ~= "id" and key ~= "name" and key ~= "args" then return nil end
        end
        if type(call.id) ~= "string" or call.id == "" or #call.id > 4096
            or type(call.name) ~= "string" or call.name == "" or #call.name > 4096 then return nil end
        calls[#calls + 1] = { id = call.id, name = call.name,
          arguments = call.args == nil and melchior.json.object() or call.args }
      elseif type(p.text) == "string" then
        local into = p.thought == true and thinking or text
        into[#into + 1] = p.text
      elseif absent(p.text) and signature(p.thoughtSignature) then
      else return nil end
    end
    return { text = table.concat(text), thinking = table.concat(thinking), calls = calls }
  end

  local function continuation(message, bound, include_id)
    if message.role ~= "assistant" or not bound then return nil end
    local saved_signature, text, thinking, calls = nil, {}, {}, melchior.json.array()
    for _, c in ipairs(message.content or {}) do
      if c.type == "thinking" then
        if saved_signature then return nil end
        saved_signature = c.signature
        thinking[#thinking + 1] = c.thinking
      elseif c.type == "text" then text[#text + 1] = c.text
      elseif c.type == "tool_call" then
        calls[#calls + 1] = { id = c.id, name = c.name, arguments = c.arguments }
      else return nil end
    end
    if type(saved_signature) ~= "string" or #saved_signature > MAX_STATE
        or saved_signature:sub(1, #PREFIX) ~= PREFIX then return nil end
    local saved = melchior.json.decode_value(saved_signature:sub(#PREFIX + 1))
    if type(saved) ~= "table" or saved.version ~= 1 or not same_state(saved.origin, bound) then return nil end
    if not same_state(projection(saved.parts), { text = table.concat(text), thinking = table.concat(thinking), calls = calls }) then return nil end
    if not include_id then
      for _, p in ipairs(saved.parts) do if p.functionCall then p.functionCall.id = nil end end
    end
    return saved.parts
  end

  local function requires_call_id(model)
    local id = model.id:lower()
    local major = tonumber(id:match("^gemini%-live%-(%d+)") or id:match("^gemini%-(%d+)"))
    return model.id:sub(1, 7) == "claude-" or model.id:sub(1, 8) == "gpt-oss-"
      or (major ~= nil and major >= 3)
  end

  local function next_call_id(scratch, supplied)
    scratch.google_call_ids = scratch.google_call_ids or {}
    local used = scratch.google_call_ids
    local id = supplied
    if type(id) ~= "string" or id == "" or used[id] then
      repeat
        scratch.google_call_count = (scratch.google_call_count or 0) + 1
        id = "melchior-google-call-" .. scratch.google_call_count
      until not used[id]
    end
    used[id] = true
    return id
  end

  -- Google names the model's turn "model" rather than "assistant", and carries tool calls and
  -- results as parts inside a turn rather than as messages of their own.
  local function part(c, include_id)
    if c.type == "text" then
      if c.text == "" then return nil end
      return { text = c.text }
    elseif c.type == "thinking" then
      if c.thinking == "" then return nil end
      return { text = c.thinking }
    elseif c.type == "image" then
      return { inlineData = { mimeType = c.media_type, data = c.data } }
    elseif c.type == "tool_call" then
      local p = { functionCall = { name = c.name, args = c.arguments } }
      if include_id then p.functionCall.id = c.id end
      return p
    elseif c.type == "tool_result" then
      local response = { name = c.name, response = { output = c.content } }
      if include_id then response.id = c.id end
      return { functionResponse = response }
    end
  end

  local function contents(messages, include_id, bound)
    local out = melchior.json.array()
    for _, m in ipairs(messages or {}) do
      local role = (m.role == "assistant") and "model" or "user"
      local parts = continuation(m, bound, include_id)
      if not parts then
        parts = {}
        for _, c in ipairs(m.content or {}) do
          local p = part(c, include_id)
          if p then parts[#parts + 1] = p end
          if c.type == "tool_result" then
            for _, image in ipairs(c.images or {}) do
              parts[#parts + 1] = { inlineData = { mimeType = image.media_type, data = image.data } }
            end
          end
        end
      end
      if #parts > 0 then
        local last = out[#out]
        -- Turns must alternate, so consecutive same-role content is merged rather than sent as
        -- two turns the API will refuse.
        if last and last.role == role then
          for _, p in ipairs(parts) do last.parts[#last.parts + 1] = p end
        else
          out[#out + 1] = { role = role, parts = parts }
        end
      end
    end
    return out
  end

  function M.request(model, ctx, opts)
    local used = {}
    for _, message in ipairs(ctx.messages or {}) do
      for _, content in ipairs(message.content or {}) do
        if (content.type == "tool_call" or content.type == "tool_result") and content.id ~= "" then
          used[content.id] = true
        end
      end
    end
    local bound = continuation_origin(model, opts)
    local body = {
      contents = contents(ctx.messages, requires_call_id(model), bound),
      __scratch = { google_call_ids = used,
        google_continuation = melchior.json.encode({ origin = bound, parts = melchior.json.array() }) },
      generationConfig = {
        maxOutputTokens = output(model, opts),
      },
    }
    -- Google puts it on the generation config, and wants the mime type set with it.
    if opts.schema then
      body.generationConfig = body.generationConfig or {}
      body.generationConfig.responseMimeType = "application/json"
      body.generationConfig.responseSchema = plain(opts.schema.schema)
    end

    if ctx.system then
      body.systemInstruction = { parts = { { text = ctx.system } } }
    end

    if ctx.tools and #ctx.tools > 0 then
      local declarations = {}
      for _, t in ipairs(ctx.tools) do
        declarations[#declarations + 1] = {
          name = t.name, description = t.description, parameters = t.parameters,
        }
      end
      body.tools = { { functionDeclarations = declarations } }
    end

    local control = opts.__google_control
    if control then
      local config = {}
      if control.kind == "level" then config.thinkingLevel = control.value
      else config.thinkingBudget = control.value end
      if control.summaries then config.includeThoughts = true end
      body.generationConfig.thinkingConfig = config
    end
    return prepared(body, model, opts, { "generationConfig", "maxOutputTokens" })
  end

  local function google_usage(previous, raw, scratch, evidence)
    if not shaped(raw, "{") then return previous, 1, {} end
    local counts, invalid = scratch.google_usage or {}, 0
    for key, field in pairs({ candidates = "candidatesTokenCount", thoughts = "thoughtsTokenCount" }) do
      local value = raw[field]
      if value ~= nil then
        local n = token_count(value)
        if n and n <= 2147483647 then
          if counts[key] and n < counts[key] then invalid = invalid + 1 end
          counts[key] = math.max(n, counts[key] or 0)
          if key == "thoughts" then counts.unknown_thoughts = nil end
        else
          invalid = invalid + 1
          if key == "thoughts" and counts.thoughts == nil then counts.unknown_thoughts = true end
        end
      end
    end
    scratch.google_usage = counts
    local normalized = { promptTokenCount = raw.promptTokenCount,
      cachedContentTokenCount = raw.cachedContentTokenCount, cost = raw.cost }
    if counts.candidates ~= nil or counts.thoughts ~= nil then
      normalized.candidatesTokenCount = (counts.candidates or 0) + (counts.thoughts or 0)
    end
    local usage, anomalies, total, measured = inclusive_usage(previous, normalized,
      "promptTokenCount", "candidatesTokenCount", nil, "cachedContentTokenCount", nil,
      scratch.usage_total, evidence)
    scratch.usage_total = total
    if counts.candidates == nil or counts.unknown_thoughts then measured.output = nil end
    return usage, invalid + anomalies, measured
  end

  local FINISH = { STOP = "end_turn", MAX_TOKENS = "length" }

  function M.on_event(state, event)
    local deltas, usage = {}, state.usage
    local scratch = state.scratch or {}
    local stream = scratch.google_continuation and melchior.json.decode_value(scratch.google_continuation)
      or { parts = melchior.json.array() }
    if type(stream) ~= "table" then stream = { failed = true, parts = melchior.json.array() } end
    local d = melchior.json.decode_value(event.data)
    if not shaped(d, "{") then stream.failed = true; d = {} end

    local u = d.usageMetadata
    local anomalies = 0
    if not absent(u) then
      local measured
      usage, anomalies, measured = google_usage(usage, u, scratch, state.measured_usage)
      deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured }
    end
    if stream.finished then return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies } end

    if not absent(d.error) then stream.failed = true end
    if shaped(d.promptFeedback, "{") and not absent(d.promptFeedback.blockReason)
        and d.promptFeedback.blockReason ~= "BLOCK_REASON_UNSPECIFIED" then stream.failed = true end
    local candidate
    if not absent(d.candidates) then
      if not shaped(d.candidates, "[") then stream.failed = true
      else candidate = d.candidates[1] end
    end
    if candidate and not shaped(candidate, "{") then stream.failed = true; candidate = nil end
    local parts
    if candidate and not absent(candidate.content) then
      if not shaped(candidate.content, "{") or (not absent(candidate.content.role) and candidate.content.role ~= "model") then stream.failed = true
      else parts = candidate.content.parts end
    end
    if not absent(parts) and not shaped(parts, "[") then stream.failed = true end
    if not stream.failed and not absent(parts) then
      if #stream.parts + #parts > MAX_PARTS then stream.failed = true
      else
        for _, p in ipairs(parts) do
          if shaped(p, "{") and shaped(p.functionCall, "{") then
            p.functionCall.id = next_call_id(scratch, p.functionCall.id)
          end
          if not projection(melchior.json.array({ p })) then stream.failed = true; break end
          stream.parts[#stream.parts + 1] = p
          if not absent(p.functionCall) then
            stream.saw_tool_call = true
            deltas[#deltas + 1] = { kind = "tool_call_start", id = p.functionCall.id, name = p.functionCall.name }
            deltas[#deltas + 1] = { kind = "tool_call_args", arguments = melchior.json.encode(
              p.functionCall.args == nil and melchior.json.object() or p.functionCall.args) }
          elseif type(p.text) == "string" and p.text ~= "" then
            if p.thought == true then deltas[#deltas + 1] = { kind = "thinking", thinking = p.text }
            else deltas[#deltas + 1] = { kind = "text", text = p.text } end
          end
        end
      end
    end

    local encoded = melchior.json.encode(stream)
    if not encoded or #encoded > MAX_STATE then stream.failed = true end
    local finish = candidate and candidate.finishReason
    if stream.failed or not absent(finish) then
      local reason = not stream.failed and FINISH[finish] or "error"
      if reason == "end_turn" and stream.saw_tool_call then reason = "tool_use" end
      if reason ~= "length" and reason ~= "error" and stream.origin and #stream.parts > 0 then
        local saved = melchior.json.encode({ version = 1, origin = stream.origin, parts = stream.parts })
        if not saved or #PREFIX + #saved > MAX_STATE then reason = "error"
        else deltas[#deltas + 1] = { kind = "signature", signature = PREFIX .. saved } end
      end
      deltas[#deltas + 1] = { kind = "stop", reason = reason }
      stream.finished = true
      stream.parts, scratch.google_call_ids, scratch.google_call_count = melchior.json.array(), nil, nil
    end
    scratch.google_continuation = melchior.json.encode(stream)
    return { scratch = scratch, usage = usage, deltas = deltas, usage_anomalies = anomalies }
  end

  -- ---------------------------------------------------------------- the two hostings

  local gemini = {}
  for k, v in pairs(M) do gemini[k] = v end
  -- `alt=sse` is what turns this into a stream; without it the endpoint answers one JSON array
  -- at the end, which looks like a very slow model.
  function gemini.endpoint(base_url, model)
    return base_url .. "/models/" .. model.id .. ":streamGenerateContent?alt=sse"
  end
  function gemini.headers(key)
    if not key then return {} end
    return { ["x-goog-api-key"] = key }
  end
  melchior.api("google-generative-ai", gemini)

  local vertex = {}
  for k, v in pairs(M) do vertex[k] = v end
  -- Vertex scopes by project and region, which the base URL carries; the model is a publisher
  -- path rather than a bare id.
  function vertex.endpoint(base_url, model)
    return base_url .. "/publishers/google/models/" .. model.id .. ":streamGenerateContent?alt=sse"
  end
  function vertex.headers(key)
    if not key then return {} end
    -- A short-lived access token from the credential chain, not an API key.
    return { authorization = "Bearer " .. key }
  end
  melchior.api("google-vertex", vertex)
end

do -- pi-messages
  local M = {}

  function M.endpoint(base_url, _model)
    return base_url .. "/messages"
  end

  function M.headers(key)
    if not key then return {} end
    return { authorization = "Bearer " .. key }
  end

  local function block(c)
    if c.type == "text" then
      if c.text == "" then return nil end
      return { type = "text", text = c.text }
    elseif c.type == "thinking" then
      if opaque_state(c.signature) then return nil end
      local b = { type = "thinking", thinking = c.thinking }
      if c.signature then b.signature = c.signature end
      return b
    elseif c.type == "image" then
      return { type = "image", data = c.data, mimeType = c.media_type }
    elseif c.type == "tool_call" then
      return { type = "toolCall", id = c.id, name = c.name, arguments = c.arguments }
    elseif c.type == "tool_result" then
      local content = { { type = "text", text = c.content } }
      for _, image in ipairs(c.images or {}) do
        content[#content + 1] = { type = "image", data = image.data, mimeType = image.media_type }
      end
      return { type = "toolResult", toolCallId = c.id, content = content, isError = c.is_error }
    end
  end

  function M.request(model, ctx, opts)
    local messages = melchior.json.array()
    for _, m in ipairs(ctx.messages or {}) do
      local blocks = {}
      for _, c in ipairs(m.content or {}) do
        local b = block(c)
        if b then blocks[#blocks + 1] = b end
      end
      if #blocks > 0 then
        messages[#messages + 1] = { role = m.role, content = blocks }
      end
    end

    local body = {
      model = model.id,
      stream = true,
      messages = messages,
      maxTokens = output(model, opts),
    }
    if ctx.system then body.system = ctx.system end
    if ctx.tools and #ctx.tools > 0 then body.tools = ctx.tools end
    if opts.thinking and opts.thinking ~= "off" then body.thinking = opts.thinking end
    return prepared(body, model, opts, { "maxTokens" })
  end

  local STOP = {
    toolUse = "tool_use", length = "length",
    stop = "end_turn", aborted = "aborted", error = "error",
  }

  function M.on_event(state, event)
    local ok, d = pcall(function() return melchior.json.decode(event.data) end)
    if not ok or type(d) ~= "table" then return { scratch = state.scratch, usage = state.usage } end

    local deltas, usage = {}, state.usage
    local kind = d.type or event.name

    if kind == "text_delta" then
      deltas[#deltas + 1] = { kind = "text", text = d.delta or "" }
    elseif kind == "thinking_delta" then
      deltas[#deltas + 1] = { kind = "thinking", thinking = d.delta or "" }
    elseif kind == "thinking_end" then
      -- The signature arrives when the block closes, not with its text.
      if d.signature then
        deltas[#deltas + 1] = { kind = "signature", signature = d.signature }
      end
    elseif kind == "toolcall_start" then
      deltas[#deltas + 1] = { kind = "tool_call_start", id = d.id or "", name = d.toolName or "" }
    elseif kind == "toolcall_delta" then
      deltas[#deltas + 1] = { kind = "tool_call_args", arguments = d.delta or "" }
    elseif kind == "done" then
      local u = d.usage
      if u then
        usage = {
          input = u.input or 0, output = u.output or 0,
          cache_read = u.cacheRead or 0, cache_write = u.cacheWrite or 0,
        }
        deltas[#deltas + 1] = { kind = "usage", usage = usage, measured = measured_usage(u, {
          input = "input", output = "output", cache_read = "cacheRead", cache_write = "cacheWrite",
        }) }
      end
      deltas[#deltas + 1] = { kind = "stop", reason = STOP[d.stopReason] or "end_turn" }
    elseif kind == "error" then
      deltas[#deltas + 1] = { kind = "stop", reason = "error" }
    end

    return { scratch = state.scratch, usage = usage, deltas = deltas }
  end

  melchior.api("pi-messages", M)
end

do -- decisions
  -- Models that decide rather than write: asked typed questions about a state, they answer each
  -- with a value and how sure they are, in one JSON document and a third of a second. TypeSafe's
  -- Jev is the first; OpenRouter serves it at `/api/alpha/decisions`, TypeSafe at `/v1/systemone`,
  -- and both take the same body.
  --
  -- A caller asks as it asks any model: a system prompt, messages, and a JSON Schema for the
  -- answer. The schema is what becomes the questions, one per property:
  --
  --   boolean                    a yes/no (`noul`), true at `x-threshold` or above (0.5)
  --   string with `enum`         a `choice` among the values
  --   number with `x-criteria`   a `score` on that rubric, lowest first
  --   string without `enum`      not asked -- such a model writes nothing. Filled from
  --                              `x-from = "<choice property>"` with what that choice means
  --
  -- `description` is the question put, and the system prompt stands in where there is none;
  -- `x-criteria` says what each answer means (a table by value, or a list for a score). What
  -- comes back is one JSON object of the schema's shape, with `_decided` beside it carrying the
  -- probabilities, so a caller that only wants the answer reads it as it reads any other.
  local M = {}

  function M.endpoint(base_url, _model)
    if base_url:match("/decisions$") or base_url:match("/systemone$") then return base_url end
    return base_url .. "/decisions"
  end

  function M.headers(key)
    if not key then return {} end
    return { authorization = "Bearer " .. key }
  end

  -- Everything said, as the state to be judged. One message goes as it is; more are labelled.
  local function state_of(ctx)
    local said = {}
    for _, m in ipairs(ctx.messages or {}) do
      local text = {}
      for _, c in ipairs(m.content or {}) do
        if c.type == "text" and c.text ~= "" then text[#text + 1] = c.text end
        if c.type == "tool_result" then text[#text + 1] = tostring(c.content or "") end
      end
      if #text > 0 then said[#said + 1] = { role = m.role, text = table.concat(text, "\n") } end
    end
    if #said == 1 then return said[1].text end
    local lines = {}
    for _, s in ipairs(said) do lines[#lines + 1] = s.role .. ": " .. s.text end
    return table.concat(lines, "\n\n")
  end

  local function question(property, asked)
    local put = property.description or asked
    local means = property["x-criteria"]
    if property.type == "boolean" then
      return { type = "noul", instructions = put,
               criteria = means or { ["true"] = "Yes", ["false"] = "No" } }
    end
    if property.enum then
      local criteria = {}
      for _, value in ipairs(property.enum) do
        criteria[tostring(value)] = (means and means[tostring(value)]) or tostring(value)
      end
      return { type = "choice", instructions = put, criteria = criteria }
    end
    if (property.type == "number" or property.type == "integer") and type(means) == "table" then
      return { type = "score", instructions = put, criteria = means }
    end
    return nil
  end

  function M.request(model, ctx, opts)
    local asked = ctx.system or "Answer about the state."
    local questions, shape = {}, { order = {}, fields = {} }
    local properties = opts.schema and opts.schema.schema and opts.schema.schema.properties
    if properties then
      for name, property in pairs(properties) do
        local q = question(property, asked)
        if q then questions[name] = q end
        shape.order[#shape.order + 1] = name
        shape.fields[name] = {
          threshold = property["x-threshold"] or 0.5,
          from = property["x-from"],
          means = property["x-criteria"],
          written = q == nil,
        }
      end
    else
      -- Asked with no shape for the answer, there is one thing such a model can say: yes or no.
      questions.answer = { type = "noul", instructions = asked,
                           criteria = { ["true"] = "Yes", ["false"] = "No" } }
      shape.plain = true
    end
    return {
      model = model.id,
      state = state_of(ctx),
      questions = questions,
      -- Kept for reading the reply by, and taken out of the body before it is sent.
      __scratch = shape,
    }
  end

  local function decided(answer)
    if answer.type == "noul" then return { p = answer.noul } end
    return { p = answer.probabilities, confidence = answer.confidence }
  end

  function M.on_event(state, event)
    -- One document and not a stream: it arrives whole, under this name.
    if event.name ~= "body" then return { scratch = state.scratch, usage = state.usage } end
    local ok, d = pcall(function() return melchior.json.decode(event.data) end)
    local shape = state.scratch or {}
    if not ok or type(d) ~= "table" or type(d.answers) ~= "table" then
      return { scratch = shape, usage = state.usage,
               deltas = { { kind = "stop", reason = "error" } } }
    end

    local deltas = {}
    local u = d.usage or {}
    local usage = {
      input = u.input_tokens or 0, output = u.output_tokens or 0,
      cache_read = 0, cache_write = 0,
      cost_micros = type(u.cost) == "number" and math.floor(u.cost * 1000000 + 0.5) or 0,
    }
    deltas[#deltas + 1] = { kind = "usage", usage = usage,
      measured = measured_usage(u, { input = "input_tokens", output = "output_tokens" }) }

    local text
    if shape.plain then
      local p = d.answers.answer and d.answers.answer.noul or 0
      text = string.format("%s (%.2f)", p >= 0.5 and "yes" or "no", p)
    else
      local out, how = {}, {}
      for name, field in pairs(shape.fields or {}) do
        local answer = d.answers[name]
        if answer then
          how[name] = decided(answer)
          if answer.type == "noul" then
            out[name] = (answer.noul or 0) >= field.threshold
          elseif answer.type == "choice" then
            out[name] = answer.choice
          else
            out[name] = answer.score
          end
        end
      end
      -- What such a model cannot write is said for it: what the choice it made means.
      for name, field in pairs(shape.fields or {}) do
        if field.written then
          local chosen = field.from and out[field.from]
          local means = field.from and shape.fields[field.from] and shape.fields[field.from].means
          out[name] = (chosen and means and means[tostring(chosen)]) or ""
        end
      end
      out._decided = how
      text = melchior.json.encode(out)
    end
    deltas[#deltas + 1] = { kind = "text", text = text }
    deltas[#deltas + 1] = { kind = "stop", reason = "end_turn" }
    return { scratch = shape, usage = usage, deltas = deltas }
  end

  melchior.api("decisions", M)
end
