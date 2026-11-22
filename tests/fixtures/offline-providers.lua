for _, id in ipairs({ "openrouter", "ollama" }) do
  melchior.provider(id, {
    name = id,
    api = "openai-completions",
    auth = { kind = "none" },
    discovery = { kind = "bundled" },
  })
end
