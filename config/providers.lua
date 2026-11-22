-- Provider declarations, bundled model selections and discovery policies.

melchior.provider("anthropic", {
  name = "Anthropic",
  api = "anthropic-messages",
  base_url = "https://api.anthropic.com",
  auth = { kind = "api-key", vars = { "ANTHROPIC_API_KEY" } },
  compat = { prompt_cache = "anthropic_ephemeral" },
  models = {
    { id = "claude-opus-4-6", name = "Claude Opus 4.6", context_window = 200000, max_tokens = 64000, reasoning = true, input = { "text", "image" }, cost = { input = 5.0, output = 25.0, cache_read = 0.5, cache_write = 6.25 } },
    { id = "claude-sonnet-4-5", name = "Claude Sonnet 4.5", context_window = 200000, max_tokens = 64000, reasoning = true, input = { "text", "image" }, cost = { input = 3.0, output = 15.0, cache_read = 0.3, cache_write = 3.75 } },
    { id = "claude-haiku-4-5", name = "Claude Haiku 4.5", context_window = 200000, max_tokens = 64000, reasoning = true, input = { "text", "image" }, cost = { input = 1.0, output = 5.0, cache_read = 0.1, cache_write = 1.25 } },
  },
})

melchior.provider("github-copilot", {
  name = "GitHub Copilot",
  api = "anthropic-messages",
  base_url = "https://api.individual.githubcopilot.com",
  auth = { kind = "github-copilot" },
  catalog = { source = "pi-1.6.0", provider = "github-copilot", models = {
    "chat:claude-haiku-4.5", "chat:claude-sonnet-4.6",
    "chat:gemini-3.8-flash", "chat:gemini-4-argon", "chat:gemini-4-carbon",
    "chat:gemini-4-hydrogen", "chat:gemini-4-neon", "chat:kimi-k3",
    "chat:grok-4.5", "chat:grok-4.6", "chat:grok-4.7",
    "chat:mai-code-1.1-flash", "chat:mai-code-2", "chat:mai-code-2-flash",
  } },
  models = {
    { id = "claude-sonnet-4.5", name = "Claude Sonnet 4.5", context_window = 200000, max_tokens = 64000, reasoning = true },
  },
})

melchior.provider("openai", {
  name = "OpenAI",
  api = "openai-responses",
  base_url = "https://api.openai.com/v1",
  auth = { kind = "api-key", vars = { "OPENAI_API_KEY" } },
  compat = { max_tokens_field = "max_completion_tokens", supports_developer_role = true, supports_reasoning_effort = true, supports_store = true },
  models = {
    { id = "gpt-5.1", name = "GPT-5.1", context_window = 400000, max_tokens = 128000, reasoning = true, input = { "text", "image" }, cost = { input = 1.25, output = 10.0, cache_read = 0.125 } },
    { id = "gpt-5.1-mini", name = "GPT-5.1 mini", context_window = 400000, max_tokens = 128000, reasoning = true, input = { "text", "image" }, cost = { input = 0.25, output = 2.0 } },
    { id = "gpt-5", name = "GPT-5", context_window = 400000, max_tokens = 128000, reasoning = true, input = { "text", "image" }, cost = { input = 1.25, output = 10.0 } },
  },
})

melchior.provider("google", {
  name = "Google",
  api = "google-generative-ai",
  base_url = "https://generativelanguage.googleapis.com/v1beta",
  auth = { kind = "api-key", vars = { "GEMINI_API_KEY", "GOOGLE_API_KEY" } },
  models = {
    { id = "gemini-3-pro", name = "Gemini 3 Pro", context_window = 1048576, max_tokens = 65536, reasoning = true, input = { "text", "image" }, cost = { input = 1.25, output = 10.0 } },
    { id = "gemini-3-flash", name = "Gemini 3 Flash", context_window = 1048576, max_tokens = 65536, reasoning = true, input = { "text", "image" }, cost = { input = 0.3, output = 2.5 } },
  },
})

