# Reserved Conventions

Decisions made ahead of the code that will use them. **These are reservations, not implementations.** Don't add code for any of them until their track starts. Until then, don't write code that would rule them out.

## Model string routing

A model string's prefix selects the provider: `deepseek:`, `ollama:`, `claude:`, `gpt:`, `nandi:`.

- `nandi:` is reserved for remote inference servers on other Nandi instances.
- The LLM proxy dispatches on the prefix. It never matches against a hardcoded list of providers.

## Inference servers are agents

A remote inference server is an agent like any other:

- It declares itself in `skill.md` with `capabilities.serves: [inference]`.
- Its endpoint uses `endpoint.type: a2a`.
- It is discovered and delegated to through the same A2A path as any other agent.

Don't special-case inference servers anywhere in the codebase.

## Containers may be long-running services

Container lifecycle must not assume one-shot task execution. A container running `command: sleep infinity` that exposes an HTTP or MCP endpoint is a valid, supported shape. The future model server (Ollama/DMR track) has exactly this shape.

## Local model container (future track; don't build now)

When it's implemented:

- The local model server is a Nandi-managed container.
- It exposes an OpenAI-compatible endpoint on a host port.
- The LLM proxy routes `ollama:` and `nandi:` model strings to it.
- GPUs are detected at first launch. CPU fallback is allowed.
- On macOS, Nandi connects to host-native Ollama (Metal) instead of running a container.
