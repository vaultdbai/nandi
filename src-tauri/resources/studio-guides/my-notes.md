---
id: my-notes
title: My notes
when: The user's notes and what Chief of Staff remembers.
---

**My notes** (left menu) are the user's own Markdown files: role, what they own, deadlines, who to ask, decisions, runbooks. They live in one folder, shared with agents as `my-notes`: the folder the user already shared under that name, otherwise `~/.nandi/notes`, created on first open.

- **Chief of Staff keeps them.** It reads them to answer questions about the user's work (for the user and for teammates who ask), and writes them as it learns things during a chat with the user. It doesn't watch in the background; the how-nandi-works guide says when it writes. The user decides how they're arranged; the app imposes no structure.
- **Writing.** With **Let Chief of Staff keep them up to date** on (the switch on the page), the share is read-and-write, and agent writes into `my-notes/` don't ask for approval. With it off, the folder is read-only to agents.
- **Editing by hand.** On the page: pick a note, then **Edit** (a Markdown editor; Ctrl+S saves), or **New note** (e.g. `runbooks/etl`).
- **Private.** Files in `my-notes/private/` are never used to answer other people.
- **What teammates can find** comes from NANDI.md `responsibilities` (Network › Sharing › What you offer), not from the notes.

If the user wants something remembered ("remember that…", "note that…"), suggest telling Chief of Staff, which files it in the right note. Chief of Staff comes with Nandi (first under Agents). Ask itself doesn't write notes.