melchior.provider("openrouter", {
  name = "OpenRouter",
  api = "openai-completions",
  base_url = "https://openrouter.ai/api/v1",
  auth = { kind = "api-key", vars = { "OPENROUTER_API_KEY" } },
  compat = { supports_reasoning_effort = true, thinking_format = "openrouter" },
  -- Four hundred and counting. Six were listed here and they were a generation behind, which
  -- reads from the inside as OpenRouter being broken rather than the list being old.
  discover = true,
  discovery = { kind = "openrouter" },
  -- Upstreams never to be served by: OpenInference answers long DeepSeek V4 sessions as if the
  -- conversation were gone.
  avoid = { "OpenInference" },
})

melchior.provider("deepseek", {
  name = "DeepSeek",
  api = "openai-completions",
  base_url = "https://api.deepseek.com",
  auth = { kind = "api-key", vars = { "DEEPSEEK_API_KEY" } },
  compat = { supports_reasoning_effort = true, supports_json_schema = false,
    chat_strict_dialect = "deepseek" },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "deepseek", models = {
    "chat:deepseek-flash", "chat:deepseek-v4-pro",
  } },
})

melchior.provider("mistral", {
  name = "Mistral",
  api = "mistral-conversations",
  base_url = "https://api.mistral.ai",
  auth = { kind = "api-key", vars = { "MISTRAL_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "mistral", models = {
    "chat:codestral-2508", "chat:codestral-latest", "chat:deepseek-v4-1-flash-1017",
    "chat:devstral-2512", "chat:devstral-latest", "chat:devstral-medium-latest",
    "chat:glm-5-2", "chat:labs-devstral-small-2512", "chat:labs-leanstral-1-5-1",
    "chat:magistral-medium-latest", "chat:ministral-14b-2512", "chat:ministral-3b-2512",
    "chat:ministral-3b-latest", "chat:ministral-8b-2512", "chat:ministral-8b-latest",
    "chat:mistral-large-2512", "chat:mistral-large-4", "chat:mistral-large-4-0-8",
    "chat:mistral-large-latest", "chat:mistral-medium-2505", "chat:mistral-medium-2508",
    "chat:mistral-medium-2604", "chat:mistral-medium-3.5", "chat:mistral-medium-latest",
    "chat:mistral-small-2506", "chat:mistral-small-2603", "chat:mistral-small-latest",
    "chat:open-mistral-nemo", "chat:pixtral-12b", "chat:voxtral-small-2507",
    "chat:voxtral-small-latest", "chat:zai-glm-5-2", "chat:zai-glm-5-3", "chat:zai-glm-5-3-flash",
  } },
})

melchior.provider("groq", {
  name = "Groq",
  api = "openai-completions",
  base_url = "https://api.groq.com/openai/v1",
  auth = { kind = "api-key", vars = { "GROQ_API_KEY" } },
  compat = { supports_store = true, supports_reasoning_effort = true,
    max_tokens_field = "max_completion_tokens" },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "groq", models = {
    "chat:openai/gpt-oss-120b", "chat:openai/gpt-oss-20b",
  } },
})

melchior.provider("cerebras", {
  name = "Cerebras",
  api = "openai-completions",
  base_url = "https://api.cerebras.ai/v1",
  auth = { kind = "api-key", vars = { "CEREBRAS_API_KEY" } },
  compat = { supports_reasoning_effort = true, max_tokens_field = "max_completion_tokens" },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "cerebras", models = { "chat:gpt-oss-120b" } },
})

