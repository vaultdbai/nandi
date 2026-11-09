# Task: Move hand-written heuristics into prompts

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** less code; the model does the reading
**Size:** S per item, one at a time

Code that guesses what text means belongs in a prompt or a `skill.md`. Keep the state and the checks in code. Do one item per change, with a test that the behaviour still holds.

| ID           | Today (code)                                                          | Instead                                                                    | Keep in code                     |
| ------------ | --------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------- |
| AI-07        | Ask's keyword fallback and a 0.5 confidence threshold                 | The model classifies and asks back when unsure                             | The routes it can pick           |
| CH-20        | Memory triggered by keywords ("always", "never") and a 3-step minimum | The agent decides what to remember, guided by its prompt                   | The cap and storage              |
| CP-17        | Ambiguity clustering and "Promote to input schema"                    | The model reads clarifications and proposes the schema change              | The approval to change it        |
| CP-19, TM-13 | A weighted provider score (match 25%, trust 20% …)                    | The model, or a simple order, picks the provider                           | The circuit breaker (state)      |
| GW-12, GW-15 | Gateway direct-match pre-routing and clarifying logic                 | Hand the catalog to the model                                              | Admission, limits, holds (trust) |
| RQ-15        | `next_action` built as an English sentence in code                    | Structured data (`kind`, `step`, `deadline`); the UI and the model word it | The fields                       |

## Done when

Each row is either moved (code removed, prompt added, test passes) or kept with a reason written here.
