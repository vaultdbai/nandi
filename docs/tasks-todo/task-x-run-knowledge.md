# Runs teach the company: a knowledge base from past runs

Status: idea, not started. Post-demo work (demo scope rule: noted here, not built now).

Original idea (owner, 2026-10-27): When people ask a question, some of it can be answered straight from past runs: what a capability found, why a job failed, and how it was fixed. Chief of Staff should reach the knowledge of every run, either directly or by asking the agent that holds those runs. Over time this makes Nandi far more useful: a failure that happened before takes the user to the fix, and can solve it automatically.

## The idea

Every finished run leaves a short **lesson**. The agent that ran it keeps the lesson. Chief of Staff finds lessons through relay and answers from them, with sources.

## 1. What a lesson holds

Not the raw log, which is noisy and full of private details. The model decides what's worth keeping, as with Chief of Staff notes. No templates or forms.

- **What was asked**, and which capability or connector handled it
- **What happened**: the outcome, and for a failure an **error signature** (e.g. "ECS service X failed health check, task exit 137")
- **What fixed it**: the steps, the edits someone made, who did it
- **Whether the fix held**: a later run succeeded
- **Documents the run produced** (audit report, plan, summary), so questions about them can be answered
- Date, the versions involved, and when it was last confirmed

**Failure → fix pairs are the most valuable kind.** A run fails, someone changes something, the next run of the same workflow succeeds. The difference between the two runs is the lesson. Spot these pairs automatically.

**Never secrets.** Remove keys, tokens and customer data when the lesson is written, not when it's read.

## 2. Where lessons live

Follows the "no extra registry; find owners live from relay" rule.

- **Each agent keeps the lessons from its own runs**, locally.
- **Each agent shares a small index** over relay: capability, error signatures, one-line lessons. No raw data.
- **Chief of Staff searches the indexes**, then asks the owning agent for details: "Priya's agent fixed this exact ECS error twice last month. Want the steps?"
- **Knowledge belongs to the capability, not the person** (to decide, see below). If Priya leaves or hands the audit over, its lessons go with the capability to its next keeper.

## 3. Answering a question

1. Exact match on an error signature. This is the strongest signal for failures.
2. A similar past question, and which capability handled it.
3. If a lesson is confident and recent: answer directly and cite it ("based on run #482, Oct 12").
4. If it's unclear: ask the owning agent. If there's still nothing: run the work for real, which writes a new lesson.

"Last time" means the most recent matching run, not the best search hit. Every answer names the run it comes from.

## 4. Questions from people who don't own the capability

Example: "What was the last audit outcome, and why did we fail last time?", asked by someone who doesn't run the audit.

1. The asker's Chief of Staff finds the owner from the index (the audit capability, kept by Priya's agent).
2. It sends the **question** to Priya's agent over relay. It doesn't ask for the document.
3. Priya's agent checks the sharing level for that capability and that person.
4. It answers within that level, with every claim cited: "Oct 20 audit passed. The Sept 15 audit failed on 3 access-review findings; they were fixed by quarterly reviews and removing 12 stale accounts."
5. It logs the exchange, so Priya can see who asked what and what was shared.

Answering about a document and handing over the document are separate permissions.

### Sharing levels

Each capability gets a default, which the owner can override for specific people or teams.

| Level   | What a non-owner gets                                                         |
| ------- | ----------------------------------------------------------------------------- |
| Open    | Summary answers, plus the document if asked                                   |
| Summary | Answers to questions, never the document itself (proposed default for audits) |
| Ask me  | The owner gets a prompt: allow once, always for this person, or decline       |
| Private | "This exists; ask the owner directly"                                         |

"Always allow for this person" is essential. Without it owners get asked every time, ignore the prompts, and the feature dies.

### Rules

- **No leaks through the model.** When answering a non-owner, the agent is given only what the sharing level allows (summary and lessons), never the raw document. This is enforced in what the agent is given, not by trusting the prompt.
- **What the asker's agent keeps:** the answer, with where it came from and when, and it expires. It doesn't pass the answer on to a third person, who should ask the owner's agent themselves.
- **Owner offline:** answer from the index ("Sept audit failed on access reviews; details from Priya's agent"), queue the full question, and follow up when the owner's agent is back, as lasting assignments already do.
- **Visibility follows the run:** someone who couldn't see the run gets only the one-line lesson and the owner's name.

## 5. Fixing things automatically

A fix moves up these levels as it proves itself. This fits the existing approval model: risky actions such as an ECS redeploy always ask first.

| Level   | What the agent does                       | When it gets there                 |
| ------- | ----------------------------------------- | ---------------------------------- |
| Suggest | Shows the past fix as steps to follow     | After it has worked once           |
| Offer   | "Run these steps?" One approval runs them | After it has worked more than once |
| Auto    | Runs the fix and tells you afterwards     | Only safe actions, only if allowed |

## 6. Getting better over time

- **Feedback:** each answer from a lesson gets 👍 or 👎. Lessons that keep proving right gain confidence; wrong ones get marked or dropped.
- **Staleness is the biggest risk.** Infrastructure and code change, and an old fix can be confidently wrong, which is worse than no answer. Confidence fades unless a lesson is confirmed again, and lessons from older versions are flagged.

## Order

1. **Failure → fix lessons only:** write the lesson when a run ends, the agent keeps them locally, Chief of Staff searches its own agent's lessons by error signature and says "this happened before, here's what fixed it."
2. **Shared indexes over relay**, and the owner's agent answering non-owners' questions at the Summary level.
3. **Sharing levels and the ask-me prompt**, the exchange log, and offline queueing.
4. **Documents from runs** as answerable knowledge.
5. **Offer-level fixes** (one approval runs the past fix).
6. Feedback, confidence and staleness.
7. Auto-level fixes, if decided.

## Things to decide

- **Where knowledge lives:** with each owner's agent, shared through indexes (proposed), or a company-wide store?
- **Who sees summaries:** everyone at the company, or only lessons from their own team?
- **Default sharing level for sensitive capabilities** such as audits: Summary is proposed.
- **Who sets sharing levels:** the capability owner, or a company admin who sets a minimum and lets owners loosen it from there (proposed)?
- **Knowledge belongs to the capability, not the person:** this matters for handover and changes how storage works.
- **Auto-fix in v1:** include the Auto level, or stop at Offer?
- **Always-on agents (later)** remove the offline gap. Keep v1 free of laptop-only assumptions.

## Test

1. A run fails, a human fixes it, the next run succeeds. A lesson is written with the error signature and the fix, and contains no secrets.
2. The same error happens again. Chief of Staff says it happened before and shows the fix, citing the run.
3. A non-owner asks "why did the audit fail last time?" and gets a cited summary from the owner's agent, not the document. The owner sees the exchange in the log.
4. With the capability at Ask me, the owner gets the prompt. "Always allow for this person" stops further prompts for them.
5. With the owner's agent offline, the asker gets the index answer and a follow-up once the agent is back.
6. A question worded to pull out the raw audit details gets nothing beyond the Summary level.
7. A lesson from an older version is flagged as possibly stale.