melchior.provider("baseten", {
  name = "Baseten",
  api = "openai-completions",
  base_url = "https://inference.baseten.co/v1",
  auth = { kind = "api-key", vars = { "BASETEN_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "baseten", models = {
    "chat:deepseek-ai/DeepSeek-V4-Flash-0731", "chat:deepseek-ai/DeepSeek-V4-Pro-0813",
    "chat:deepseek-ai/DeepSeek-V4.1-Flash", "chat:deepseek-ai/DeepSeek-V4.1-Flash-1017",
    "chat:deepseek-ai/DeepSeek-V4.1-Flash-1017-Fast", "chat:deepseek-ai/DeepSeek-V4.1-Flash-Fast",
    "chat:moonshotai/Kimi-K3", "chat:moonshotai/Kimi-K3.1-Pro",
    "chat:nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B", "chat:openai/gpt-oss-120b",
    "chat:zai-org/GLM-5.2", "chat:zai-org/GLM-5.2-Fast",
    "chat:zai-org/GLM-5.3", "chat:zai-org/GLM-5.3-Fast", "chat:zai-org/GLM-5.3-Flash",
  } },
  models = {
    { id = "moonshotai/Kimi-K3.1-Pro",
      capabilities = { input = { "text", "image" }, output = { "text" }, thinking_levels = melchior.json.array() },
    },
  },
})

melchior.provider("fireworks", {
  name = "Fireworks",
  api = "anthropic-messages",
  base_url = "https://api.fireworks.ai/inference",
  auth = { kind = "api-key", vars = { "FIREWORKS_API_KEY" } },
  discovery = { kind = "bundled" },
  compat = { supports_reasoning_effort = true, anthropic_replay = true,
    anthropic_schema = "output_config", prompt_cache = "anthropic_ephemeral",
    supports_forced_tool_choice = false, supports_strict_tools = false },
  catalog = { source = "pi-1.6.0", provider = "fireworks", models = {
    "chat:accounts/fireworks/models/deepseek-v4p1-flash",
    "chat:accounts/fireworks/models/deepseek-v4p1-flash-1017",
    "chat:accounts/fireworks/models/ember-1", "chat:accounts/fireworks/models/minimax-m3",
    "chat:accounts/fireworks/models/minimax-m3p1-flash", "chat:accounts/fireworks/models/muse-spark-open",
    "chat:accounts/fireworks/models/qwen3p8-2p4t-a95b", "chat:accounts/fireworks/models/qwen3p8-max",
    "chat:accounts/fireworks/routers/deepseek-flash-latest",
    "chat:accounts/fireworks/routers/deepseek-v4p1-flash-1017-spot",
    "chat:accounts/fireworks/routers/deepseek-v4p1-flash-1017-ultrafast",
    "chat:accounts/fireworks/routers/deepseek-v4p1-flash-spot",
    "chat:accounts/fireworks/routers/kimi-fast-latest", "chat:accounts/fireworks/routers/kimi-latest",
    "chat:accounts/fireworks/routers/minimax-latest", "chat:accounts/fireworks/routers/qwen-max-latest",
    "chat:accounts/fireworks/models/glm-5p3", "chat:accounts/fireworks/models/glm-5p3-flash",
    "chat:accounts/fireworks/models/kimi-k3", "chat:accounts/fireworks/models/kimi-k3p1-pro",
    "chat:accounts/fireworks/routers/glm-5p3-fast", "chat:accounts/fireworks/routers/glm-5p3-spot",
    "chat:accounts/fireworks/routers/glm-fast-latest", "chat:accounts/fireworks/routers/glm-flash-latest",
    "chat:accounts/fireworks/routers/glm-latest", "chat:accounts/fireworks/routers/kimi-k3-fast",
    "chat:accounts/fireworks/routers/kimi-k3-spot",
  } },
})

