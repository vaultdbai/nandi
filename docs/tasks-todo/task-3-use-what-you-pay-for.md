# Task: Use What You Already Pay For

**From:** owner, 2026-11-22: "some way for people to be able to easily add their existing paying accounts into nandi so they don't pay additional ... if someone has chatgpt or claude accounts or gemini accounts then help them setup local or get key or whatever that particular account type allows."
**Serves:** trust (honest about what each plan allows, keys stay in the keychain). Mostly words and links in setup; the model does the rest.
**Order:** in 0.3.0, after an internal review of how Nandi uses Claude sign-ins.

## 1. "What do you already use?" in Setup

A short chooser in the setup's model step (Step2_Model). Each choice shows only what that plan really allows, in two or three steps, with a link or a **Do it for me** button where one exists.

| You pay for                                     | What Nandi can use                                                                                                                                          | Extra cost                       |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- |
| Claude Pro, Max, Team or Enterprise             | Claude Code agents, signed in through Anthropic's own flow                                                                     | none for Claude Code             |
| Company AWS or Azure                            | Amazon Bedrock through AWS SSO, Azure AI Foundry. Already supported                                                                                        | the company's                    |
| ChatGPT Plus, Pro or Team                       | Not the subscription: Codex in the container needs an OpenAI API key (a ChatGPT login can't go in a container). Say so plainly, link to the key page        | API usage, billed apart          |
| Gemini or Google AI Pro                         | Not the subscription. A Gemini API key from Google AI Studio has a free tier. Needs a `gemini:` provider (Google's OpenAI-compatible endpoint): a new slice | free tier, or usage              |
| Microsoft 365 Copilot, GitHub Copilot           | No API for other apps. Point to the company's Azure AI Foundry if it has one                                                                               | none possible                    |
| Nothing yet                                     | The free tokens, or local models through Ollama (free, on your own computer)                                                                               | none                             |

- [ ] Words only where possible: `locales/*.json` and links. No plan detection, no scraping: the person picks.
- [ ] Honest copy: never promise a subscription works where it doesn't. Note that Google's free tier may use prompts to improve its products, so it isn't private.
- [ ] `docs/testing/` entry and the user guide.

## 2. A Gemini Provider (separate slice)

- [ ] `gemini:` models through `https://generativelanguage.googleapis.com/v1beta/openai/` with a key in the keychain, like DeepSeek. Free-tier rate limits shown calmly.
