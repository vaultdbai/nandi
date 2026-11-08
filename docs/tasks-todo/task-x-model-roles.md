# Task: Model roles (pick models by what the work needs)

**From:** owner, 2026-11-07. Not prioritized yet.
**Why:** GIFs, images and video monitoring need a model that reads them; screening and planning can use a small, cheap one. Today there is one default model and an optional per-agent `model:`.

## Decisions so far

- [x] **Roles, not hard-coded models.** Packs still set no `model:`. An agent declares what it needs; the user maps each role to a model.
- [x] **Company sign-in first, API keys last**, as for the default model. An unset role falls back to Default.
- [x] **Per-agent `model:` stays** for the user's own agents and wins over roles.

## Roles

| Role         | Used for                                               |
| ------------ | ------------------------------------------------------ |
| Default      | Chat, the Chief of Staff, most agents                  |
| Vision       | Screenshots, GIFs, images, the browser pilot's screens |
| Video        | Recordings, monitoring feeds                           |
| Fast         | Classifying, screening, planning, check summaries      |
| Long context | Big documents, whole threads, codebases                |

## Work

- [ ] **Model capabilities:** what each configured model can take (images, video, context size), from the provider's model list where it says so, else a small table in code.
- [ ] **Roles in Settings › Models** (portable, in `.nandi/settings.yaml` next to the default model).
- [ ] **Agents declare needs** in `skill.md` (`needs: [vision]`), checked by the domain; the registry entry repeats them so the catalog can say "needs a vision model".
- [ ] **Routing per call:** a turn whose input has an image or video goes to the Vision or Video model for that step, then back to the agent's model. Nandi's own calls (ask classifier, planner, screening summaries) use Fast.
- [ ] **No model for a need:** the run says which role to set, in plain words, never a raw provider error.
- [ ] Tests: role fallback, per-agent override wins, routing by input type, an agent whose need has no model.

## Not in scope

- Choosing models automatically by price or quality.
- Fine-tuned or local models beyond what the providers already offer.
