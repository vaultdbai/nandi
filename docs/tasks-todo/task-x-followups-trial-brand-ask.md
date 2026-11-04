# Task: Follow-ups from the free trial, bison brand and Ask rename

**From:** review and wrap-up on 2026-10-20, after `ea5f4e6` (trial fixes), `0d37ed9` (mascot poses, icons) and `711352c` / `52ed930` (Studio is called Ask, second in the sidebar).
**Size:** S items, independent. Some need the owner, not code.
**Elsewhere:** the live-model acceptance checks for Studio guides and start-request are in `task-x-studio-knows-nandi.md`. Tasks 1 to 3 have their own files.

## Owner actions (no code)

- [ ] **DeepSeek spending cap.** Set a hard monthly cap on the DeepSeek account behind `NANDI_TRIAL_DEEPSEEK_KEY`. The key is only masked in the binary (`build.rs`), so anyone can recover it, and deleting the keychain entry `nandi.trial.deepseek.used` resets a computer's allowance. The cap is the only real limit. A Nandi-run proxy is the step after that if abuse shows up.
- [ ] **Key rotation plan.** Decide how to rotate the key (new CI secret, new release; old builds lose the trial when the old key is revoked) and write it down in `docs/developer/sign-in.md` or next to `services/llm_trial.rs`.
- [ ] **Repo secret.** Make sure `NANDI_TRIAL_DEEPSEEK_KEY` is set in the GitHub repo, or release builds ship without a trial.
- [x] **Untracked files in the main checkout.** Done 2026-11-01: the brand kit and `docs/ideas/` moved out of this repo.

## Check in the running app

- [ ] Switching palette in Settings changes the title-bar bull and the taskbar icon (Windows) to that palette's bison.
- [ ] Settings › Preferences › rename mascot shows the full-body bison; it changes pose when an approval waits (`wait`), agents run (`carry`) and after an approval (`done`).
- [ ] A release build (`pnpm tauri build`) shows the simple mark at 16 and 24 px (Explorer, Start menu) and the nandi tile from 32 px up.
- [ ] Ctrl+K: the page is listed as Ask and the command reads "New chat in Ask".
- [ ] Free trial with a real key: the counter goes down after a call, a cancelled run still counts, turning the trial on then off gives back the old default model.

## Code

- [ ] **Two windows share one open file (Ask).** `StudioService.open_artifact` is one app-wide value set by `setStudioOpenArtifact`. With the desktop window and a web client both on Ask, whichever reported last wins, and either one leaving clears it for both, so the chat can read "this" as the other window's file. Key it by window or client (the HTTP token, or the Tauri window label) and pass that with each chat message.
- [ ] **macOS Dock icon follows the palette.** `useWindowIconEffect` sets the window icon, which Windows and Linux show in the taskbar. The Dock keeps the bundled nandi icon. Needs a small native call (`NSApplication.setApplicationIconImage`) behind a command.
- [ ] **One-colour silhouette** (`nandi-mark-mono`, prompt 8 in `GEMINI_ICON_PROMPT.md`): black bison head on white, for a future tray icon (macOS template image) and a monochrome favicon. Not needed until the app has a tray icon.
- [ ] **Trial status is read per call.** `llm_trial::chosen` and `available` read the keychain on every DeepSeek call. If that's slow on macOS, cache the mode in memory and refresh it when it's set.

## Acceptance

- [ ] The owner actions are done or written down as decisions.
- [ ] The app checks above pass, or each failure has its own fix.
