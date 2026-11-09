# Task: Files: share, save, create, attachments and drafts (safe files only)

**From:** owner, 2026-11-07. "Share files, save and create them, download email attachments when required, attach files to drafts. Only files that are always safe, nothing else, and below a safe size."
**Size:** L. App first; sharing with a person needs the relay (Needs relay: yes for that part only).
**Builds on:** intake screening (`services/intake/screen.rs`, which notes a malware scanner plugs in later), the email connectors (`services/email/`, read-only today), the user's folders and notes (`services/live/folders.rs`), the agents' container (files reach it through mounts, never the host directly).

## The safe-file rule (applies to every path below)

- [ ] **Allowlist only**, decided by the file's content (magic bytes), not its name. A file whose content and extension disagree is refused.

  | Kind      | Types                                                                                                   |
  | --------- | ------------------------------------------------------------------------------------------------------- |
  | Documents | PDF (no JavaScript, no embedded files, no launch actions), plain text, Markdown, CSV                    |
  | Images    | PNG, JPEG, GIF, WebP (decoded and re-encoded before use, metadata stripped)                             |
  | Office    | DOCX, XLSX, PPTX without macros, embedded objects or external links (`.docm`, `.xlsm`, `.pptm` refused) |

- [ ] **Always refused:** executables and installers, scripts (`.js`, `.ps1`, `.bat`, `.sh`, `.vbs`, …), archives (`.zip`, `.7z`, `.rar`, `.iso`, `.img`), HTML and SVG (they carry scripts), shortcuts (`.lnk`, `.url`), anything encrypted or password-protected, anything the checks can't parse.
- [ ] **Size limit:** 10 MB per file by default (Settings; an admin can lower it, never above 25 MB); a total per email or share.
- [ ] **Malware scan** once a scanner is available (the scanner item in the intake task): a file that fails is refused and the item says why.
- [ ] A refused file is never stored, opened or passed to a model: the item shows "1 attachment held back: invoice.zip (archives aren't allowed)".

## 1. Email attachments, when needed

- [ ] The mail tools list attachments (name, type, size) without downloading them.
- [ ] `mail_get_attachment` downloads one **only when the work needs it** (the Chief of Staff or an agent asks, in a run the user approved, or the user clicks it), through the safe-file rule, into the run's files (not the user's folders) for the container to read.
- [ ] Attachments in queued items are screened with the item; an item with a refused attachment can still be approved without it.

## 2. Agents' results are files

Agents often answer with a file: a report PDF, a CSV, a chart, a draft document. Those files are first-class results.

- [ ] **A file card** wherever the result shows (the chat, the intake thread, the Request page, Run Detail's Outputs, Home's "Since you were away"): name, type, size, and a preview (PDF pages, images, CSV as a table, Markdown rendered, DOCX/XLSX as text or a table).
- [ ] **What the user can do with it:** open, download, save to a granted folder, attach to a draft (section 5), share with a person (section 4), or hand to another agent.
- [ ] **Leaving the run:** a result file leaves the container (preview, download, save, share, attach) only if it passes the safe-file rule. Anything else an agent writes (a zip, a script, a binary) stays in the run's outputs, listed with "kept inside the run: archives aren't allowed out"; text-like ones can be shown as text.
- [ ] The Chief of Staff's notes and the thread say which files a run produced, so "send me last week's report" finds it.

## 3. Save and create files

- [ ] Agents create files in the run's output folder (as today) and, with the user's approval, save them to a folder the user granted (`services/live/folders.rs`), through the same rule (a script an agent wrote is saved as text, never as runnable, and only if the user asks for it).
- [ ] Notes can hold safe files (images, PDFs) next to the Markdown.

## 4. Share files

- [ ] **Between the user's own agents:** a file from one run passed to another (a workflow step, a delegation), by reference, checked again on arrival.
- [ ] **With a person** (relay): sealed end to end like tasks, within the size limit, through their intake (screened, queued for their approval). Needs relay: yes.
- [ ] Never to an outside service unless a connector the user granted sends it, with approval.

## 5. Attach to drafts

- [ ] Drafts of email replies (created in the user's mailbox, never sent by Nandi) can carry safe files from the run or the user's folders. Needs `Mail.ReadWrite` on Outlook and `gmail.compose` on Gmail: new scopes, user consent, under the owner's "read and draft, never send" decision (2026-11-07, `task-5-email-notes-v1.md`).
- [ ] The draft opens in the user's mail app for them to review and send.

## 6. Attach in the ask box and chats

- [ ] **Attach button** in the ask box (Home, Studio, the companion), agent chats and intake threads: pick files or drop them in. Agent chats already take attachments (`send_message` `attachments`); bring the ask box and threads to the same, through the same safe-file rule.
- [ ] **Paste becomes a file:** pasting an image (a screenshot) attaches it as a PNG; pasting long text (over ~2,000 characters, or anything that looks like a log, CSV or code) attaches it as a `.txt` / `.csv` / `.md` file with a short preview chip, instead of flooding the box. A short paste stays as text. The user can turn a chip back into text.
- [ ] The chip shows name, type and size, and a refused file says why ("archives aren't allowed").

## Tests

- Each allowed type passes; each refused kind is refused by content, including a renamed `.exe` called `invoice.pdf`, a PDF with JavaScript, a `.docm`, an SVG with a script, a zip.
- Size limit per file and per item; an oversized attachment is held back and the item still works.
- An attachment is downloaded only when asked; a refused one never reaches a model or the container.
- A shared file is re-checked on arrival; a draft carries only safe files.
- A result PDF and CSV show as cards with previews; a zip an agent wrote stays inside the run.
- Pasting a screenshot attaches a PNG; a long paste becomes a file chip, a short one stays text; a refused file in the ask box says why.