melchior.provider("nvidia", {
  name = "NVIDIA",
  api = "openai-completions",
  base_url = "https://integrate.api.nvidia.com/v1",
  auth = { kind = "api-key", vars = { "NVIDIA_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "nvidia", models = { "chat:stepfun-ai/step-5-preview" } },
  models = {
    { id = "stepfun-ai/step-5-preview",
      capabilities = { input = { "text", "image" }, output = { "text" }, thinking_levels = melchior.json.array() },
      input_limits = { images = { max_per_request = 8,
        resize = { max_width = 2000, max_height = 2000, max_bytes = 4718592, jpeg_quality = 80 } } },
    },
  },
})

melchior.provider("xai", {
  name = "xAI",
  api = "openai-responses",
  base_url = "https://api.x.ai/v1",
  auth = { kind = "api-key", vars = { "XAI_API_KEY" } },
  compat = { supports_strict_mode = false },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "xai", models = {
    "chat:grok-4.3", "chat:grok-4.5", "chat:grok-4.6", "chat:grok-4.7",
  } },
  models = {
    { id = "grok-4.5", thinking = {
      off = melchior.json.null, minimal = melchior.json.null,
      low = "low", medium = "medium", high = "high", xhigh = "high", max = melchior.json.null,
    } },
  },
})

melchior.provider("meta", {
  name = "Meta",
  api = "openai-responses",
  base_url = "https://api.meta.ai/v1",
  auth = { kind = "api-key", vars = { "META_API_KEY" } },
  compat = { supports_strict_mode = false },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "meta", models = {
    "chat:muse-ember-1.0", "chat:muse-spark-1.1", "chat:muse-spark-1.2",
    "chat:muse-spark-1.2-contributor", "chat:muse-spark-1.3",
    "chat:muse-spark-1.3-contributor", "chat:muse-wonder-1.0",
  } },
  models = {
    { id = "muse-spark-1.2-contributor", name = "training-enabled: Muse Spark 1.2 Contributor" },
    { id = "muse-spark-1.3-contributor", name = "training-enabled: Muse Spark 1.3 Contributor" },
  },
})

for _, region in ipairs({
  { id = "minimax", name = "MiniMax", url = "https://api.minimax.io/anthropic", key = "MINIMAX_API_KEY" },
  { id = "minimax-cn", name = "MiniMax China", url = "https://api.minimax.cn/anthropic", key = "MINIMAX_CN_API_KEY" },
}) do
  melchior.provider(region.id, {
    name = region.name,
    api = "anthropic-messages",
    base_url = region.url,
    auth = { kind = "api-key", vars = { region.key } },
    compat = { anthropic_replay = true, allow_empty_signature = true,
      supports_forced_tool_choice = false, supports_strict_tools = false },
    discovery = { kind = "bundled" },
    catalog = { source = "pi-1.6.0", provider = region.id, models = {
      "chat:MiniMax-M3", "chat:MiniMax-M3.1-Flash",
    } },
    models = {
      { id = "MiniMax-M3", base_url = region.url, compat = { anthropic_thinking = "adaptive" },
        thinking = { off = "off", minimal = "low", low = melchior.json.null, medium = melchior.json.null,
          high = melchior.json.null, xhigh = melchior.json.null, max = melchior.json.null },
      },
      { id = "MiniMax-M3.1-Flash", base_url = region.url },
    },
  })
end

