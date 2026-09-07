-- A provider for something melchior does not ship.
--
-- Copy this into `~/.config/melchior/plugin/` and `melchior models` lists it in the next run --
-- no edit to `providers.lua`, no fork of the eight that ship, no rebuild.
--
-- **This is what P3 and P4 were for together.** A file on disk used to *replace* the shipped
-- catalog, so adding one endpoint meant copying all of it and owning every later fix to any of
-- them forever. Now the shipped ones run first and this layers over the top, in a file of its
-- own that nothing else has to name.
--
-- What a provider owes:
--
--   name        what a person sees.
--   api         which wire protocol it speaks -- a *protocol*, not a vendor. `melchior verbs`
--               will not tell you these; `apis.lua` declares them, and llama.cpp, ollama, vLLM,
--               LM Studio and most local servers all speak `openai-completions`.
--   base_url    where it lives. Omitted when the endpoint comes from configuration.
--   auth        how to prove who you are. `kind = "none"` for something on your own machine.
--   models      what it offers, or `discover = true` to ask it.
--
-- Registering the same id twice replaces, so a file in `after/plugin/` can override any of this.

melchior.provider("local-llama", {
  name = "llama.cpp (local)",
  api = "openai-completions",
  base_url = "http://127.0.0.1:8080/v1",

  -- Nothing to prove on a socket bound to localhost. A provider that needs a key names the
  -- environment variables to look in instead: `{ kind = "api-key", vars = { "MY_KEY" } }`.
  auth = { kind = "none" },

  -- **Asked rather than listed.** A local server knows what it has loaded and this changes every
  -- time you swap a GGUF; a list here would be out of date by the afternoon. What comes back is
  -- cached and refreshed daily, and never written back into any file.
  discover = true,
})

melchior.provider("local-ollama", {
  name = "Ollama",
  api = "openai-completions",
  base_url = "http://127.0.0.1:11434/v1",
  auth = { kind = "none" },

  -- The other half of the choice: named, with what melchior needs in order to budget a turn.
  -- `context_window` is the one that matters -- it is what decides when a session compacts, and
  -- a wrong number here is a conversation that truncates early or a request that is refused.
  models = {
    {
      id = "qwen2.5-coder:32b",
      name = "Qwen2.5 Coder 32B",
      context_window = 32768,
      max_tokens = 8192,
      -- No `cost`, because there is none. A model with no cost is reported as free rather than
      -- as unknown, which is the truth for something running on your own machine.
    },
  },
})
