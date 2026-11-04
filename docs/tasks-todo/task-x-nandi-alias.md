# Task: Call Nandi anything

**From:** owner request 2026-10-21: "allow people to give any alias to Nandi".
**Size:** S–M. Mostly wiring an existing setting through to the places that still say "Nandi".

## Today

- The mascot already has a name the user can change: Settings › Preferences › rename mascot (`RenameMascot.tsx`), stored as `mascot_name` in `~/.nandi/config.json` (`services/mascot.rs`, `DEFAULT_NAME = "Nandi"`, 1–24 characters), shared with the CLI (`cli_mascot.rs`), read in the UI with `useMascotName()`.
- It's used in only a few places: the title bar, Ask's assistant label (`displayAgent('studio', useMascotName())`), the mascot's spoken lines and `NandiBull`'s label.
- Everywhere else "Nandi" is hard-coded as the assistant:
  - Ask's system prompt: "You are Nandi, the assistant on the Ask page…" (`services/studio/prompt.rs`).
  - The companion's title and its "Open in Nandi" / Quit labels (`companion.title`, `companion.openInNandi`, `CompanionHeader.tsx`).
  - Fallback display names in `orchestrator.rs`, `run_analysis.rs`, `snapshots.rs`.
  - The runtime label for native agents (`agent-page-model.ts`, `runtime-options.ts`).
  - Notifications and approval text that name the assistant.

## Decide first

- **Assistant vs product.** The alias should name the _assistant_ (the bison, Ask, the companion, "X needs your approval"). The _app_ is still Nandi: the window title, the installer, menus ("Quit Nandi"), docs, Settings › About, and the "Nandi (built in)" runtime. Confirm this split, or decide that the alias replaces the product name in the UI too.
- **One name or more.** Recommended: one display name plus optional extra names it answers to (see "Calling it by name"). Keep a single `mascot_name` as the display name.
- **Per person.** The alias is local (`config.json`), not in the repo, so teammates sharing a repo each keep their own. Network members still see the sender's product name, never the alias.

## Work

- [ ] Rename the setting in the UI from "mascot" to "Your assistant's name" (keep the `mascot_name` key and migrate nothing). Show it in Settings › Appearance › Companion as well as Preferences, with a live preview of the bison and a "Reset to Nandi" button.
- [ ] Ask's prompt: "You are {alias}, the assistant in Nandi…" (`studio_prompt` gets the name from `PromptFacts`; tests: `the_prompt_matches_the_code` with a custom name, and the default still reads "Nandi").
- [ ] Companion: header title, the orb's tooltip and its aria label, "Ask {alias}" placeholder; the main window keeps "Open in Nandi".
- [ ] Ask page: the empty state, the New menu's "New chat with {alias}", starter prompts that name it.
- [ ] Notifications and approval cards that name the assistant ("{alias} needs your approval to …").
- [ ] Backend fallback names (`orchestrator.rs:1823`, `run_analysis.rs:178`, `snapshots.rs:282`): use the alias for the assistant's own messages only; agents keep their own names.
- [ ] CLI: `nandi mascot name` already reads it; make CLI messages that speak as the assistant use it.
- [ ] i18n: every string that names the assistant takes `{{name}}` (en, fr, ar, ja). Grep the locales for "Nandi" and sort each hit into assistant (parameterise) or product (keep).
- [ ] Validation stays as today (1–24 characters, letters, digits, spaces, `-`, `'`, `.`). Refuse names that are the same as an agent's name or ID, so "@name" stays unambiguous.

## Calling it by name (optional, second step)

- [ ] Ask and the companion treat "{alias}, …" and "@{alias} …" the same as a plain message to the assistant (today "@Agent …" sends to that agent: `companion::mention_target`). The alias must win over an agent with the same name, hence the validation above.
- [ ] Extra names it answers to (`mascot_aliases: [..]` in `config.json`, up to 5), e.g. a short form or the old name after a rename.

## Out of scope

- Changing the app's own name, icon or bundle ID.
- A voice wake word.
- Per-agent aliases (agents already have their own names).

## Acceptance

- [ ] Rename to "Juno": the title bar, Ask's chat label, the companion card and its tooltip, approval notifications and the bison's lines all say Juno; the window title, menus and About still say Nandi.
- [ ] In Ask, "who are you?" answers as Juno.
- [ ] Reset brings back "Nandi" everywhere it changed.
- [ ] An agent can't be named Juno while the assistant is (and the reverse), with a plain error.
