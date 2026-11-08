# Chief of Staff knows you: a personal knowledge base

Status: idea, not started. Post-v1 work (demo scope rule: noted here, not built now).

Original idea (owner, 2026-11-07): Chief of Staff should look things up before it answers: everything about Nandi, the user's notes, and their history. It should also be able to read the user's own folders, such as Documents and Pictures, so it knows their family and the people around them. Later, health agents could review medical results automatically, and Chief of Staff could comment on buying habits against the user's goals, for example "you're buying a lot of alcohol, and you said you want to lose weight."

Related: `task-x-run-knowledge.md` (lessons from runs, sharing levels) and `task-x-studio-knows-nandi.md` (product guides, fetched by ID).

## 1. Search first, not a vector database

Different knowledge needs a different lookup:

| Knowledge                         | Shape                                 | Lookup                                             |
| --------------------------------- | ------------------------------------- | -------------------------------------------------- |
| About Nandi                       | Small, ships with the app             | Already done: guide index plus fetch by ID         |
| User notes (Chief of Staff notes) | Markdown files, hundreds to thousands | Keyword search, then read the whole note           |
| History (runs, requests, chats)   | Structured, already in SQLite         | Ordinary queries by time, agent, status and person |
| Lessons from runs                 | Short records with error signatures   | Exact match first, then keyword search             |
| Photos                            | Images                                | Image embeddings and face grouping (see section 3) |

Most questions depend on dates, names, IDs and error codes, where vector search is weak. At one person's or one company's size, SQLite full-text search (FTS5) answers in milliseconds, so a separate vector database doesn't make anything faster.

- **The index is a rebuildable copy.** Files stay the gold copy. FTS5 tables live in the SQLite projection and rebuild from the files. Nothing new to run or back up, and it works offline.
- **The model calls search tools** instead of having text pushed into every prompt: `search_knowledge(query, kinds?, since?)` returns snippets with sources; `read_note(id)` and `get_run(id)` return the full text; `query_history(filters)` returns structured results. The model can retry with different words.
- **A small always-loaded profile:** who the user is, their preferences and active work. Details are fetched on demand.
- **Permissions are checked before results reach the model**, as in the run-knowledge sharing levels.
- **Embeddings later, in the same SQLite file** (`sqlite-vec`), combined with keyword ranking. Nandi Cloud (v2) maps to Postgres with `pgvector`. Keep search behind one interface.
- **Decide with a test set:** 30 to 50 real questions with the expected source. If keyword search misses on rewording, add embeddings. If it misses on dates or structure, improve the queries.

## 2. Personal and company knowledge stay apart

Nandi sells "the company owns the work". Family, health and shopping must never end up in the company record.

- A separate **personal space** with its own storage. Never shared over relay, never in company backups, never handed over when someone leaves.
- The work Chief of Staff can't see personal knowledge unless the user brings something across, case by case. Two modes of one agent, or two agents (to decide).
- If this boundary is fuzzy, companies won't allow the app and people won't trust it with health data.

## 3. Reading the user's folders

- **The user picks the folders.** Nothing is scanned by default. Folders can be removed at any time. The container mounts them read-only (agents run only in the container).
- **Facts with sources, not copies of files.** The scanner writes small records into the personal space, for example "Person: Asha, likely your mother, seen in 340 photos" or "Lab result 2026-03: ALT raised (report.pdf, page 2)". They are files, like other definitions.
- **A "what I know about you" page.** The user can see, edit and delete every fact. This page is the trust feature.
- **The user confirms people and anything sensitive.** Face grouping is automatic; "your mother" is a fact only after the user says so. A guessed relationship or diagnosis is never treated as fact until confirmed.
- **Updates in small steps:** watch for new and changed files, no full rescans.
- **Photos need embeddings:** "pictures from the beach with Mom" can't be found by keyword. Use a local image model for tags or embeddings and a local model for face grouping.

## 4. Where the model runs

Sending health records and family photos to a cloud model raises retention, training and transport questions.

1. **Scan locally** (proposed): small local models for OCR, face grouping and image tags. Only short facts the user has approved go to the cloud model when Chief of Staff answers.
2. Cloud models only under a no-retention agreement, with sensitive categories blocked unless allowed.
3. Everything local: most private, noticeably weaker answers on a laptop today.

Health is its own category with stricter defaults: encrypted with a key in the OS keyring, put in a prompt only when the question needs it, and every use logged.

## 5. Health review

- Summarise, compare with past results, and flag what to ask the doctor: "ALT went from 32 to 58 since March; worth mentioning at your next visit."
- Never diagnose, and never suggest changing medication.
- Legal review before shipping. HIPAA usually doesn't cover a consumer app, but the US FTC Health Breach Notification Rule does, and GDPR treats health data as special category data.

## 6. Habit nudges

- **Only against goals the user set.** "I want to lose 5 kg" or "my doctor flagged fatty liver" lets Chief of Staff link purchases to that goal. Without a stated goal it says nothing. Unprompted moral comments get the feature switched off.
- The user chooses the tone and how often: a weekly summary or a comment at the moment of purchase.
- Purchase data needs a source: bank or card exports, email receipts, or store accounts. Each is a connector with its own sign-in.
- Linking health and spending is a separate opt-in from either alone.

## Order

1. Search layer: FTS5 over notes, lessons and run summaries, the search tools, the always-loaded profile.
2. Personal space, separate from company data, with the "what I know about you" page.
3. Documents: OCR, extracted facts with sources, keyword search.
4. Photos: local face grouping, user-confirmed people, image search with `sqlite-vec`.
5. Goals the user sets, then nudges tied only to those goals.
6. Health: encrypted category, summarise-and-flag only, after the legal review.
7. Purchase connectors.

## Things to decide

- **Product:** is personal Chief of Staff a v1 selling point, or a later consumer product next to the company one? This decides pack or core.
- **One agent or two:** a work and a personal mode of Chief of Staff, or two agents.
- **Local only for photos and health:** acceptable even if answers are somewhat weaker?
- **Backup:** can the personal space sync to the user's own backup (the personal-backup pack)? That needs encryption on the device before upload.
- **Chat history:** searchable as-is, or summarised by the model like Chief of Staff notes?

## Test

1. "What did we decide about X last week?" finds the right note and cites it.
2. A question worded differently from the note still finds it once embeddings are on (test set measures this).
3. Removing a folder removes its facts and index entries.
4. Nothing from the personal space appears in the company record, relay traffic or company backups.
5. A face group is labelled only after the user confirms it; deleting the fact removes it from answers.
6. With local scanning, no file contents leave the machine; only approved facts reach the answering model.
7. A health result is summarised and flagged, with no diagnosis.
8. With no goal set, buying alcohol gets no comment. With a weight-loss goal, the weekly summary mentions it.
