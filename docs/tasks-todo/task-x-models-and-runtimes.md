# Task: One "Models & runtimes" page, a real trial switch, and a Build image button

**From:** owner feedback 2026-10-20: "no clear way to disable the DeepSeek default key and use a specific model", "the list of models should be where we choose each runtime", "no way to build the runtime Docker image, it just has info", "runtimes and models are confusing".
**Size:** L. Three waves; each ships alone.

## Today

- **Settings › Models** (`ModelsSection.tsx` → `LlmKeysSection.tsx`): the free trial line with one Turn off / Use free tokens button (`FreeTrialField`), a free-text default model with a `<datalist>` (`DefaultModelField`), then provider key rows and Ollama.
- **Settings › Runtimes** (`RuntimesPanel` in `RuntimesSection.tsx`): a Nandi card that only says "Uses your Models settings", a `HarnessCard` for Claude Code, Codex and Gemini (sign-in, Test, Make default, "Use an API key instead" jumps back to Models), the Bedrock model picker, the LiteLLM gateway, sessions. `setHarnessDefault(id, model)` is always sent `null`.
- **Trial** (`llm_trial.rs`, `llm_keys.rs` `resolve_model`): Auto turns itself off once the user's own model works. Turning it off with no key leaves no model and no guidance. The button disappears when the tokens run out. "Use free tokens" overwrites the default model. The fallback order (preferred → first keyed provider → trial) is shown only as "using X instead".
- **Images**: built by hand (`docker build -t nandi-harness:latest runtimes/nandi-harness`, `… nandi-runtime …`). Rust only checks `image_present` and prints the command. The Dockerfiles aren't bundled, so an installed app can't build them.

## What the images are for (say this in the UI)

- `nandi-harness`: the sandbox Claude Code, Codex and Gemini CLI run in. Needed only by agents with `runtime: claude-code | codex | gemini-cli`.
- `nandi-runtime`: the tool sandbox for Nandi's own agents (`shell`, files). Needed only when an agent uses those tools.
- Ask and chat-only agents need neither.

## Wave A: model picker and trial switch (no Docker)

- [x] Merge the rail items into one section, "Models & runtimes". **Decided:** it keeps the id `models` (dozens of links use it); `runtimes` links redirect to it with `focus: 'runtimes'`.
- [x] **Engines** block, one row per engine with a "Default for new agents" radio:
  - Nandi (built in): a `Select` model picker grouped by provider (from `KEYED_PROVIDERS` and the Ollama models; "Custom…" opens the free-text field). Models whose provider has no key are marked "needs key" and open that key row inline.
  - Free tokens: a real `Switch` with tokens left, always shown (disabled with "used up" when 0).
  - Show the chosen model and the model really used, with the reason ("No Anthropic key, so DeepSeek Flash").
  - Trial off and no key: "Pick a model and add its key", not a blank.
- [x] Kept `set_free_trial` as it is (on remembers the model, off gives it back); the page now says plainly what's used and why, and "no model" says to pick one and add its key.
- [x] Claude Code / Codex / Gemini cards and the Bedrock models sit under Coding agents on the same page.
- [ ] A model picker per CLI runtime saved with `setHarnessDefault(id, model)`.
- [x] **Provider keys** block: the existing `ProviderRow` / `OllamaRow` list, shared by both kinds of engine.
- [x] **Advanced**: LiteLLM gateway, sessions, `EnterpriseConfigSection`.
- [x] i18n in en, fr, ar, ja (flat keys `ai.*`).
- [x] Tests: `SettingsView.test.tsx`, `harness-ui.test.tsx`, the aliases, the trial switch on/off and the model it leaves.

## Wave B: build the images from the app

- [x] `services/images.rs`: embed both build contexts (`include_bytes!`), tar in memory, build with bollard `build_image`. Label the image with the app version so a Rebuild hint shows after an update.
- [x] Commands (and HTTP routes): `get_image_status(kind)` → `{present, dockerRunning, builtVersion}`, `build_image(kind)` → build ID (returns at once, runs in `tokio::spawn`), no cancel yet.
- [x] One event, `images://build` (`{image, line, finished, error}`); the log is a ring of about 500 lines kept in Rust, so leaving the page doesn't lose it.
- [x] Engine rows: image status, **Build / Rebuild** button, progress and a collapsible log. Docker missing or stopped is detected with a ping first and says what to do.
- [x] Pass `HTTP(S)_PROXY` as build args (corporate networks).
- [x] Replace `harness.card.noSandbox` (raw command) with the button.

## Wave C (later): pull prebuilt images

- [ ] Publish both images to GHCR from CI and pull them; Build stays as the offline fallback.

## Acceptance

- [ ] A new user with no key can see the trial is on, turn it off, pick `gpt:…`, add the key in the same place, and see "Using GPT-5".
- [ ] Claude Code's model is picked on the Claude Code row, not in a separate block.
- [ ] Clicking Build on a machine with Docker produces `nandi-harness:latest`, and the Claude Code card turns Ready without a restart.
