# Task: Desktop Pilot — agents drive allowed desktop apps on the host (v2)

**Target:** v2. Not in v1.
**Goal:** agents can launch allowed desktop apps on the user's machine, operate them, and read data out of them. Company apps with no API are the point: SAP GUI, desktop Excel and Outlook, line-of-business tools, Citrix-hosted apps. A typical job: "Pull this report out of the desktop app every Monday."
**Size:** XL

## Why it needs the tightest limits

Launching programs and driving the mouse and keyboard is, in practice, arbitrary code on the host, which the "agents run only in the container" rule exists to prevent. Pilots are the deliberate exception (`docs/developer/pilots.md`). This one gets the strictest controls of any pilot:

- **Only the app talks to the pilot.** The orchestrator calls it and gives the model `pilot_desktop__<tool>` tools. Containers are never given its address. Coding agents get it later, through Nandi over ACP, the same way as in the browser pilot's phase 4.
- **The placeholder desktop pilots are gone** (Win Pilot, UIA-X, MacPilot and Desktop Pilot were never published; removed from `pilots/catalog.rs` on 2026-10-29), so this fills that slot.
- **It stays separate** from the browser pilot (`task-x-browser-pilot.md`, for websites) and the machine doctor (`task-x-machine-doctor.md`, fixed diagnostic commands only, never this pilot).

## Design

### Which apps

- **An allow-list of apps**, identified by executable path and publisher signature. The user adds apps in the pilot's Settings, and policies can narrow the list (an `apps` rule block, like `pilots.allow`).
- **Never allowed, by design:** shells and script hosts, because launching one is arbitrary code. That covers `cmd`, PowerShell, Windows Terminal, Terminal.app, bash/zsh, `python`, `node`, `wscript`/`cscript`, `regedit`, `osascript`, and the like.
- **Launching needs approval**, with "always allow this app for this agent" offered after the first time.

### How it sees and acts

1. **The accessibility tree first:** Windows UI Automation, macOS Accessibility, Linux AT-SPI. It's structured, reliable and cheap: named buttons, fields and text.
2. **Pixels only as a fallback,** for apps without a usable tree (Citrix, canvas, remote desktops): window capture, local OCR and click coordinates.
3. **OCR runs on the machine** with the OS's own engine (Windows.Media.Ocr, macOS Vision). It's free and private, and needs no extra install.
4. **`read_all(window)`:** scroll, OCR and remove duplicates on the host, and return text. The model doesn't pay for a screenshot per scroll.
5. **Only the target window is captured,** never the whole screen. Captures are saved as run files the user can see.

### Safety at run time

- **Focus check:** before every click or keystroke, the pilot confirms the window in front belongs to an allowed app the agent launched or was given, and refuses otherwise.
- **Takeover mode:** a visible overlay while the agent drives the mouse and keyboard.
- **Kill switch:** a global hotkey that stops the agent at once.
- **Pause on user input:** if the user moves the mouse or types, the agent stops and waits.
- **Grants in tiers, per agent:**
  - `desktop.read` (accessibility tree, OCR, window capture)
  - `desktop.input` (click, type)
  - `desktop.launch`
  - `desktop.clipboard`
- **Approvals that look at the call** (the generic classifier from the browser pilot), holding:
  - clicks on elements named like Send, Submit, Delete, Pay, Save or Post
  - typing into password fields
  - closing an app with unsaved work
  - reading the clipboard
- **Audit:** every call goes to `pilots.log` with redacted arguments, the app, the window and the outcome. Captures are linked from the run.

### Build

A built-in pilot written in Rust inside the app, not a third-party server. Here the safety checks are the product: the allow-list, the focus check, window-only capture and redaction. Candidates, to verify at the time:

- Windows: the `windows` crate (UI Automation, Windows.Media.Ocr)
- macOS: Accessibility and Vision bindings (`objc2`)
- Screen capture: `xcap`
- Input: `enigo`

### Platforms

- **Windows:** first, and best supported.
- **macOS:** second. The user grants Nandi Accessibility and Screen Recording permission once in System Settings; the app guides them through it and detects when it's missing.
- **Linux:** best effort. X11 works, but Wayland needs the system's RemoteDesktop/ScreenCast permission prompt for input and capture.

## Phases

- [ ] **1. Read only (Windows):** the app allow-list and the never-allowed list, launch with approval, the accessibility tree, window capture, local OCR, `read_all`, audit.
- [ ] **2. Input:** click and type, the focus check, takeover mode with the kill switch, pausing on user input, and the classifier rules.
- [ ] **3. macOS:** permissions onboarding, Accessibility and Vision.
- [ ] **4. Policies and coding agents:** the `apps` policy block, and coding agents reaching the pilot through Nandi over ACP.
- [ ] **5. Linux (best effort):** X11 first, then Wayland portals.
- [ ] Each phase adds its CP- entries to `docs/testing/`.

## Acceptance

- [ ] The user allows an app; the agent launches it (after approval), reads a table from it through the accessibility tree or OCR, and returns it as text.
- [ ] A shell or an app not on the list can't be launched, and the tool result says why.
- [ ] With another window in front, the agent's typing is refused, not sent there.
- [ ] The kill switch stops the agent at once, and the user moving the mouse pauses it.
- [ ] Clicking "Send" or "Delete" waits for approval in the Inbox.
- [ ] Every call is in the audit log, and captures cover only the target window.

## Out of scope

Running apps in a separate desktop, VM or sandbox (later); continuous screen recording; games and full-screen apps; driving apps from inside the container.