for _, region in ipairs({
  { id = "xiaomi", name = "Xiaomi", host = "api", key = "XIAOMI_API_KEY" },
  { id = "xiaomi-token-plan-cn", name = "Xiaomi Token Plan China", host = "token-plan-cn", key = "XIAOMI_TOKEN_PLAN_CN_API_KEY" },
  { id = "xiaomi-token-plan-ams", name = "Xiaomi Token Plan Amsterdam", host = "token-plan-ams", key = "XIAOMI_TOKEN_PLAN_AMS_API_KEY" },
  { id = "xiaomi-token-plan-sgp", name = "Xiaomi Token Plan Singapore", host = "token-plan-sgp", key = "XIAOMI_TOKEN_PLAN_SGP_API_KEY" },
}) do
  local models = { "chat:mimo-v2.6-flash", "chat:mimo-v2.6-pro" }
  if region.id == "xiaomi" then models[#models + 1] = "chat:mimo-v2.6-pro-ultraspeed" end
  melchior.provider(region.id, {
    name = region.name,
    api = "openai-completions",
    base_url = "https://" .. region.host .. ".xiaomimimo.com/v1",
    auth = { kind = "api-key", vars = { region.key } },
    compat = { max_tokens_field = "max_completion_tokens", supports_json_schema = false },
    discovery = { kind = "bundled" },
    catalog = { source = "pi-1.6.0", provider = region.id, models = models },
  })
end

for _, region in ipairs({
  { id = "zai", name = "Z.AI", host = "api.z.ai", key = "ZAI_API_KEY",
    models = { "glm-4.7", "glm-5-turbo", "glm-5.2", "glm-5.2-highspeed",
      "glm-5.3", "glm-5.3-flash", "glm-5.3-highspeed" } },
  { id = "zai-coding-cn", name = "Z.AI Coding China", host = "open.bigmodel.cn", key = "ZAI_CODING_CN_API_KEY",
    models = { "glm-4.6v", "glm-5.3", "glm-5.3-flash", "glm-5.3-highspeed" } },
}) do
  local selection, overrides = {}, {}
  for _, id in ipairs(region.models) do
    selection[#selection + 1] = "chat:" .. id
    local binary = id == "glm-4.7" or id == "glm-5-turbo" or id == "glm-4.6v"
    local model = { id = id, compat = { supports_store = false, supports_reasoning_effort = not binary,
      max_tokens_field = "max_tokens", thinking_format = "zai", supports_strict_mode = false, zai_tool_stream = true } }
    if binary then
      model.capabilities = { input = id == "glm-4.6v" and { "text", "image" } or { "text" },
        output = { "text" }, thinking_levels = { "off", "high" } }
    end
    overrides[#overrides + 1] = model
  end
  melchior.provider(region.id, {
    name = region.name,
    api = "openai-completions",
    base_url = "https://" .. region.host .. "/api/coding/paas/v4",
    auth = { kind = "api-key", vars = { region.key } },
    compat = { supports_json_schema = false, requires_tool_result_name = region.id == "zai" },
    discovery = { kind = "bundled" },
    catalog = { source = "pi-1.6.0", provider = region.id, models = selection },
    models = overrides,
  })
end

for _, region in ipairs({
  { id = "moonshotai", name = "Moonshot", url = "https://api.moonshot.ai/v1", key = "MOONSHOT_API_KEY" },
  { id = "moonshotai-cn", name = "Moonshot China", url = "https://api.moonshot.cn/v1", key = "MOONSHOT_API_KEY" },
  { id = "kimi-coding", name = "Kimi Coding", url = "https://api.kimi.com/coding/v1", key = "KIMI_API_KEY" },
}) do
  local ids = region.id == "kimi-coding"
    and { "k3", "k3-256k", "k3.1-pro", "k3.1-pro-highspeed", "kimi-for-coding", "kimi-for-coding-highspeed" }
    or { "kimi-k2.6", "kimi-k2.7-code", "kimi-k2.7-code-highspeed", "kimi-k3", "kimi-k3.1-pro", "kimi-k3.1-pro-highspeed" }
  local selection, overrides = {}, {}
  for _, id in ipairs(ids) do
    selection[#selection + 1] = "chat:" .. id
    local model = { id = id }
    local optional = id == "kimi-k2.6"
    if optional or id == "kimi-k2.7-code" or id == "kimi-k2.7-code-highspeed" or id == "kimi-for-coding-highspeed" then
      model.capabilities = { input = { "text", "image" }, output = { "text" },
        thinking_levels = optional and { "off", "high" } or { "high" } }
    end
    if optional then
      model.compat = { supports_store = false, supports_reasoning_effort = false,
        max_tokens_field = "max_tokens", supports_strict_mode = false, thinking_format = "deepseek",
        supports_mid_convo_system_messages = true, moonshot_keep_thinking = true }
    end
    overrides[#overrides + 1] = model
  end
  melchior.provider(region.id, {
    name = region.name,
    api = "openai-completions",
    base_url = region.url,
    auth = { kind = "api-key", vars = { region.key } },
    headers = { ["User-Agent"] = "melchior" },
    compat = { supports_prompt_cache_key = true },
    discovery = { kind = "bundled" },
    catalog = { source = "pi-1.6.0", provider = region.id, models = selection },
    models = overrides,
  })
end

-- Where ollama is, as its own client reads `OLLAMA_HOST`: a bare `host:port`, or a whole URL
-- when it carries a scheme. Unset is the daemon on this machine.
local function ollama_at()
  local said = os.getenv("OLLAMA_HOST")
  if not said or said == "" then return "http://localhost:11434/v1" end
  local url = said:match("^https?://") and said or ("http://" .. said)
  return (url:gsub("/+$", "")) .. "/v1"
end

for _, plan in ipairs({
  { id = "qwen-token-plan", name = "Qwen Token Plan Team", key = "QWEN_TOKEN_PLAN_TEAM_API_KEY" },
  { id = "qwen-token-plan-cn", name = "Qwen Token Plan Team CN", key = "QWEN_TOKEN_PLAN_TEAM_CN_API_KEY", china = true },
  { id = "qwen-token-plan-individual", name = "Qwen Token Plan Individual", key = "QWEN_TOKEN_PLAN_INDIVIDUAL_API_KEY", individual = true },
  { id = "qwen-token-plan-individual-cn", name = "Qwen Token Plan Individual CN", key = "QWEN_TOKEN_PLAN_INDIVIDUAL_CN_API_KEY", individual = true, china = true },
}) do
  local ids = {
    "deepseek-v4-flash-0731", "deepseek-v4-pro", "deepseek-v4-pro-0813",
    "deepseek-v4.1-flash", "deepseek-v4.1-flash-1017", "glm-5.2", "glm-5.3",
    "qwen3.6-flash", "qwen3.7-max", "qwen3.7-plus", "qwen3.8-flash", "qwen3.8-max",
    "qwen4-max-preview",
  }
  if not plan.individual then
    for _, id in ipairs({ "MiniMax-M2.5", "glm-5", "glm-5.1", "kimi-k2.5", "kimi-k2.6", "kimi-k2.7-code", "qwen3.6-plus" }) do
      ids[#ids + 1] = id
    end
  end
  local selection = {}
  for _, id in ipairs(ids) do selection[#selection + 1] = "chat:" .. id end
  melchior.provider(plan.id, {
    name = plan.name,
    api = "openai-completions",
    base_url = "https://token-plan.maas." .. (plan.china and "qianwenaiapi.com" or "qwencloudapi.com") .. "/compatible-mode/v1",
    auth = { kind = "api-key", vars = { plan.key, plan.china and "QWEN_TOKEN_PLAN_CN_API_KEY" or "QWEN_TOKEN_PLAN_API_KEY" } },
    compat = { max_tokens_field = "max_completion_tokens" },
    discovery = { kind = "bundled" },
    catalog = { source = "pi-1.6.0", provider = plan.id, models = selection },
  })
end

melchior.provider("together", {
  name = "Together",
  api = "openai-completions",
  base_url = "https://api.together.ai/v1",
  auth = { kind = "api-key", vars = { "TOGETHER_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "together", models = {
    "chat:deepseek-ai/DeepSeek-V4-Pro-0813", "chat:deepseek-ai/DeepSeek-V4.1-Flash",
    "chat:meta-models/Muse-Spark-Open-816B-A42B", "chat:MiniMaxAI/MiniMax-M3",
    "chat:moonshotai/Kimi-K3", "chat:moonshotai/Kimi-K3.1-Pro",
    "chat:nvidia/NVIDIA-Nemotron-3.5-Super-VL", "chat:openai/gpt-oss-120b",
    "chat:Qwen/Qwen3.5-9B", "chat:Qwen/Qwen3.6-Plus", "chat:Qwen/Qwen3.7-Max",
    "chat:reflection/Beam-1.0-501B-A23B", "chat:thinkingmachines/Inkling",
    "chat:zai-org/GLM-5.2", "chat:zai-org/GLM-5.3", "chat:zai-org/GLM-5.3-Flash",
  } },
})

melchior.provider("ant-ling", {
  name = "Ant Ling",
  api = "openai-completions",
  base_url = "https://api.ant-ling.com/v1",
  auth = { kind = "api-key", vars = { "ANT_LING_API_KEY" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "ant-ling", models = {
    "chat:Ling-2.6-1T", "chat:Ling-2.6-flash", "chat:Ring-2.6-1T",
  } },
})

melchior.provider("huggingface", {
  name = "Hugging Face",
  api = "openai-completions",
  base_url = "https://router.huggingface.co/v1",
  auth = { kind = "api-key", vars = { "HF_TOKEN" } },
  discovery = { kind = "bundled" },
  catalog = { source = "pi-1.6.0", provider = "huggingface", models = {
    "chat:MiniMaxAI/MiniMax-M1-80k", "chat:MiniMaxAI/MiniMax-M2", "chat:MiniMaxAI/MiniMax-M2.1",
    "chat:MiniMaxAI/MiniMax-M2.5", "chat:MiniMaxAI/MiniMax-M2.7", "chat:MiniMaxAI/MiniMax-M3", "chat:MiniMaxAI/MiniMax-M3.1-Flash",
    "chat:Qwen/Qwen2.5-Coder-32B-Instruct", "chat:Qwen/Qwen3-235B-A22B",
    "chat:Qwen/Qwen3-235B-A22B-Instruct-2507", "chat:Qwen/Qwen3-235B-A22B-Thinking-2507",
    "chat:Qwen/Qwen3-30B-A3B", "chat:Qwen/Qwen3-30B-A3B-Instruct-2507", "chat:Qwen/Qwen3-32B", "chat:Qwen/Qwen3-8B",
    "chat:Qwen/Qwen3-Coder-30B-A3B-Instruct", "chat:Qwen/Qwen3-Coder-480B-A35B-Instruct", "chat:Qwen/Qwen3-Coder-Next",
    "chat:Qwen/Qwen3-Next-80B-A3B-Instruct", "chat:Qwen/Qwen3-VL-235B-A22B-Instruct",
    "chat:Qwen/Qwen3-VL-235B-A22B-Thinking", "chat:Qwen/Qwen3-VL-30B-A3B-Instruct",
    "chat:Qwen/Qwen3.5-122B-A10B", "chat:Qwen/Qwen3.5-27B", "chat:Qwen/Qwen3.5-35B-A3B",
    "chat:Qwen/Qwen3.5-397B-A17B", "chat:Qwen/Qwen3.5-9B", "chat:Qwen/Qwen3.6-27B", "chat:Qwen/Qwen3.6-35B-A3B",
    "chat:Qwen/Qwen3.8-2.4T-A95B", "chat:Qwen/Qwen3.8-27B", "chat:XiaomiMiMo/MiMo-V2.5", "chat:XiaomiMiMo/MiMo-V2.5-Pro",
    "chat:deepseek-ai/DeepSeek-R1", "chat:deepseek-ai/DeepSeek-R1-0528", "chat:deepseek-ai/DeepSeek-V3",
    "chat:deepseek-ai/DeepSeek-V3-0324", "chat:deepseek-ai/DeepSeek-V3.1", "chat:deepseek-ai/DeepSeek-V3.2",
    "chat:deepseek-ai/DeepSeek-V4-Flash", "chat:deepseek-ai/DeepSeek-V4-Flash-0731", "chat:deepseek-ai/DeepSeek-V4-Flash-Vision-Exp",
    "chat:deepseek-ai/DeepSeek-V4-Pro", "chat:deepseek-ai/DeepSeek-V4-Pro-0813",
    "chat:deepseek-ai/DeepSeek-V4.1-Flash", "chat:deepseek-ai/DeepSeek-V4.1-Flash-1017",
    "chat:google/gemma-3-12b-it", "chat:google/gemma-3-27b-it", "chat:google/gemma-3-4b-it",
    "chat:google/gemma-4-26B-A4B-it", "chat:google/gemma-4-31B-it",
    "chat:meta-llama/Llama-3.1-70B-Instruct", "chat:meta-llama/Llama-3.1-8B-Instruct",
    "chat:meta-llama/Llama-3.2-1B-Instruct", "chat:meta-llama/Llama-3.2-3B-Instruct", "chat:meta-llama/Llama-3.3-70B-Instruct",
    "chat:moonshotai/Kimi-K2-Instruct", "chat:moonshotai/Kimi-K2-Instruct-0905", "chat:moonshotai/Kimi-K2-Thinking",
    "chat:moonshotai/Kimi-K2.5", "chat:moonshotai/Kimi-K2.6", "chat:moonshotai/Kimi-K2.7-Code",
    "chat:moonshotai/Kimi-K3", "chat:moonshotai/Kimi-K3.1-Pro", "chat:openai/gpt-oss-120b", "chat:openai/gpt-oss-20b",
    "chat:stepfun-ai/Step-3.5-Flash", "chat:stepfun-ai/Step-3.7-Flash", "chat:tencent/Hy3", "chat:tencent/Hy4-flash", "chat:tencent/Hy4-preview",
    "chat:thinkingmachines/Inkling", "chat:thinkingmachines/Inkling-2-Small", "chat:thinkingmachines/Inkling-Small",
    "chat:zai-org/GLM-4.5", "chat:zai-org/GLM-4.5-Air", "chat:zai-org/GLM-4.5V", "chat:zai-org/GLM-4.6",
    "chat:zai-org/GLM-4.6V", "chat:zai-org/GLM-4.6V-Flash", "chat:zai-org/GLM-4.7", "chat:zai-org/GLM-4.7-Flash",
    "chat:zai-org/GLM-5", "chat:zai-org/GLM-5.1", "chat:zai-org/GLM-5.2", "chat:zai-org/GLM-5.3", "chat:zai-org/GLM-5.3-Flash",
  } },
})

melchior.provider("ollama", {
  name = "Ollama",
  api = "openai-completions",
  base_url = ollama_at(),
  auth = { kind = "none" },
  -- A thinking level goes as `reasoning_effort`, which ollama reads; off goes as its "none".
  compat = { supports_reasoning_effort = true },
  -- Whatever you have pulled, which is the only list that could be right. Its listing names each
  -- model and nothing more, so `/api/show` is asked for the window and whether it thinks.
  discover = true,
  details = "ollama",
  discovery = { kind = "ollama" },
})


-- Models that decide rather than write, through OpenRouter's decisions endpoint and on the same
-- key as everything else it serves. They are not in its model listing, so they are named here.
-- Asked with a JSON Schema for the answer, which the `decisions` protocol turns into typed
-- questions: see `apis.lua`. What suits them is a judgement a knowledgeable person makes in a
-- few seconds -- is this command safe, which of these does this belong to -- in a third of a
-- second and for next to nothing; what does not is anything that has to be written or worked out.
melchior.provider("decisions", {
  name = "OpenRouter decisions",
  api = "decisions",
  base_url = "https://openrouter.ai/api/alpha",
  auth = { kind = "api-key", vars = { "OPENROUTER_API_KEY" } },
  models = {
    { id = "typesafe/jev-1.13", name = "TypeSafe Jev 1.13", context_window = 32000, max_tokens = 1024,
      cost = { input = 0.042, output = 0.0 } },
  },
})
