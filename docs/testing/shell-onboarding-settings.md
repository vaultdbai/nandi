# App shell, first run and Settings

Covers app launch, the first-run gate and onboarding, the main window (title bar, sidebar, panes), Home, the command palette and command system, shortcuts, menus, toasts and notifications, the updater, the error boundary and recovery files, logging, themes and languages, the mascot, and every Settings section. Data paths: identity lives in `~/.nandi/repo/NANDI.md`. Portable settings live in `~/.nandi/repo/.nandi/settings.yaml`. Machine config lives in `~/.nandi/config.json`. Machine-local state lives in `~/.nandi/local/state.db` (`local_settings`). Secrets live in the OS keychain. The startup copy of the preferences and the `recovery/` folder live in the Tauri app data dir. Its identifier is `com.tauri-app.app`, so on Windows that is `%APPDATA%\com.tauri-app.app\`.

There is no sign-in today. Identity is local: a display name and a handle (the "user slug") in `NANDI.md`. See Gaps for the GitHub and Entra docs, which are stale.

## 1.1 Launch and first-run gate

### ON-01 — First-run gate (AuthGate)

- **What:** At launch the app reads the local identity. It shows a spinner, then either onboarding or the workspace. If the identity can't be read it "fails open": the app opens anyway and the error is logged.
- **Where:** `src/components/layout/AuthGate.tsx`, `src/components/onboarding/identity-model.ts` (`needsOnboarding`), `src-tauri/src/commands/identity.rs` (`get_identity_status`), `src-tauri/src/services/auth/mod.rs`; `GET /api/identity/status`.
- **Preconditions:** None.
- **Steps:**
  1. Start with a fresh `NANDI_HOME` (no `NANDI.md`) and launch the app.
  2. Finish onboarding, then quit and relaunch.
  3. Set `onboarding.step` to `"packs"` in `local_settings` (or quit during step 2–4) and relaunch.
- **Expected:** (1) The spinner shows, then the "Welcome to Nandi" onboarding (`data-testid="onboarding-gate"`). (2) The app opens straight on Home. (3) Onboarding opens again at the stored step. `IdentityStatus` reports `onboarded`, `displayName`, `userSlug`, `branch` (`<prefix>/<slug>`) and `onboardingStep`. The workspace opens only when the home is onboarded (a saved name) and the stored step is `done`.
- **Edge / negative cases:**
  - Make `get_identity_status` error (for example, unreadable `NANDI.md` frontmatter): the app opens and a warning is logged ("Failed to load identity status; opening the app").
  - A home with a name but no stored step opens onboarding (resuming at Model), not the workspace.
- **Automation:** Covered by: `src/components/layout/AuthGate.test.tsx` (including 'keeps onboarding open while a named home has no step recorded'), `src/App.test.tsx`, `src/components/onboarding/identity-model.test.ts`, Rust tests in `src-tauri/src/services/auth/mod.rs` (`identity_status_tracks_writes_and_emits`, `onboarding_step_is_remembered_per_home`) | Layer: api | Gap: no ui-e2e for relaunch resume.
- **Needs relay:** no
- **Status:** Shipped

### SH-01 — Startup sequence

- **What:** On mount the app does six things: starts agent event sync, registers the command system, loads the saved language (or the system locale), builds the native menu (desktop only), removes recovery files older than 7 days, and starts update checks (desktop only).
- **Where:** `src/App.tsx`, `src/main.tsx`, `src/i18n/language-init.ts`, `src/lib/menu.ts`, `src/lib/recovery.ts`, `src/store/update-store.ts`, `src-tauri/src/lib.rs`.
- **Preconditions:** None.
- **Steps:**
  1. Launch the desktop app with devtools open.
  2. Read the console and stdout.
- **Expected:**
  - Logs include "Frontend application starting up", "Command system initialized", "Application menu built successfully" (desktop) and "App environment".
  - Rust logs "Application starting up".
  - The web server logs "Web access on http://..." (token in `~/.nandi/web_token`) or "Web server not started: ...".
- **Edge / negative cases:**
  - Preferences fail to load: a warning ("Failed to initialize language or menu") and English is used.
  - The browser transport (`--server`): no native menu and no update checks.
  - A `~/.nandi/local/state.db` from another schema version (not v2) is refused at startup: "The local database is from another Nandi build (schema N, this build uses 2). Quit Nandi, delete ~/.nandi/local/state.db with its -wal and -shm files, and start it again…" (data area).
- **Automation:** Covered by: `src/App.test.tsx`, Rust `db.rs` (`a_database_from_another_schema_is_refused_with_what_to_do`) | Layer: ui-e2e | Gap: no test asserts the order of startup calls.
- **Needs relay:** no
- **Status:** Shipped

### SH-02 — Single instance

- **What:** Launching a second copy focuses and unminimizes the existing main window instead of opening a new one. The lock is skipped when `NANDI_HOME` is set, so two homes can run side by side.
- **Where:** `src-tauri/src/lib.rs` (`tauri_plugin_single_instance`), `utils::nandi_config::has_custom_home`.
- **Preconditions:** A desktop build.
- **Steps:**
  1. Launch the app.
  2. Minimize it.
  3. Launch it again.
  4. Repeat with `NANDI_HOME=~/.nandi-b` for the second launch.
- **Expected:** (3) The first window is restored and focused, and there is no second process window. (4) A second, independent window opens with its own state.
- **Edge / negative cases:** The main window is hidden (closed to the companion) when the second launch happens. The existing window is focused, but check whether it becomes visible.
- **Automation:** Covered by: none | Layer: manual | Gap: manual only.
- **Needs relay:** no
- **Status:** Shipped

### SH-03 — Window size and position remembered; closing hides the window

- **What:** The main window's position, size and state are saved and restored (the companion is excluded). Closing the main window hides it instead of quitting, so agents keep running. On Windows and Linux the companion is shown as the way back. On macOS the Dock icon reopens the window. A real quit (Cmd+Q, Quit from the companion's menu) stops the orchestrators and unregisters global shortcuts.
- **Where:** `src-tauri/src/lib.rs` (`tauri_plugin_window_state`, `RunEvent::WindowEvent CloseRequested`, `RunEvent::Reopen`, `RunEvent::Exit`), `commands::companion::show_after_main_closed`. Window defaults are in `src-tauri/tauri.conf.json`: 1200x800, minimum 1000x700.
- **Preconditions:** A desktop build.
- **Steps:**
  1. Move and resize the window, then quit and relaunch.
  2. Click the window's close button while an agent runs.
  3. On macOS, click the Dock icon.
  4. Quit from the companion's menu.
- **Expected:**
  - (1) The window reopens at the same place and size.
  - (2) The window disappears (log "Main window hidden"), the process keeps running, the agent run continues, and the companion appears (Windows/Linux).
  - (3) The window comes back (log "Main window reopened from dock").
  - (4) Log "Application exiting — performing cleanup".
- **Edge / negative cases:**
  - A saved position on a monitor that is gone.
  - Closing while a run is in flight, then quitting: the run is marked interrupted or resumes on the next launch (see the runs area).
- **Automation:** Covered by: none | Layer: manual | Gap: manual only.
- **Needs relay:** no
- **Status:** Shipped

## 1.2 Onboarding (first-run setup)

### ON-02 — Onboarding steps, step indicator and resume

- **What:** First-run setup has six steps: Email (skippable), Name, Model, Computer (recommended, skippable, ON-16), Network, Agents (starter packs). Every step after Email has **Back** (Name goes back to Email). The heading says "A few quick steps. You can change all of this later." (no count). A numbered indicator shows done steps with a check; the wizard keeps one width on every step and the step labels wrap rather than cut off ("Na…"), in every language including Arabic. The step reached is saved per Nandi home, so quitting midway resumes there. A resumed setup offers "Skip the rest of setup".
- **Where:** `src/components/views/OnboardingView.tsx` (`StepIndicator`), `identity-model.ts` (`ONBOARDING_STEPS`, `resumeStep`), `set_onboarding_step` (`PUT /api/identity/onboarding-step`, body `{"step":"model"}`). Stored in `local_settings` key `onboarding.step` in `~/.nandi/local/state.db`.
- **Preconditions:** A home that is not onboarded.
- **Steps:**
  1. Complete the Email and Name steps and go to Model.
  2. Quit and relaunch.
  3. Click "Skip the rest of setup".
- **Expected:**
  - (1) `onboarding.step = "model"` is stored before the name is written.
  - (2) Setup opens on Model, with Email and Name marked done. Quitting before a name is saved opens on Email (or on Name once Email was left).
  - (3) The step becomes `done` and the app opens on Home.
- **Edge / negative cases:**
  - A failed step save is only logged ("Onboarding: saving the step failed"). Setup continues.
  - `PUT /api/identity/onboarding-step` without `step` returns 400 "Body needs a \"step\"".
  - An unknown stored value counts as unset.
- **Automation:** Covered by: `src/components/views/OnboardingView.test.tsx` ('shows six steps, Email first', 'opens where an interrupted setup stopped', 'renders nothing on a home that finished setup'), `identity-model.test.ts`, Rust `onboarding_step_is_remembered_per_home` | Layer: api + unit | Gap: none significant.
- **Needs relay:** no
- **Status:** Shipped

### ON-03 — Step 2: your name and handle

- **What:** With no mailbox connected, the user enters a display name (1–64 characters). A handle is derived from it and previewed as "Your handle: @slug". "Customize" reveals the handle field, which takes 2–32 characters: lowercase letters, digits and hyphens, starting and ending with a letter or digit. Errors show after the field is left or Continue is pressed. If the name derives no valid handle (emoji only, for example), the handle field opens. **With a mailbox connected in step 1** the step reads "You're Ada Lovelace (ada.lovelace@company.com) · Change" instead of asking: the name and address come from the mailbox, the handle from the address's local part (`ada-lovelace`), and an Outlook mailbox also shows title, department and manager (for example "Analyst · Finance · reports to Priya Rao (VP Finance)"). Continue saves them. Change opens the usual fields, filled in, to edit any of them.
- **Where:** `src/components/onboarding/Step1_Name.tsx`, `identity-model.ts` (`slugify`, `validateName`, `validateSlug`, `mailboxIdentity`, `draftFromMailbox`).
- **Preconditions:** None.
- **Steps:**
  1. Type "Ada Lovelace" and check the preview.
  2. Click Customize and type "Ada!" as the handle.
  3. Clear the name.
  4. Enter valid values and click Continue.
- **Expected:**
  - (1) "@ada-lovelace".
  - (2) "Use lowercase letters, numbers and hyphens…".
  - (3) "Enter your name."
  - (4) A spinner, then the Model step.
- **Edge / negative cases:**
  - A 65-character name: "Use at most 64 characters."
  - Accented names: "Zoë" gives "zoe".
  - A name in non-Latin script only: the handle field is required.
  - RTL: the handle field stays LTR.
  - A mailbox with no name (or one that isn't connected) falls back to the form above. A local part that makes no valid handle (`+++@…`) takes the handle from the name.
  - Settings › Account then notes "Name and handle came from your Outlook mailbox." while the name is unchanged.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('derives the handle…', 'validates, then saves…', 'says who the user is from the connected mailbox…', 'saves the mailbox identity with one click', 'asks for the name when the email step is skipped'), `identity-model.test.ts` (`mailbox identity`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### ON-04 — Saving the identity (NANDI.md)

- **What:** Continue on step 1 writes, in order, the step, the display name and the slug. The core creates `NANDI.md` from the template if it is missing. It changes only the identity keys in the frontmatter (`display_name`, `owner.display_name`, `user_slug`, and `name` while it is still the default) and replaces the file atomically. It emits `identity://changed`. If a remote is configured, its `user_slug` follows.
- **Where:** `set_display_name` and `set_user_slug` (`src-tauri/src/commands/identity.rs`); `PUT /api/identity/display-name` `{name}` and `PUT /api/identity/user-slug` `{slug}`; `services/auth/mod.rs` (`validate_display_name`, `validate_slug`, `validate_slug_for`, `sync_remote_slug`).
- **Preconditions:** None.
- **Steps:**
  1. Finish step 1 as "Ada Lovelace".
  2. Open `~/.nandi/repo/NANDI.md`.
  3. Call `PUT /api/identity/user-slug` with `{"slug":"A"}`.
- **Expected:**
  - (2) The frontmatter has `display_name: Ada Lovelace` and `user_slug: ada-lovelace`. Other keys and the Markdown body are unchanged.
  - (3) An error: "'A' isn't a valid user slug: use 2–32 lowercase letters…".
- **Edge / negative cases:**
  - An empty name: "Enter your name.".
  - A name with control characters is refused.
  - Frontmatter that doesn't parse is never overwritten.
  - A save failure shows the toast "Couldn't save: …" and stays on step 1.
- **Automation:** Covered by: Rust tests in `services/auth/mod.rs` (`creates_nandi_md_from_the_template`, `set_display_name_preserves_the_rest_of_nandi_md`, `set_user_slug_writes_and_invalid_frontmatter_is_never_overwritten`, `validates_slugs`, `validates_display_names`), `OnboardingView.test.tsx` ('stays on the name step when saving fails'), `http/mod.rs` (`wave_final_routes_are_mounted`: the `/api/identity/*` routes answer) | Layer: api | Gap: no test of what the identity routes do over HTTP.
- **Needs relay:** no
- **Status:** Shipped

### ON-05 — Step 3: use a key already on this computer

- **What:** If the keychain already holds a provider key (from another home on this computer), the step says "Found a key for <provider> on this computer." and offers one-click "Use <provider> (<model>)". That sets the default model and moves on.
- **Where:** `src/components/onboarding/Step2_Model.tsx`, `useLlmKeyStatus`, `set_default_model` (`PUT /api/default-model`).
- **Preconditions:** A provider key in the keychain (for example `nandi.llm.anthropic`).
- **Steps:**
  1. Reach step 2.
  2. Click "Use Anthropic (…)".
- **Expected:** `default_model` is written to `.nandi/settings.yaml` and the Network step opens.
- **Edge / negative cases:**
  - Several providers found: "Found keys for A and B…".
  - `set_default_model` errors: a toast, and the step stays.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('uses a key already in the keychain in one click') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### ON-06 — Step 3: enter and test a provider key

- **What:** The user chooses a provider (Anthropic, OpenAI, DeepSeek) and pastes a key. The key is tested first and stored only if it works. When the provider can't be reached (offline), it can be saved untested. A "get a key" link opens the provider's key page. Keys never come back to the webview.
- **Where:** `src/components/models/ProviderKeyForm.tsx`, `KeyTestResult.tsx`; `test_llm_key` (`POST /api/llm-keys/{provider}/test`), `set_llm_api_key` (`PUT /api/llm-keys/{provider}`); keychain entries `nandi.llm.anthropic`, `nandi.llm.openai`, `nandi.llm.deepseek`.
- **Preconditions:** A real or fake key. Network access for a real test.
- **Steps:**
  1. Choose OpenAI and paste a bad key.
  2. Click Test.
  3. Paste a good key and click Test.
- **Expected:** (2) The test result shows the failure (for example "The key was rejected.") and nothing is stored. (3) The toast "API key saved" and the default model is set.
- **Edge / negative cases:**
  - Offline: "Couldn't reach the provider…" and a "Save without testing" button appear.
  - An empty key.
  - The key must never appear in `~/.nandi` files or in logs.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('tests a typed key before storing it', 'never stores a rejected key'), Rust `services/llm_keys.rs` tests (`keys_are_write_only_and_clearable`, `key_test_failures_map_every_provider_answer`, `testing_without_a_key_reports_no_key_and_stores_nothing`) | Layer: api | Gap: a real provider round trip is manual.
- **Needs relay:** no
- **Status:** Shipped

### ON-07 — Step 3: Ollama and Amazon Bedrock options

- **What:** The Ollama choice detects a local Ollama. It says "Ollama isn't running…" or "…has no models yet", with "Check again". Otherwise it offers its models and "Use Ollama". Amazon Bedrock is listed with the hint "Works with the Claude Code runtime. Set it up after setup, in Settings › Models & runtimes."
- **Where:** `Step2_Model.tsx` (Ollama option), `detect_ollama` (`GET /api/ollama`).
- **Preconditions:** Ollama installed or not.
- **Steps:**
  1. Stop Ollama, choose Ollama, and read the message.
  2. Start Ollama with a model and click Check again.
  3. Click Use Ollama.
- **Expected:** (3) The default model is `ollama:<model>`, and the free trial switches off for good (`llm_trial::own_model_set_up`).
- **Edge / negative cases:** Ollama is running with no models.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('says when Ollama is not running', 'offers an installed Ollama model') | Layer: unit | Gap: real Ollama is manual.
- **Needs relay:** no
- **Status:** Shipped

### ON-08 — Step 3: free tokens notice and skipping without a model

- **What:** With no key, while the free trial is offered, the step says "You can start now: Nandi comes with free tokens on <model>". Otherwise it says "Nandi will use <model>" when some model is usable. "Skip for now" asks a second time ("Without a model, agents can't run…", with Go back and Skip anyway).
- **Where:** `Step2_Model.tsx`, `get_default_model` (`GET /api/default-model`), `services/llm_trial/`.
- **Preconditions:** No keys. The trial gateway reachable for the trial line.
- **Steps:**
  1. Reach step 2 with no keys.
  2. Click Skip for now, then Skip anyway.
- **Expected:** (1) The trial or "will use" line. (2) The Network step, and no default model is written.
- **Edge / negative cases:**
  - A dev build without the trial key: no trial line.
  - "Go back" cancels the skip.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('asks twice before skipping without a model'), Rust `services/llm_trial/` tests | Layer: unit | Gap: the trial line against the live gateway is manual.
- **Needs relay:** no
- **Status:** Shipped

### ON-09 — Step 4: join a network (optional)

- **What:** The choices are "Not now" (the default), "Join the public relay" (VaultDB.ai's, by signing in with GitHub, Google or Microsoft, RL-45; shown when the app offers one, `RelayStatus.publicRelayUrl`), "Join the Nandi network" (the relay this build comes with, `RelayStatus.builtInUrl` from `NANDI_DEFAULT_RELAY_URL`, named by its host; not shown when the build has none) and "Join my team's network" (a `ws://` or `wss://` address, a **Join code** password field and **Paste invite**). Nothing joins and no seat is taken until the user clicks **Join** (team network) or **Continue**: that calls `join_relay` (the build's own join code for the Nandi network, which the UI never sees). The result shows "Joined <host>. Connecting…" or a calm line saying why (unreachable, code needed, wrong code, relay full with the relay's own words, version mismatch), and **Continue anyway** goes on without a network. The join code is cleared once used and never shown. A relay already joined (or set by `NANDI_RELAY_URL`) is preselected and not joined again; env-set relays say so. "Not now" leaves a relay alone, except one joined on this step, which it leaves again (`setRelayUrl('')`). Choosing the public relay asks it for its sign-in providers and shows a button for each; signing in shows the code and **Open sign-in page**, polls, and ends "Signed in: Nandi is connected to the public relay." It never holds **Continue** up: without signing in, Continue goes on with no network, and an unreachable public relay says calmly "The public relay isn't reachable right now — you can join later from Network › Connection." A public relay signed in to before is preselected.
- **Where:** `src/components/onboarding/Step3_Network.tsx`, `identity-model.ts` (`relayNeedsSave`, `skipTurnsRelayOff`, `isRelayUrl`, `relayDraftFrom`), `components/network/JoinRelayFields.tsx`, `join-model.ts`, `relay-invite.ts`, `PublicRelayJoin.tsx`; `join_relay`, `set_relay_url`, `get_relay_status`, `get_public_relay_offer`.
- **Preconditions:** For the join cases, a relay (RL-07) with `NANDI_RELAY_JOIN_CODE`.
- **Steps:**
  1. Leave "Not now" selected and click Continue.
  2. Go back and choose "Join the Nandi network" (a build with its own relay), then Continue.
  3. Choose "Join my team's network" and type "http://x".
  4. Paste an invite message into the address field.
  5. Type an unreachable `wss://` URL and click Join.
  6. Choose "Join the public relay" with the public relay unreachable (`NANDI_PUBLIC_RELAY_URL=wss://127.0.0.1:9/relay`), then Continue.
- **Expected:**
  - (1) Nothing is saved or enrolled, and step 4 shows "You can join a network later in Network." The relay's `/health` users count is unchanged.
  - (2) Nothing joins on choosing; Continue joins (one `/relay/enroll`), `relay_url` is saved to `config.json`.
  - (3) "Enter a ws:// or wss:// address."
  - (4) Address and join code fill from the invite; the code field is a password field.
  - (5) "Couldn't reach a relay at <host>…" with the detail, and "Continue anyway" goes on.
  - (6) The calm unreachable line with **Try again**, no error; Continue goes on at once and nothing is saved.
- **Edge / negative cases:**
  - Skipping never clears a relay that is already configured.
  - Signed in to the public relay on this step, then "Not now": Continue leaves it again (the token stays kept for later).
  - A save failure shows "Couldn't save the network: …", and the next Continue proceeds.
  - Regression: Continue straight from a typed address must work.
- **Automation:** Covered by: `OnboardingView.test.tsx` (the network tests, four of them for the public relay: providers, unreachable, signing in, kept), `identity-model.test.ts`, `offline_suite.rs` (`with_no_relay_onboarding_completes_with_not_now`, `an_unreachable_public_relay_never_blocks_onboarding`) | Layer: unit + api | Gap: a real relay is manual (see the network area).
- **Needs relay:** yes
- **Status:** Shipped

### ON-10 — Step 4: "Let others send requests to my Nandi"

- **What:** With a relay chosen, ticking this publishes the gateway card at the relay once, when the step is left. A note says the user must still let an agent accept requests and trust callers. On failure a warning toast says the gateway can be turned on later in Network.
- **Where:** `OnboardingView.tsx` (`publishGateway`), `make_gateway_reachable`, `identity-model.ts` (`publishesGateway`).
- **Preconditions:** A reachable relay.
- **Steps:**
  1. Choose Public.
  2. Tick the box.
  3. Click Continue.
- **Expected:** The gateway is published (it shows in Network › Sharing), and the call happens only once even if the user goes back and forward.
- **Edge / negative cases:**
  - A relay error: the toast "Your Nandi isn't visible on the network yet: …".
  - Not now with the box: never publishes.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('publishes the gateway once…') | Layer: unit | Gap: a real relay is manual.
- **Needs relay:** yes
- **Status:** Shipped

### ON-11 — Step 5: starter packs

- **What:** A compact picker of marketplace starter packs. Packs tagged `official` (the Nandi Dev Pack and Nandi Personal Backup) are listed first when their publisher signature verifies, and start ticked; the user can untick them. The user picks 1–3 (a 4th shows "Pick up to 3 now; add more from the Marketplace later."). Packs already installed are hidden. "Install n packs" shows progress per pack ("Installing n of m…"). Failures offer "Retry" or "Continue without them". The step also has Skip and Back. Load errors ("Couldn't load the starter packs") offer Retry or Skip. An empty catalog ("No new starter packs") offers Continue.
- **Where:** `src/components/marketplace/StarterPackPicker.tsx` (`MAX_STARTER_PACKS = 3`), `OnboardingView.tsx`.
- **Preconditions:** The marketplace catalog is reachable (bundled or registry).
- **Steps:**
  1. Pick 2 packs and click Install.
  2. Try to pick a 4th pack.
  3. Make one install fail (offline registry).
- **Expected:**
  - (1) Agents are written under `~/.nandi/repo/.nandi/agents/`, then Home opens with the welcome strip.
  - (2) The limit message.
  - (3) "Packs that didn't install: n. You can try again later.", with Retry and Continue without them.
- **Edge / negative cases:**
  - Skip with nothing picked: Home without a welcome.
  - Back returns to Network.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('installs the chosen packs…', 'allows at most three packs', 'skipping packs lands on Home'), `src/components/views/StarterPacksView.test.tsx` (incl. 'starts with the official packs ticked…'), Rust `marketplace::tests::verified_official_entries_come_first` | Layer: unit | Gap: real pack install is in the marketplace area.
- **Needs relay:** no
- **Status:** Shipped

### ON-12 — Finishing setup and the welcome strip

- **What:** Finishing stores `onboarding.step = done` and opens Home. If packs were installed, a one-time welcome strip lists what was installed and suggests something to try. It is dismissible and rides on the navigation, so it is not shown again.
- **Where:** `OnboardingView.tsx` (`finish`, `leave`), `src/components/dashboard/FirstRunHints.tsx` (`WelcomeStrip`), `dashboard-model.ts` (`welcomeFor`).
- **Preconditions:** Onboarding in progress.
- **Steps:**
  1. Install a pack in step 4.
  2. Read Home.
  3. Navigate away and back.
- **Expected:** (2) "You're set up. n agents installed from <packs>.", "Agents: <names>" and "Try:" suggestions. (3) The strip is gone.
- **Edge / negative cases:** If saving `done` fails, it is logged and the app still opens. The next launch reopens setup at the stored step.
- **Automation:** Covered by: `src/components/home/Home.test.tsx` ('welcomes the packs just installed…'), `dashboard-model.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### ON-15 — Step 1: connect your email (skippable)

- **What:** Setup opens on step 1 (**Email** → Name → Model → Network → Agents), titled "Connect your email". One sign-in covers which mailboxes Chief of Staff reads: a checkbox each for **Outlook** and **Gmail**, both allowed. Ticking one shows **Connect <name>**, which opens the browser sign-in; each mailbox shows its own progress and then "Connected as …". The step repeats what Nandi reads and never does. **How often should Chief of Staff check?** offers 5, 15, 30 minutes, 1 hour and 2 hours, with **30 minutes** chosen. A provider without a client ID in this build shows "Not available in this build" and can't be ticked. The button reads **Skip for now** until a mailbox is connected, then **Continue**; both save the interval.
- **Where:** `src/components/onboarding/StepEmail.tsx`, `components/email/ConnectMailbox.tsx`, `EmailReads.tsx`, `EmailSignInStatus.tsx`, `components/views/OnboardingView.tsx`, `onboarding/identity-model.ts` (`ONBOARDING_STEPS`); `OnboardingStep::Email` (`services/auth/mod.rs`); `set_email_settings`.
- **Preconditions:** A fresh `NANDI_HOME`; a build with `NANDI_MICROSOFT_CLIENT_ID` (and the Google ID and secret for Gmail).
- **Steps:** 1. Start setup on a fresh home. 2. Tick Outlook and Gmail. 3. **Connect Outlook**, sign in; **Connect Gmail**, sign in. 4. Pick 15 minutes. 5. **Continue**: the Name step reads "You're <name> (<address>) · Change" (ON-03). Finish setup. 6. Repeat on another fresh home and choose **Skip for now**: the Name step asks.
- **Expected:** Both cards say "Connected as …"; Settings › Email shows both, checking every 15 minutes. Right after setup, one "Email: N new emails" run starts for the Chief of Staff (not one interval later), and its My role holds who the user is: the "Name and title" line gets the name, the Outlook job title and the address, and the "Team and manager" line the department and manager (only the template placeholders are replaced; a line the user wrote is kept). Quitting on this step resumes on it. Skipping goes on with the interval saved and leaves the Home to-do (SH-35).
- **Edge / negative cases:** Sign-in problems explain themselves as in CP-03 (admin approval link, declined, network). There is no Back (it is the first step). The title, department and manager come from Graph `/me` and `/me/manager` (`User.Read`); if either call fails they stay empty and sign-in still succeeds. Gmail gives name and address only. A mailbox connected earlier shows as connected and its box can't be unticked.
- **Automation:** Covered by: `StepEmail.test.tsx` (both mailboxes, default 30 and skip, unavailable provider), `email-model.test.ts`, `OnboardingView.test.tsx`, `builtin_agents.rs` (`the_mailbox_owner_goes_into_my_role`), `email/profile.rs` tests, `email/tests.rs` (`outlook_pages_to_the_delta_link_and_resumes_from_it`: profile read at connect) | Layer: unit | Gap: a live sign-in during setup (manual); time zone and working hours need `MailboxSettings.Read` and are not read yet.
- **Needs relay:** no
- **Status:** Shipped (live test pending)

### ON-16 — Step 4: get your computer ready (recommended, skippable)

- **What:** After Model, setup shows "Get your computer ready" with the line "Without Docker, agents can only talk, not act." and the same checklist as Settings › This computer (ST-33), re-checked every 10 seconds. **Skip for now** goes on while Docker isn't ready; **Continue** replaces it once Docker answers. **Back** returns to Model, and Network's Back returns here. Skipping leaves the Home item (SH-37).
- **Where:** `src/components/onboarding/StepComputer.tsx`, `identity-model.ts` (`ONBOARDING_STEPS` has `computer`), Rust `auth::OnboardingStep::Computer` (stored as `"computer"`).
- **Preconditions:** A home that is not onboarded. Docker stopped (or not installed), then running.
- **Steps:**
  1. Go through Email, Name and Model.
  2. With Docker stopped, look at the step, then click Skip for now.
  3. Click Back on Network, start Docker, wait up to 10 seconds.
- **Expected:**
  - (1) `onboarding.step = "computer"` is stored; the indicator shows Computer as step 4 of 6.
  - (2) Docker running is "Not running" with Start Docker; Skip for now opens Network.
  - (3) The step is back; once Docker answers, the summary says agents can act and Continue shows.
- **Edge / negative cases:**
  - Quitting on this step resumes on it.
  - The check failing (core error) shows "Nandi couldn't check this computer: …"; setup still goes on.
- **Automation:** Covered by: `src/components/machine/MachineChecklist.test.tsx` ('the onboarding step is skippable…', 'the onboarding step continues once Docker answers'), `OnboardingView.test.tsx` ('shows six steps, Email first', the model step leading to the computer step) | Layer: unit | Gap: the real Docker start, by hand.
- **Needs relay:** no
- **Status:** Shipped

### ON-17 — Settings › Account › Run setup again

- **What:** **Run setup again** (Settings › Account, "Setup") opens the whole setup from the Email step with the saved choices filled in: the saved name and handle, the configured relay, the default model, connected mailboxes. Nothing is removed. The step reached is not recorded while re-running, so quitting or **Cancel and go back** (under the steps) leaves setup finished and changes nothing else; each step still saves its own choice when left with Continue. Finishing returns to Home as usual.
- **Where:** `src/components/settings/sections/AccountSection.tsx`, `OnboardingView.tsx` (`again`), the `{ kind: 'onboarding', again: true }` view (`MainPane.tsx`).
- **Preconditions:** A home that finished setup.
- **Steps:**
  1. Settings › Account › Run setup again.
  2. Skip Email; look at the Name step.
  3. Click Cancel and go back; quit and relaunch.
  4. Run it again, change the name, Continue, then Cancel.
- **Expected:**
  - (1) Setup opens on Email with "Running setup again: each step starts from what you saved, and nothing is removed."
  - (2) The saved name is in the field.
  - (3) Home opens; after relaunch the app opens normally (`onboarding.step` is still `done`).
  - (4) The new name is saved (Settings › Account shows it); setup isn't reopened at launch.
- **Edge / negative cases:**
  - A name step left without Continue saves nothing.
- **Automation:** Covered by: `OnboardingView.test.tsx` ('goes back from Name to Email', 'runs setup again with the saved name filled in, recording no step, and cancels', 'saves the name again without touching the step'), `SettingsView.test.tsx` ('Account › Run setup again reopens the whole setup') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### ON-13 — Starter packs page (after setup)

- **Duplicate:** tested under LB-20. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ON-14 — Upgrades from pre-identity installs

- **What:** An install from before local identity skipped onboarding when it had a name, and offered to re-add its old GitHub agents repo.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

## 1.3 Main window layout and navigation

### SH-04 — Title bar and window controls

- **What:** The title bar is custom per platform:
  - macOS: traffic lights on the left.
  - Windows: Minimize, Maximize/Restore and Close on the right.
  - Linux: a toolbar under the native decorations.
  - Browser: the Linux layout.

  The bar is a drag region. Its left side has a sidebar toggle. The center has the bull and "Nandi". The right side has the Update button, the account menu and the work tray toggle. Settings is in the sidebar and on Cmd/Ctrl+, (no title-bar gear).

- **Where:** `src/components/titlebar/TitleBar.tsx`, `TitleBarContent.tsx`, `WindowsWindowControls.tsx`, `MacOSWindowControls.tsx`, `LinuxTitleBar.tsx`, `src/lib/commands/window-commands.ts`; decorations in `src-tauri/tauri.{windows,macos,linux}.conf.json`; `src/hooks/useSquareCornersEffect.ts` (square corners when fullscreen).
- **Preconditions:** A desktop build.
- **Steps:**
  1. On Windows, click Minimize, Maximize, Restore and Close.
  2. Drag the window by the bar.
  3. Press Ctrl+,.
- **Expected:**
  - (1) The window acts on each click. Close hides the window (SH-03).
  - (2) The window moves.
  - (3) Settings opens on the last section used, else Models.
- **Edge / negative cases:**
  - A window command fails: the toast "toast.error.windowCloseFailed" and similar.
  - macOS green button: fullscreen versus maximize with Alt.
  - Windows window-control aria labels are hard-coded English.
- **Automation:** Covered by: `src/App.test.tsx` ('renders title bar with traffic light buttons'), `src/hooks/use-platform.test.ts`, `src/lib/commands/commands.test.ts` | Layer: ui-e2e | Gap: real window ops are manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-05 — Title-bar bull (status at a glance)

- **What:** The bull next to the title changes pose:
  - `wait` while any approval is pending.
  - `carry` while agents run.
  - `done` for about 1.5 s after an approval is granted anywhere, or 2.5 s after the hidden phrase.
  - `rest` otherwise.

  It uses the mascot's name. The window and taskbar icon follow the palette (ST-28).

- **Where:** `TitleBarContent.tsx` (`TitleBarTitle`), `src/hooks/useMascot.ts` (`useNandiState`), `src/lib/mascot.ts` (`nandiState`), `src/assets/brand/NandiBull`.
- **Preconditions:** An agent with a tool that needs approval.
- **Steps:**
  1. Start a run that waits for approval.
  2. Approve it.
  3. Let a run continue.
- **Expected:** (1) The `wait` pose. (2) A short `done`. (3) The `carry` pose while it runs, then `rest`.
- **Edge / negative cases:** Several approvals at once: still `wait` until all are decided.
- **Automation:** Covered by: `src/lib/mascot.test.ts` | Layer: unit | Gap: the pose rendering is manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-06 — Account menu

- **What:** A person icon in the title bar opens a menu with two items: Account (Settings › Account) and Backup (Settings › Backup). There is no sign-in, so there is no Log out and no Sync now.
- **Where:** `src/components/titlebar/UserMenu.tsx`.
- **Preconditions:** None.
- **Steps:**
  1. Click the person icon.
  2. Choose Account, then reopen it and choose Backup.
- **Expected:** Each item opens that Settings section.
- **Edge / negative cases:** Keyboard: open with Enter and move with the arrows.
- **Automation:** Covered by: `AuthGate.test.tsx` ('opens Settings › Account', 'opens Settings › Backup') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-07 — Panes and panel toggles

- **What:** There are three resizable panes:
  - The sidebar: default 20%, 14–32%.
  - The current view: at least 35%.
  - The work tray (SH-36): default 18%, 14–28%.

  Cmd/Ctrl+1, the title-bar button and View › Toggle Left Sidebar show or hide the sidebar. Cmd/Ctrl+2, the title-bar button on the right, the tray's own close button and View › Toggle Work Tray show or hide the tray. Both are saved in this machine's preferences (`preferences.json`: `sidebarVisible`, `workTrayVisible`), never in the repo. Until the user shows or hides the tray, it follows the window: hidden below 1000px wide, shown above.

- **Where:** `src/components/layout/AppShell.tsx`, `layout-model.ts` (`paneLayout`, `NARROW_WINDOW_PX`), `LeftSideBar.tsx`, `MainPane.tsx`, `src/components/tray/WorkTray.tsx`, `src/store/ui-store.ts`, `src/hooks/use-pane-preferences.ts`, `src/services/preferences.ts` (`savePaneLayout`); `AppPreferences` in `src-tauri/src/types.rs`.
- **Preconditions:** None.
- **Steps:**
  1. Press Ctrl+1 twice.
  2. Press Ctrl+2 twice.
  3. Drag the handles between the panes.
- **Expected:** (1) The sidebar hides, then shows. (2) The tray hides, then shows. (3) The pane widths change within their limits.
- **Edge / negative cases:**
  - Relaunch: each pane is as the user left it.
  - A narrow window (under 1000px) on first launch: the tray starts hidden; widening shows it until the user picks.
  - Saving fails: the panes still toggle; no toast.
- **Automation:** Covered by: `src/store/ui-store.test.ts`, `src/components/layout/AppShell.test.tsx`, `layout-model.test.ts`, `src/hooks/use-pane-preferences.test.tsx`, `services/preferences.rs` (`theme_and_language_live_in_settings_yaml`) | Layer: unit | Gap: resizing is manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-08 — Sidebar: the eight pages

- **What:** The sidebar is a tree with eight top items: Home, Ask, Notes, Agents, Automations, Library, Network, Settings. Home has two children: Inbox (with the same count as Home) and All activity. Home's count is its own button and opens the Inbox page; the rest of the row opens Home. Each opens its page. The current page is highlighted, and a parent stays bold while a child is open. Views without their own node (a run, a request, a member profile) light their page.
- **Where:** `src/components/layout/Sidebar.tsx`, `nav-tree.ts` (`buildNavTree`, `activePath`), `src/store/navigation-store.ts` (`NAV_SECTIONS`, `SECTION_VIEWS`, `navSection`).
- **Preconditions:** None.
- **Steps:**
  1. Click each top item.
  2. Open a run detail from Agents.
- **Expected:** (1) Each page opens with `aria-current="page"`. (2) Agents stays highlighted.
- **Edge / negative cases:** Onboarding and other views with no section light nothing.
- **Automation:** Covered by: `src/components/layout/Sidebar.test.tsx`, `nav-tree.test.ts`, `src/store/navigation-store.test.ts`, `AppShell.test.tsx` ('has the seven pages…') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-09 — Sidebar: live children

- **What:** Three items have children:
  - Ask: "New Chat" (with a + icon), the 5 newest chats, then "All chats (n)".
  - Agents: Chief of Staff first (built in, always shown), then up to 8 other agents, busiest first (waiting, running, error, idle, then by name), each with a status dot (amber waiting, pulsing blue running, red error, green idle). Then "All agents (n)" when there are more than 8 others.
  - Automations: Workflows and Triggers.

  Clicking a name opens it and expands its branch. The chevron only expands or collapses.

- **Where:** `nav-tree.ts` (`chatNodes`, `agentNodes`, `LIST_LIMIT = 8`, `CHAT_LIMIT = 5`), `Sidebar.tsx`.
- **Preconditions:** Several agents and chats.
- **Steps:**
  1. Expand Agents with 10 agents, one of them running.
  2. Click "New Chat" under Ask.
- **Expected:**
  - (1) The running agent is listed first with a pulsing dot, and "All agents (10)" opens the Agents list.
  - (2) An empty draft chat. It is created only with its first message.
- **Edge / negative cases:**
  - A chat without a title shows "New chat".
  - An agent with an empty name shows its id.
- **Automation:** Covered by: `Sidebar.test.tsx` ('opens a page's branch, with its live list', 'has New chat and All chats under Ask'), `nav-tree.test.ts`, `AppShell.test.tsx` ('opens a new chat…') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-10 — Sidebar: keyboard and remembered branches

- **What:** The sidebar supports the keyboard:
  - Up and Down move through the visible items (with wrap-around).
  - Home and End jump to the first and last item.
  - Right expands a branch or enters it.
  - Left collapses a branch or goes to the parent.
  - Enter opens the item.

  Expanded branches are remembered in localStorage `nav-tree-expanded`. Branches above the current page open by themselves.

- **Where:** `Sidebar.tsx` (`onKeyDown`), `src/store/nav-tree-store.ts`, `nav-tree.ts` (`visibleIds`, `parentOf`).
- **Preconditions:** None.
- **Steps:**
  1. Focus Home and press Down three times.
  2. On Agents, press Right, then Left.
  3. Expand Automations and relaunch.
- **Expected:** (1) Focus moves. (2) The branch expands, then collapses. (3) Automations is still expanded.
- **Edge / negative cases:** Blocked localStorage: the tree starts collapsed with no error.
- **Automation:** Covered by: `Sidebar.test.tsx` ('moves between items with the arrow keys', 'opens and closes branches with Right and Left', 'opens the branches above the current page by itself, and remembers them') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-11 — Sidebar badges and dots

- **What:**
  - Home shows the Inbox count, which leaves failures out. The aria label is "Home, n waiting".
  - Settings shows an amber dot when any section needs attention (ST-01).
  - Network shows a health dot: green when reachable, amber when connecting or not listed, red when the relay refuses, and none without a network.
- **Where:** `nav-tree.ts`, `Sidebar.tsx` (`NetworkDot`), `useInbox`, `src/hooks/useSettingsAttention.ts`, `useNetworkHealth`.
- **Preconditions:** A pending approval, no model, or a relay.
- **Steps:**
  1. Have one pending approval.
  2. Remove all keys and the trial.
  3. Connect to a relay.
- **Expected:** (1) A "1" badge on Home. (2) A dot on Settings. (3) A green or amber dot on Network.
- **Edge / negative cases:** A failed run is not counted in the badge.
- **Automation:** Covered by: `Sidebar.test.tsx` ('counts the Inbox on Home…', 'shows the connection as a dot…', 'is amber while connected…', 'shows no dot without a network') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-12 — Navigation history and view validation

- **What:** Navigation keeps a back stack of up to 50 views. Navigating to the same view adds nothing. Untrusted view JSON from the palette or the companion is checked by `parseView` against the current view kinds; anything else (including retired kinds such as `dashboard`, `team` or `egress`) is rejected. The Rust palette emits only current kinds.

- **Where:** `src/store/navigation-store.ts` (`MAX_HISTORY`, `goBack`, `parseView`), `MainPane.tsx`; `src-tauri/src/services/palette.rs`.
- **Preconditions:** None.
- **Steps:**
  1. In the palette, search "egress" and open the result.
  2. Search "team" and open the result.
  3. Pass `{kind:'nope'}` and `{kind:'dashboard'}` to `parseView`.
- **Expected:** (1) Settings › Internet access (`{kind:'settings', section:'internet'}`). (2) Network › People. (3) Both rejected (`null`).
- **Edge / negative cases:**
  - More than 50 moves: the oldest are dropped.
  - Back with an empty history does nothing.
- **Automation:** Covered by: `src/store/navigation-store.test.ts` | Layer: unit | Gap: there is no visible Back button or shortcut, so `goBack` is only used by views.
- **Needs relay:** no
- **Status:** Shipped

### SH-13 — Leave guard ("Discard changes?")

- **What:** A view with unsaved edits sets `leaveGuarded`. Today that is the workflow detail and trigger detail editors. Any move away, including Back, is held and asks "Leave without saving?" with Stay and "Leave and discard". A move to the same view passes.
- **Where:** `src/components/layout/LeaveGuardDialog.tsx`, `navigation-store.ts` (`setLeaveGuarded`, `confirmLeave`, `cancelLeave`), callers `WorkflowDetailView.tsx` and `TriggerDetailView.tsx`.
- **Preconditions:** A workflow or trigger.
- **Steps:**
  1. Edit a trigger without saving.
  2. Click Home in the sidebar.
  3. Choose Stay, then repeat and choose Leave and discard.
- **Expected:** (2) The dialog "Leave without saving?" opens. (3) Stay keeps the edits and the page. Leave and discard opens Home and drops the edits.
- **Edge / negative cases:**
  - Esc acts as Stay.
  - Palette navigation and companion hand-offs are held too.
  - Closing the window is not guarded.
- **Automation:** Covered by: `src/store/leave-guard.test.ts` | Layer: unit | Gap: no UI test of the dialog itself.
- **Needs relay:** no
- **Status:** Shipped

### SH-14 — Opening views from the companion window

- **Duplicate:** tested under AI-18. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## 1.4 Home

### SH-15 — Home page

- **What:** Home is one scrolling page with:
  - "Ask Nandi…": typed text is sent to Ask, and empty text opens Ask.
  - A network line: "Connected · n people online", or "Not connected." with "Join a network".
  - The email to-do and the Chief of Staff's digest.
  - The Inbox.
  - Recent changes.

  Running and finished work are in the work tray (SH-36) on every page, and All activity is in the sidebar. When nothing waits, it says "Nothing needs you."

- **Where:** `src/components/home/HomeView.tsx`, `HomeRows.tsx`, `home-model.ts` (`buildHomeFeed`, `RECENT_WINDOW_MS`, `MAX_RECENT`, `STALE_AFTER_MS`), `src/hooks/useHome.ts`, `src/components/inbox/Inbox.tsx`.
- **Preconditions:** Some runs, delegations or approvals.
- **Steps:**
  1. Type "summarize today" in Ask Nandi and press Enter.
  2. Start a long workflow.
- **Expected:**
  - (1) Ask opens with the message sent.
  - (2) Home doesn't list it; the work tray shows it under Working.
- **Edge / negative cases:**
  - An approval decided elsewhere disappears on the next event.
  - A step waiting for approval is in the Inbox, not under Working.
- **Automation:** Covered by: `src/components/home/Home.test.tsx`, `home-model.test.ts`, `activity-model.test.ts`, `AppShell.test.tsx` ("Home's Ask Nandi sends the text to Studio") | Layer: unit | Gap: the Inbox decisions belong to the inbox/approvals area.
- **Needs relay:** no
- **Status:** Shipped

### SH-16 — Home on a fresh install (Get started)

- **What:** With no agents, runs or waiting items, Home shows Get started instead of empty sections. Its cards are "Install a starter pack" (button "Browse packs", until there is an agent), "Create an agent by describing it" (button "Open Ask") and "Join a network" (button "Set up network", until a relay is set). It also shows the no-model notice. "Hide Get started" dismisses it (localStorage).
- **Where:** `src/components/dashboard/FirstRunHints.tsx` (`GetStarted`), `dashboard-model.ts` (`isFreshDashboard`, `getStartedCards`), `src/lib/dismissals.ts`.
- **Preconditions:** A fresh home.
- **Steps:**
  1. Finish onboarding with no packs.
  2. Click each card's button.
  3. Hide Get started.
- **Expected:**
  - (1) Three cards.
  - (2) Starter packs, Ask and Network › Connection open.
  - (3) It stays hidden after a relaunch.
- **Edge / negative cases:** Cleared localStorage brings it back.
- **Automation:** Covered by: `Home.test.tsx` ('shows Get started only', 'sends each Get started card to its page', 'offers to connect a model…'), `dashboard-model.test.ts`, `src/lib/dismissals.test.ts`, `AppShell.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-17 — Backup nudge

- **Duplicate:** tested under DT-32. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### SH-18 — Recent results: rate and retry

- **What:** A finished delegation can be rated once with 1–5 stars, then shows "Rated n/5". A failed request offers Retry, which runs at once. A failed workflow run offers Retry after a confirmation ("Run \"<title>\" again?", with Cancel and "Run again"), because every step runs again.
- **Where:** `src/components/home/HomeRows.tsx` (`RetryButton`, rating), `useRetryRequest`, work action `rate`.
- **Preconditions:** A completed delegation and a failed workflow run.
- **Steps:**
  1. Rate a delegation 4 stars.
  2. Click Retry on a failed workflow and choose Run again.
- **Expected:** (1) "Rated 4/5" (the provider's `userRatingAvg` updates). (2) The toast "Started again", and a new run appears under Running.
- **Edge / negative cases:** Cancelling the workflow confirmation does nothing.
- **Automation:** Covered by: `Home.test.tsx` ('lists recent results; a delegation can be rated…', 'asks before rerunning a failed workflow', 'offers a recent failure with Dismiss') | Layer: unit | Gap: ratings over a real relay are in the network area.
- **Needs relay:** no
- **Status:** Shipped

### SH-19 — All activity

- **What:** Home › All activity lists runs and delegations (both ways) in one table: When, Request, Source, Agent or member, Took, State. It has search and filters for Date, Status, Source, Direction, Agent and Member, plus "Clear filters". "Export CSV" exports exactly the filtered rows. "Load more" pages the list. Filters live in the view, so they survive Back.
- **Where:** `src/components/home/AllActivityView.tsx`, `activity-model.ts`; view `{kind:'home-activity', filters}`; `GET /api/runs`.
- **Preconditions:** Activity exists.
- **Steps:**
  1. Filter Status = failed.
  2. Export CSV.
  3. Open a row, then go Back.
- **Expected:** (2) A CSV with only the failed rows. (3) The filter is still applied.
- **Edge / negative cases:**
  - No results with filters on: "No activity matches these filters." with "Clear filters".
  - No activity at all: "Nothing has run yet." with "Ask Nandi".
- **Automation:** Covered by: `Home.test.tsx` ('lists runs and delegations together…', 'keeps filters in the view…'), `activity-model.test.ts` | Layer: unit | Gap: the CSV content is not asserted in the UI.
- **Needs relay:** no
- **Status:** Shipped

### SH-20 — "No model connected" notice

- **What:** While no model can be called (no key for any provider, no Claude Code sign-in, no Ollama default, and no trial), a notice says "No model connected. Nandi can't run agents yet." When the trial is used up it says "Your free tokens are used up. Connect your own model to keep going."; paused: "The free trial is paused for today. ..."; unreachable: "Can't reach the free trial (offline?). Nandi's built-in help still works." "Connect a model" opens Settings › Models & runtimes; **How?** opens the help guide on "Connect a model" (SH-41). It shows on Ask, on the agent message box, and on a fresh Home.
- **Where:** `src/components/layout/LlmKeyNotice.tsx`, `MainPane.tsx`, `src/components/agent-console/AgentComposer.tsx`, `HomeView.tsx`, `useDefaultModel` (`effectiveModel === null`).
- **Preconditions:** No model available.
- **Steps:**
  1. Remove all keys and turn the trial off.
  2. Open Ask.
  3. Add a key.
- **Expected:** (2) The notice. (3) The notice disappears without a reload.
- **Edge / negative cases:** While the status is loading or errors, it stays hidden.
- **Automation:** Covered by: `AppShell.test.tsx` ('asks for a key on agent views…', 'stays hidden when another provider has a key'), `Home.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-36 — Work tray (Needs you · Working · Done)

- **What:** A right-hand pane on every page shows three groups in small type, each row one line with a coloured dot: **Needs you** (the Inbox, amber), **Working** (Home's Running, blue, pulsing) and **Done** (Home's Recent: green, red when it failed, grey when cancelled; finished rows read quieter). Each group shows its count and a few rows (5, 6, 8), and an **All** link: Needs you opens the Inbox page, Working opens All activity filtered to running, Done opens All activity. A row opens its own page; a tool approval has ✓ and ✗ to decide it in place. The open page's row is highlighted.
- **Where:** `src/components/tray/WorkTray.tsx`, `tray-model.ts` (`buildTray`, `inboxTrayRow`, `homeTrayRow`), `src/components/inbox/InboxView.tsx`; data from `useHomeFeed`.
- **Preconditions:** None. With a relay, held requests, assignments and trust requests also show under Needs you.
- **Steps:**
  1. Start an agent run that asks for a tool approval, from any page other than Home.
  2. Click ✓ on its tray row.
  3. Let a workflow run finish; click its row under Done.
  4. Click **All** on each group.
- **Expected:** (1) The approval appears under Needs you, the run under Working, without leaving the page. (2) The run goes on; the row leaves Needs you. (3) The run's page opens. (4) The Inbox page, All activity (running) and All activity open.
- **Edge / negative cases:**
  - An empty group says so in one muted line ("Nothing running").
  - No relay: the tray works the same; person-to-person rows just never appear.
- **Automation:** Covered by: `src/components/tray/tray-model.test.ts`, `WorkTray.test.tsx` (empty groups, ✓ and ✗ decide, rows and All open, the close button), `AppShell.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-37 — Home: "Agents can talk, but need Docker to act"

- **What:** Until Docker answers, Home shows a calm line "Agents can talk, but need Docker to act." with **Set up**, which opens Settings › This computer. It disappears once Docker runs. Nothing else on Home waits on it.
- **Where:** `src/components/machine/MachineHomeItem.tsx` in `HomeView.tsx`; `useMachineReport` (`check_machine`, `GET /api/machine/readiness`).
- **Preconditions:** Docker stopped or not installed.
- **Steps:**
  1. Open Home.
  2. Click Set up.
  3. Start Docker and come back to Home.
- **Expected:**
  - (1) The line shows, without a warning colour.
  - (2) Settings › This computer opens.
  - (3) The line is gone (within 30 seconds, or on reload).
- **Edge / negative cases:**
  - While the check runs, or if it fails, nothing shows.
- **Automation:** Covered by: `MachineChecklist.test.tsx` ('Home shows a calm Set up line until Docker is ready', 'Home shows nothing once Docker is ready') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-38 — Agent page: "Agents need Docker to act. Set up"

- **What:** When a run's tools can't start, the core still sends `agent://warning` ("Tool execution unavailable: …") and the agent answers without tools. The agent page now shows a calm line "Agents need Docker to act." with **How?** (the help guide's Docker topic, SH-41), **Set up** (opens Settings › This computer) and Dismiss, instead of the warning text. Other agent warnings show as before.
- **Where:** `src/components/agent-console/ContainerControls.tsx` (`AgentWarningBanner`), `src/components/machine/DockerSetupPrompt.tsx`, `machine-model.ts` (`isToolsUnavailable`).
- **Preconditions:** Docker stopped; an agent with a model.
- **Steps:**
  1. Send the agent a message.
  2. Click Set up.
  3. Go back, click Dismiss.
- **Expected:**
  - (1) The agent answers in text; the line shows above the chat, not "Tool execution unavailable".
  - (2) Settings › This computer opens.
  - (3) The line goes away until the next warning.
- **Edge / negative cases:**
  - A connector warning (not tools) shows its own text in the warning banner.
- **Automation:** Covered by: `MachineChecklist.test.tsx` ('"Tool execution unavailable" becomes…', 'other warnings show as they are'), Rust `chat::tests` (the warning event is unchanged) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## 1.5 Command palette, commands and shortcuts

### SH-21 — Command palette: open, search, open results

- **What:** Cmd/Ctrl+K toggles the palette, even from inside inputs. Shift and Alt variants are ignored. Typing searches agents, workflows, runs, packages, tools and actions on the server (debounced 150 ms) plus the local commands. Results are grouped (Ask, Actions, Agents, Workflows, Runs, Packages, Tools) and ranked: exact, prefix, word prefix, contains, then description. A run ranks below its agent. Keys: Up, Down, Home, End, Enter, Esc. Choosing an entity opens its view.
- **Where:** `src/components/palette/CommandPalette.tsx`, `PaletteResult.tsx`, `palette-items.ts`, `src/hooks/usePalette.ts`; `search_palette` (`src-tauri/src/commands/palette.rs`, `services/palette.rs`); `GET /api/palette/search?q=`.
- **Preconditions:** Some agents and workflows.
- **Steps:**
  1. Press Ctrl+K and type the start of an agent's name.
  2. Press Down, then Enter.
  3. Press Esc.
- **Expected:** (1) The agent is in the Agents group, first. (2) Its page opens and the palette closes. (3) The palette closes.
- **Edge / negative cases:**
  - No match: "No results found."
  - Each opening starts empty.
  - Server error: the local commands still list.
- **Automation:** Covered by: `src/components/palette/CommandPalette.test.tsx`, `palette-items.test.ts`, Rust tests in `services/palette.rs` and `src-tauri/src/http/palette.rs` (`searches_actions`, `executes_ui_actions`, `errors_and_bad_bodies_are_400`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-22 — Palette: empty state, recents, and "Ask Nandi"

- **What:** An empty query shows Recent (the last 6 items opened, per browser profile, in localStorage `nandi.palette.recents`) and the server's suggestions. Typed text always gets a first row, "Ask Nandi: <text>", which sends the text to Ask.
- **Where:** `recents.ts` (`MAX_RECENTS = 6`), `palette-items.ts` (`askItem`), `usePalette.ts` (`sendToStudio`).
- **Preconditions:** None.
- **Steps:**
  1. Open two agents through the palette.
  2. Reopen the palette.
  3. Type "draft a report agent" and press Enter.
- **Expected:** (2) Both agents are under Recent. (3) Ask opens with that message sent.
- **Edge / negative cases:** Blocked storage: no recents and no error.
- **Automation:** Covered by: `CommandPalette.test.tsx` ('shows suggestions for an empty query', 'offers "Ask Nandi: <text>" first…') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-23 — Palette command mode (`>`) and local commands

- **What:** A leading `>` switches to actions only, with the "Commands" chip. The local registry adds:
  - Show Left Sidebar / Hide Left Sidebar (⌘1).
  - Show Right Sidebar / Hide Right Sidebar (⌘2).
  - Open Settings (⌘,).
  - "Settings: <section>" for each section (Models, Account, Cloud credentials, Internet access, Backup, Sources, Logs, Appearance).
  - Close Window, Minimize Window, Toggle Maximize, Enter Fullscreen / Exit Fullscreen (desktop only).

  Commands that are not available are hidden (for example, Show sidebar while it is shown).

- **Where:** `src/lib/commands/` (`registry.ts`, `navigation-commands.ts`, `window-commands.ts`, `index.ts`), `src/hooks/use-command-context.ts`, `docs/developer/command-system.md`.
- **Preconditions:** None.
- **Steps:**
  1. Type ">".
  2. Type ">settings logs" and press Enter.
  3. Type ">hide left".
- **Expected:** (1) Only actions are listed. (2) Settings › Logs opens. (3) "Hide Left Sidebar" is listed, and Enter hides the sidebar.
- **Edge / negative cases:**
  - In the browser, the window commands are absent.
  - A failing command shows an error toast.
- **Automation:** Covered by: `CommandPalette.test.tsx` ('shows only actions in > mode, including local commands', 'runs the highlighted action on Enter'), `src/lib/commands/commands.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-24 — Palette server actions

- **What:** The built-in actions run through the core:
  - "Open view" for the listed pages: Home, Ask, Agents, All activity, Workflows, Triggers, Library, Tools, Shared with me, People, Connection, Trust graph, Shared by me, Internet access, Logs.
  - "Pause <agent>" and "Resume <agent>" (per agent).
  - "Pause all agents" and "Resume all agents" (restarts the ones the palette paused).
  - "Install <agent>" (per marketplace entry).
  - "Run <workflow>" (per workflow).
  - "New chat in Ask".

  The outcome may show a success toast and navigate.

- **Where:** `services/palette.rs` (`actions()`, `VIEWS`, `execute_palette_action`), `execute_palette_action` command, `POST /api/palette/actions/{action_id}` (body `{args}`).
- **Preconditions:** Docker for pause and resume. A workflow.
- **Steps:**
  1. Type ">run" and choose "Run <workflow name>".
  2. Type ">pause all".
  3. Type ">resume all".
- **Expected:** (1) The toast and the workflow's page. (2) The agents' containers stop. (3) They restart.
- **Edge / negative cases:**
  - An unknown action id: "Unknown palette action '<id>'".
  - Open view with a bad kind (over the API): "'view' needs a string 'kind'".
  - Partial failures in Pause all are listed.
- **Automation:** Covered by: Rust `services/palette.rs` tests, `src-tauri/src/http/palette.rs` tests, `CommandPalette.test.tsx` ('navigates to the outcome view…') | Layer: api | Gap: pause and resume with real Docker is manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-25 — Global keyboard shortcuts

- **What:** Cmd/Ctrl+K opens the palette. Cmd/Ctrl+, opens Settings. Cmd/Ctrl+1 toggles the sidebar. Cmd/Ctrl+2 toggles the work tray. On desktop, Cmd/Ctrl+W closes the window, Cmd/Ctrl+M minimizes it and F11 enters or leaves fullscreen: these run the palette's window commands, so the keys the palette shows are the keys that work. Each calls `preventDefault`. The companion hotkey Ctrl/Cmd+Shift+Space is a global OS shortcut (companion area).
- **Where:** `src/hooks/use-keyboard-shortcuts.ts`, `src/lib/commands/window-commands.ts`, `useMainWindowEventListeners.ts`, `docs/developer/keyboard-shortcuts.md`.
- **Preconditions:** None.
- **Steps:**
  1. Press each shortcut on any page and inside a text field.
- **Expected:** Each one acts as described, including inside inputs.
- **Edge / negative cases:**
  - Cmd/Ctrl+Shift+W and Cmd/Ctrl+Shift+M are left alone.
  - In the web client the window keys do nothing.
- **Automation:** Covered by: `src/hooks/use-keyboard-shortcuts.test.ts` (window keys and F11), `CommandPalette.test.tsx` (Cmd/Ctrl+K), `src/store/ui-store.test.ts` | Layer: unit | Gap: the real window close, minimize and fullscreen are manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-26 — Native application menu

- **What:** On desktop the menu is built from JavaScript, in the current language, and rebuilt when the language changes:
  - App menu: About Nandi (a native dialog with the version), Check for Updates…, Preferences… (Cmd/Ctrl+,), Hide, Hide Others, Show All, Quit.
  - View menu: Toggle Left Sidebar (Cmd/Ctrl+1).
- **Where:** `src/lib/menu.ts` (`buildAppMenu`, `setupMenuLanguageListener`), `docs/developer/menus.md`.
- **Preconditions:** A desktop build. The menu bar is visible on macOS. The Windows and Linux windows have no native decorations or menu bar, so check there whether it shows at all.
- **Steps:**
  1. On macOS, choose Nandi › Check for Updates….
  2. Switch the language to Français.
  3. Open the menu again.
  4. Choose Nandi › À propos de Nandi.
- **Expected:**
  - (1) A toast: "Up to date", "Update available" or "check failed".
  - (3) The menu labels are in French.
  - (4) A native dialog titled "À propos de Nandi" that says "Version <x.y.z>".
- **Edge / negative cases:** On Windows and Linux there may be no menu bar to open.
- **Automation:** Covered by: none | Layer: manual | Gap: no tests.
- **Needs relay:** no
- **Status:** Partial (visibility on Windows and Linux unverified)

### SH-27 — Context menu guard

- **What:** In the desktop app the browser's right-click menu (with its OS "Share") is suppressed. Editable fields get a native Undo/Cut/Copy/Paste menu. Selected text gets a Copy menu. Elsewhere there is nothing. In dev builds, Shift+right-click keeps the browser menu.
- **Where:** `src/lib/context-menu.ts` (`installContextMenuGuard`, `showTextInputContextMenu`, `showEditContextMenu`), `App.tsx`.
- **Preconditions:** A desktop build.
- **Steps:**
  1. Right-click an input.
  2. Select text and right-click.
  3. Right-click empty space.
- **Expected:** (1) The text-input menu. (2) The edit menu. (3) No menu.
- **Edge / negative cases:** A component that handles `contextmenu` itself (for example the companion bison) is left alone.
- **Automation:** Covered by: `src/lib/context-menu.test.ts` | Layer: unit | Gap: the native menus are manual.
- **Needs relay:** no
- **Status:** Shipped

## 1.6 Notifications, updates, errors and logs

### SH-28 — Toasts and native notifications

- **What:** In-app toasts (sonner, bottom-right) follow the light/dark mode. `notify()` can send a native OS notification instead (desktop). If that fails, it falls back to an error toast. The app sends a native notification when a workflow manual step becomes ready. The companion sends its own approval and question notifications (companion area).
- **Where:** `src/lib/notifications.ts`, `src/components/layout/MainWindow.tsx` (`Toaster`), `src-tauri/src/commands/notifications.rs` (`send_native_notification`), `src/lib/agent-sync.ts` (`workflow://manual_step_ready`), `src/lib/toast-error.ts`, `src/lib/friendly-error.ts`, `docs/developer/notifications.md`.
- **Preconditions:** OS notifications allowed, for the native ones.
- **Steps:**
  1. Run a workflow with a manual step while the app is in the background.
  2. Trigger any save error.
- **Expected:** (1) An OS notification titled with the workflow, with the instructions as body. Log "Sending native notification: …". (2) A friendly error toast.
- **Edge / negative cases:**
  - Notifications denied by the OS: the error is logged and a fallback toast shows.
  - Mobile: an error ("not supported").
- **Automation:** Covered by: `src/lib/friendly-error.test.ts` | Layer: manual | Gap: native delivery is manual. No test for `notify`.
- **Needs relay:** no
- **Status:** Shipped

### SH-29 — In-app updates

- **Duplicate:** tested under PL-08. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### SH-30 — Error boundary and crash file

- **What:** An uncaught render error replaces the app with a screen in the current language: "Something went wrong", "Reload Application" and "Try Again". In dev builds it also shows the stack. It saves `crash-<timestamp>.json` (state, error, stack, component stack, user agent, URL) to the recovery folder.
- **Where:** `src/components/ErrorBoundary.tsx`, `src/lib/recovery.ts` (`saveCrashState`), `save_emergency_data`.
- **Preconditions:** A way to throw in a component (dev).
- **Steps:**
  1. Force a render error.
  2. Look in `<app data dir>/recovery/`.
  3. Click Reload Application.
- **Expected:** (2) A new `crash-*.json`. (3) The app reloads.
- **Edge / negative cases:**
  - Saving fails: the error is only logged.
  - In Arabic the screen is translated and the dev details align to the start (RTL).
- **Automation:** Covered by: none | Layer: unit | Gap: no ErrorBoundary test.
- **Needs relay:** no
- **Status:** Shipped

### SH-31 — Emergency data files (recovery)

- **What:** Small JSON files are saved, loaded and cleaned up under `<app data dir>/recovery/`. Filenames are 1–100 characters of letters, digits, `-`, `_` or `.`. Content is capped at 10 MB. Files older than 7 days are removed at startup. Today only the crash file uses it.
- **Where:** `src-tauri/src/commands/recovery.rs` (`save_emergency_data`, `load_emergency_data`, `cleanup_old_recovery_files`), `services/recovery.rs`, `src-tauri/src/types.rs` (`validate_filename`, `MAX_RECOVERY_DATA_BYTES`, `RecoveryError`); `GET` and `PUT /api/emergency-data/{filename}`, `POST /api/emergency-data/cleanup`.
- **Preconditions:** None.
- **Steps:**
  1. Call `PUT /api/emergency-data/draft-1` with `{"a":1}`.
  2. Call `GET` on the same path.
  3. Call `PUT /api/emergency-data/..%2Fx`.
  4. Make a file 8 days old, relaunch or POST cleanup.
- **Expected:**
  - (2) `{"a":1}`.
  - (3) `ValidationError`.
  - (4) The file is removed, and the cleanup count is returned.
- **Edge / negative cases:**
  - A missing file: `FileNotFound` (the UI treats it as null).
  - More than 10 MB: rejected.
- **Automation:** Covered by: none | Layer: api | Gap: no Rust tests for `services/recovery.rs` or `validate_filename`.
- **Needs relay:** no
- **Status:** Shipped

### SH-32 — Logging

- **What:** Rust logs through `tauri-plugin-log`. Debug builds log at Debug level, release builds at Info. Logs always go to stdout. On macOS they also go to the app log directory. On macOS and Windows they also go to the webview console (not Linux). The frontend `logger` writes to the devtools console. The headless server prints the access link at start (with the mascot line "<name> is up. n agents ready.").
- **Where:** `src-tauri/src/lib.rs` (log plugin targets), `src/lib/logger.ts`, `src-tauri/src/headless.rs`, `docs/developer/logging.md`.
- **Preconditions:** None.
- **Steps:**
  1. Run a release build on Windows from a terminal.
  2. On macOS, open `~/Library/Logs/<identifier>/`.
  3. Run headless.
- **Expected:** (1) Info logs on stdout. (2) A log file. (3) "Nandi is up. n agents ready. Running headless. Open: <url>".
- **Edge / negative cases:** Windows and Linux release builds write no log file (as `logging.md` says).
- **Automation:** Covered by: none | Layer: manual | Gap: manual.
- **Needs relay:** no
- **Status:** Partial (log file on macOS only)

## 1.7 Mascot

### SH-33 — Mascot name from the CLI

- **What:** `nandi name` prints the bull's name. `nandi rename <NAME>` renames it. `nandi rename --reset` restores "Nandi". These share `config.json` `mascot_name` with Settings.
- **Where:** `src-tauri/src/cli_mascot.rs` (`parse`, `run`, `output`), `services/mascot.rs` (`validate_name`, `read`, `write`, `MAX_NAME_CHARS = 24`).
- **Preconditions:** The CLI binary.
- **Steps:**
  1. Run `nandi rename Gauri`.
  2. Run `nandi name`.
  3. Run `nandi rename --reset`.
  4. Run `nandi name extra`.
- **Expected:**
  - (1) "Nandi is now called Gauri."
  - (2) "Gauri".
  - (3) The name is back to Nandi.
  - (4) "name takes no arguments".
- **Edge / negative cases:**
  - `nandi rename` with no name: "rename needs a name, or --reset".
  - A name over 24 characters, or with symbols: rejected.
  - Settings shows the new name after reopening.
- **Automation:** Covered by: Rust tests in `src-tauri/src/cli_mascot.rs` (`parses_name_and_rename`, `leaves_everything_else_to_the_cli`, `renames_and_replies_to_the_phrase_with_words_only`), `services/mascot.rs` tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-34 — Mascot hidden phrase

- **What:** A hidden phrase gets a one-line reply from the bull, and never an action. It works in the palette (Enter), the companion's input and the CLI. The UI shows the line as a toast and the bull's `done` pose. The phrase is stored only as a hash.
- **Where:** `services/mascot.rs` (`handle_phrase`, `PHRASES`), `mascot_phrase` command, `POST /api/rpc/mascot_phrase` `{input}`, `src/hooks/useMascot.ts` (`handlePhrase`), `src/lib/mascot.ts` (`isMascotPhrase`).
- **Preconditions:** Knowing the phrase (see the tests).
- **Steps:**
  1. Open the palette, type the phrase and press Enter.
- **Expected:** The palette closes, a toast shows the line, the bull celebrates, and no command runs.
- **Edge / negative cases:** Any other text goes on to normal handling.
- **Automation:** Covered by: `CommandPalette.test.tsx` ('answers the hidden phrase without running anything'), `src/lib/mascot.test.ts`, Rust `services/mascot.rs` tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## 1.8 Settings page

### ST-01 — Settings page and section rail

- **What:** Settings is a full page with a rail of eight sections: Models & runtimes, Account, Cloud credentials, Internet access, Backup, Sources, Logs, Appearance. Below 900 px the rail becomes a select. The page opens on the requested section, else the last section used this session, else Models. A section shows an amber "needs attention" dot in these cases:
  - Models: no callable model, or the default runtime is not ready.
  - Sources: a registry has a warning or error.
  - Backup: the remote is configured but unreachable, or has conflicts.

  The same rule drives the sidebar's Settings dot.

- **Where:** `src/components/settings/SettingsView.tsx`, `SettingsFrame.tsx` (`SettingsSectionFrame`, `SettingsGroup`, `AdvancedBlock`), `src/lib/settings-nav.ts` (`SETTINGS_SECTIONS`, `DEFAULT_SETTINGS_SECTION`), `src/hooks/useSettingsAttention.ts`, `ui-store.ts` (`settingsSection`).
- **Preconditions:** None.
- **Steps:**
  1. Open Settings from the sidebar.
  2. Open Backup, go to Home, and open Settings again.
  3. Narrow the window below 900 px.
  4. Remove all model access.
- **Expected:**
  - (1) Models & runtimes opens.
  - (2) Backup opens again.
  - (3) A section select in the header.
  - (4) An amber dot on Models.
- **Edge / negative cases:** After a relaunch the last section resets to Models (session only).
- **Automation:** Covered by: `src/components/settings/SettingsView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### ST-02 — Settings deep links and focus highlight

- **What:** Other places open a specific section and item. `openSettings(section, {focus, logTab, agentId})` scrolls to the element marked `data-settings-focus` and rings it for 2 s, retrying for up to 2 s while it renders. Examples: "Use an API key instead" on a runtime card opens Models & runtimes focused on that provider; the coding agents' anchor is `focus: runtimes`; the AWS access key anchor is `aws-access-key`.
- **Where:** `src/lib/settings-nav.ts`, `src/hooks/useSettingsFocus.ts`.
- **Preconditions:** None.
- **Steps:**
  1. In Settings › Models & runtimes, on a runtime card that needs a key, click "Use an API key instead".
- **Expected:** Models & runtimes scrolls to the provider row, which is highlighted briefly.
- **Edge / negative cases:** A focus target that doesn't exist: nothing happens, with no error.
- **Automation:** Covered by: `src/lib/settings-nav.test.ts` | Layer: unit | Gap: the scroll and highlight are not tested.
- **Needs relay:** no
- **Status:** Shipped

### ST-03 — Models: default model

- **Duplicate:** tested under MD-04, MD-05, MD-06. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-04 — Models: free tokens (trial)

- **Duplicate:** tested under MD-13, MD-14. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-05 — Models: provider keys (add, test, remove)

- **Duplicate:** tested under MD-01, MD-02, MD-03. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-06 — Models: Claude sign-in instead of an Anthropic key

- **Duplicate:** tested under MD-11. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-07 — Models: Ollama row

- **Duplicate:** tested under MD-12. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-08 — Models: build the sandbox images

- **Duplicate:** tested under RT-01, RT-02. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-09 — Models: coding agent runtimes (cards)

- **Duplicate:** tested under RT-08, RT-09, RT-10, RT-11, RT-12. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-10 — Models: Bedrock models and gateway

- **Duplicate:** tested under MD-20, MD-21. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-11 — Models › Advanced: enterprise config (`.nandi/llm.yaml`)

- **Duplicate:** tested under MD-18. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-12 — Models › Advanced: harness sessions

- **Duplicate:** tested under RT-21. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-13 — Account: edit name and handle

- **What:** Settings › Account is the only place to change the display name and handle after setup. It uses the same validation as onboarding and shows "Others find you as @handle." and the backup branch preview. If a remote is configured, changing the handle asks first ("Change your handle?"): "Your backup branch will change from <old> to <new>. Existing backups stay on the old branch.", with Cancel and "Change handle". Save shows "Identity saved."
- **Where:** `src/components/settings/sections/AccountSection.tsx`, `src/components/settings/IdentitySection.tsx`, `useIdentity` (`useSetDisplayName`, `useSetUserSlug`), ON-04 commands.
- **Preconditions:** Onboarded. A remote, for the confirmation.
- **Steps:**
  1. Change the name and Save.
  2. With a remote, change the handle and click Save.
  3. Click Change handle.
- **Expected:**
  - (1) `NANDI.md` is updated and `identity://changed` fires.
  - (2) The confirmation dialog.
  - (3) The remote's `user_slug` in `config.json` follows, and the branch becomes `<prefix>/<new>`.
- **Edge / negative cases:**
  - An invalid handle: Save is disabled with an error.
  - A load error: "Couldn't load your identity: …".
  - A save error: "Couldn't save: …".
- **Automation:** Covered by: `BackupSection.test.tsx` ('edits the identity'), Rust `services/auth/mod.rs` tests | Layer: unit | Gap: no test of the handle-change confirmation.
- **Needs relay:** no
- **Status:** Shipped

### ST-14 — Account › Details: device ID

- **What:** Under Details, shows the Device ID (the relay `instanceId`) with a copy button ("Copy device ID"), and "Stored in" `~/.nandi/repo/NANDI.md`.
- **Where:** `AccountSection.tsx`, `useRelay`.
- **Preconditions:** None.
- **Steps:**
  1. In Settings › Account, expand Details.
  2. Click the copy button next to the device ID.
- **Expected:** The clipboard holds the device ID.
- **Edge / negative cases:** No relay status yet: "—" and no Copy button.
- **Automation:** Covered by: none | Layer: unit | Gap: untested.
- **Needs relay:** no
- **Status:** Shipped

### ST-32 — Account: sign in with GitHub (one token for every github.com repository)

- **What:** Settings › Account › GitHub has **Sign in with GitHub**. It shows a code ("Enter this code on the GitHub page that opened:"), opens `https://github.com/login/device`, and waits ("Waiting for GitHub…", with Cancel). Once approved it shows "Signed in to GitHub as @login." with **Sign out**. The same panel (without Sign out) appears wherever a github.com repository is chosen: Library › GitHub › Setup, Settings › Backup when the credential is **GitHub sign-in** (offered only for `https://github.com/...` addresses), Add source with "Use my GitHub sign-in", the Create team dialog for a github.com registry URL, and Publish. The token is used for: the GitHub connector (after a pasted token, before the GitHub CLI), Backup push/pull/test, team registry sync and push, capability registry sync, marketplace sources with "Use my GitHub sign-in", creating a community source, and publishing. It is never sent to non-github.com addresses, and SSH addresses keep using SSH keys.
- **Where:** `src-tauri/src/services/auth/github.rs`, `commands/identity.rs` (`get_github_sign_in`, `start_github_sign_in`, `poll_github_sign_in`, `sign_out_github`), `/api/github/sign-in*`, `src/components/github/GithubSignInPanel.tsx`, `src/hooks/useGithubSignIn.ts`. Token: keychain entry `nandi.github.token`; login: `github_login` in `~/.nandi/config.json`.
- **Preconditions:** The Nandi OAuth App (or one set with `NANDI_GITHUB_CLIENT_ID` at build time) has "Enable Device Flow" on.
- **Steps:**
  1. Settings › Account → **Sign in with GitHub**; enter the code on github.com and approve.
  2. Library › GitHub: Start works with no pasted token.
  3. Settings › Backup: URL `https://github.com/<you>/<repo>.git`, credential **GitHub sign-in**, Save, Test, Push.
  4. Create a team with a private `https://github.com/...` registry; it syncs.
  5. **Sign out**, then Backup › Push.
- **Expected:** Steps 2–4 work without any pasted token. After step 5 the push fails with "Sign in to GitHub (Settings › Account) to use it for this remote", and the GitHub connector's Setup shows "Sign in to GitHub, or add a GitHub token" (unless the GitHub CLI is signed in). No token appears in any file under `~/.nandi`.
- **Edge / negative cases:** Cancel on github.com → "Sign-in was cancelled."; letting the code expire → "The sign-in code expired. Please start again."; the GitHub sign-in credential with a non-github.com URL → refused on Save; "Use my GitHub sign-in" while signed out → "Sign in to GitHub first (Settings › Account)"; a connector with a GitHub Enterprise `api_url` doesn't use the sign-in.
- **Automation:** Covered by: `auth::github` tests (URL rule, token lookup, device-flow provider), `oauth.rs` device/PKCE flow tests, `catalog.rs` setup-hint test | Layer: unit | Gap: the browser approval itself is manual.
- **Needs relay:** no
- **Status:** Shipped

### ST-15 — Cloud credentials: AWS sign-in (profiles and SSO)

- **Duplicate:** tested under MD-19. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-16 — Cloud credentials: AWS access key for agents

- **Duplicate:** tested under SB-08. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-17 — Internet access

- **Duplicate:** tested under SB-10. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-18 — Backup: set up the git remote

- **Duplicate:** tested under DT-18. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-19 — Backup: status, push, pull, change, remove

- **Duplicate:** tested under DT-20, DT-22, DT-25, DT-28. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-20 — Backup: push conflicts and merge conflicts

- **Duplicate:** tested under DT-24, DT-26. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-21 — Backup: remote branches

- **Duplicate:** tested under DT-27. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-22 — Backup: move off the old `nandi/` branch

- **What:** Settings › Backup offered to move a remote off the old `nandi/` branch (was a duplicate of DT-29).
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### ST-23 — Backup › This computer's copy

- **Duplicate:** tested under DT-31. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-24 — Sources (registries)

- **What:** One list of where the Library finds things, in two labelled kinds: Agents & packs (marketplace registries) and Capabilities. Each row has its URL, labels, last sync, Sync, a ⋯ menu and Remove. "Sync all" syncs them in turn. "Add source" chooses the kind first, then that kind's form. Overlaps the marketplace and capabilities areas.
- **Where:** `src/components/settings/sections/SourcesSection.tsx`, `sources-model.ts`, `useMarketplace` (`useRegistries`), `useCapabilityRegistries`.
- **Preconditions:** A registry URL.
- **Steps:**
  1. Add a capability registry.
  2. Click Sync.
  3. Click Sync all.
  4. Remove it.
- **Expected:** The last sync updates. A warning or error marks the row and the Settings dot.
- **Edge / negative cases:** A bad URL. An unreachable registry.
- **Automation:** Covered by: `src/components/settings/sections/SourcesSection.agents.test.tsx`, `SourcesSection.capabilities.test.tsx` | Layer: unit | Gap: see the marketplace area.
- **Needs relay:** no
- **Status:** Shipped

### ST-25 — Logs

- **What:** Settings › Logs shows the redacted audit logs in tabs: Model calls, Internet, Triggers, Pilots, Connectors, Decisions (approval decisions and who decided them). It can be opened on a tab (`logTab`). Overlaps the audit area.
- **Where:** `src/components/views/AuditLogView.tsx`, `src/hooks/useAuditLog.ts` (`AUDIT_KINDS`), `src-tauri/src/audit.rs`, `GET /api/audit/{kind}`.
- **Preconditions:** Some activity.
- **Steps:**
  1. Open Logs.
  2. Switch tabs.
- **Expected:** The entries are listed with durations and outcomes. Secrets are redacted.
- **Edge / negative cases:** An empty tab.
- **Automation:** Covered by: `src/components/views/AuditLogView.test.tsx` | Layer: unit | Gap: see the audit area.
- **Needs relay:** no
- **Status:** Shipped

### ST-33 — Settings › This computer (get your computer ready)

- **What:** A checklist of what agents need to act, checked read-only: Docker installed, Docker running, WSL 2 (Windows), Virtualization (Windows, macOS), Disk space (about 15 GB), Nandi's images (downloaded, or ghcr.io reachable). Each row shows Ready, Missing, Not running, Needs an update, Blocked or Couldn't tell, and what Nandi saw. A row that isn't ready adds one sentence saying why it matters, numbered steps for this OS (Linux: for the distribution family), copyable commands, and buttons: **Start Docker** (Docker Desktop, no admin rights) and links to official pages. A summary line says whether agents can act. The page re-checks every 10 seconds; **Check again** re-runs the slower checks too. "Docker Desktop is unable to start" lists, in order: quit Docker Desktop, `wsl --update`, `wsl --shutdown`, start it, check virtualization, Docker's factory reset. Linux shows `sudo systemctl start docker` and the `docker` group fix as commands; Nandi never runs them.
- **Where:** `src/components/settings/sections/ThisComputerSection.tsx`, `src/components/machine/` (`MachineChecklist`, `machine-model.ts`), `hooks/useMachine.ts`; Rust `services/machine/` (`check_machine`, `start_docker`; `GET /api/machine/readiness?fresh=true`, `POST /api/machine/start-docker`). See `docs/developer/machine-readiness.md`.
- **Preconditions:** None. For the states: Docker not installed, stopped, failing to start; on Linux, a user outside the `docker` group.
- **Steps:**
  1. Open Settings › This computer with Docker running.
  2. Quit Docker Desktop and wait up to 10 seconds.
  3. Click Start Docker.
  4. Click Check again.
  5. Linux: remove yourself from the `docker` group (new login) and open the page.
- **Expected:**
  - (1) Every row Ready (on the dev machine 2026-11-09: the Docker Desktop path, "Docker Engine 29.9.2", "WSL 3.0.5.0", "Turned on", "38.0 GB free at …", images "Downloaded"); "This computer is ready. Agents can act."
  - (2) Docker running: Not running, with Start Docker and steps; the summary says agents can only talk.
  - (3) Docker Desktop opens; the row turns Ready on a later re-check.
  - (4) WSL, virtualization, disk and image rows are re-read (otherwise kept for a minute).
  - (5) Docker running: Blocked, with `sudo usermod -aG docker $USER` and "Log out and back in".
- **Edge / negative cases:**
  - Offline or ghcr.io blocked with the image not downloaded: Nandi's images is Blocked with network and proxy steps; nothing else is affected.
  - Podman's `docker`, or rootless Docker only: Couldn't tell, with a note that it isn't supported yet.
  - Intel Docker Desktop on Apple silicon: Needs an update, with the Apple silicon download.
  - Arabic: steps read right to left, commands stay left to right.
- **Automation:** Covered by: Rust `services::machine::parse::tests`, `services::machine::assess::tests` (every state on Windows, macOS, Linux), `probe::tests::an_unreachable_registry_says_so_quickly`, offline suite `with_no_relay_the_computer_checklist_answers`; Vitest `machine-model.test.ts`, `MachineChecklist.test.tsx`, `SettingsView.test.tsx` | Layer: api + unit | Gap: macOS and Linux by hand.
- **Needs relay:** no
- **Status:** Shipped

## 1.9 Appearance, language and preferences

### ST-26 — Language and right-to-left layout

- **What:** Settings › Appearance › Language offers System Default, English, Français, 日本語 and العربية. A change applies at once, is saved to preferences and `settings.yaml`, rebuilds the native menu, and sets `<html lang dir>`. Arabic switches to right-to-left. At startup the saved language is used, else the system locale if it is available, else English.
- **Where:** `src/components/preferences/panes/AppearancePane.tsx`, `src/i18n/config.ts` (`rtlLanguages`), `src/i18n/language-init.ts`, `src/lib/system-locale.ts`, `locales/{en,fr,ja,ar}.json` (3,701 keys each), `docs/developer/i18n-patterns.md`.
- **Preconditions:** None.
- **Steps:**
  1. Choose العربية.
  2. Check the layout.
  3. Relaunch.
  4. Choose System Default on a French OS.
- **Expected:**
  - (2) `dir="rtl"`, the sidebar on the right and arrows mirrored.
  - (3) Still Arabic.
  - (4) French.
- **Edge / negative cases:**
  - An unsupported system locale (de) falls back to English.
  - Strings that are not translated: the Windows window-control labels.
- **Automation:** Covered by: `src/i18n/locales.test.ts` (only checks that the files are registered) | Layer: unit | Gap: no test that key sets match across locales, and no RTL visual test.
- **Needs relay:** no
- **Status:** Shipped

### ST-27 — Light, dark and system mode

- **What:** Mode can be Light, Dark or System (follows the OS live). It applies at once, persists, and is shared with the other windows (`theme-changed` event). Toasts follow the mode.
- **Where:** `AppearancePane.tsx`, `src/components/ThemeProvider.tsx`, `src/lib/theme-context.ts`, `src/hooks/use-theme.ts`, `src/theme-variables.css`; localStorage `ui-theme`.
- **Preconditions:** None.
- **Steps:**
  1. Choose Dark.
  2. Relaunch.
  3. Choose System and flip the OS theme.
- **Expected:** (1) The `dark` class is on `<html>`. (2) Still dark. (3) The app follows the OS without a reload.
- **Edge / negative cases:**
  - `save_preferences` rejects a mode other than light, dark or system ("Invalid theme…").
  - Dropdown lists stay readable in dark mode (fixed in `fc349ce`).
- **Automation:** Covered by: Rust `services/preferences.rs` (`theme_and_language_live_in_settings_yaml`) | Layer: unit | Gap: no ThemeProvider test.
- **Needs relay:** no
- **Status:** Shipped

### ST-28 — Color palettes and window icon

- **What:** There are seven palettes, each with a light and a dark version: Nandi (the default), Aurora, Ocean, Sunset, Forest, Graphite and High contrast (darker text and borders). Each card has a live preview. The choice sets `data-theme` on `<html>`, persists, and syncs to the other windows. The main window's taskbar and Alt+Tab icon change to that palette's bison. The installer, Start menu and macOS Dock keep the bundled icon.
- **Where:** `src/themes/palettes.ts`, `palette-icons.ts`, `contrast.ts`, `AppearancePane.tsx` (`PaletteCard`), `ThemeProvider.tsx`, `src/hooks/useWindowIconEffect.ts`; localStorage `ui-palette`.
- **Preconditions:** None.
- **Steps:**
  1. Choose Ocean.
  2. Look at the taskbar (Windows).
  3. Relaunch.
- **Expected:** (1) Colors change at once and `data-theme="ocean"` is set. (2) The icon changes. (3) Ocean persists.
- **Edge / negative cases:**
  - An unknown palette value falls back to Nandi.
  - The companion window updates to match.
- **Automation:** Covered by: `src/themes/palettes.test.ts`, `src/themes/contrast.test.ts` | Layer: unit | Gap: the icon swap is manual.
- **Needs relay:** no
- **Status:** Shipped

### ST-29 — Rename the mascot

- **What:** Settings › Appearance › Mascot sets the bull's name (1–24 characters: letters, digits, spaces, `-`, `'` or `.`, starting with a letter or digit). "Rename" shows the toast "The bull is now called <name>." "Back to Nandi" appears when the name is not the default. The name is shared with the CLI (`config.json` `mascot_name`).
- **Where:** `src/components/preferences/RenameMascot.tsx`, `useMascot` (`useRenameMascot`), `mascot_get` and `mascot_set` (`src-tauri/src/rpc/mascot.rs`); `POST /api/rpc/mascot_get|mascot_set` `{name}` (null resets).
- **Preconditions:** None.
- **Steps:**
  1. Enter "Gauri" and click Rename.
  2. Run `nandi name`.
  3. Click Back to Nandi.
- **Expected:** (1) The toast, and the title-bar bull uses "Gauri". (2) "Gauri". (3) "Nandi".
- **Edge / negative cases:**
  - "@@" is refused with the error toast "Couldn't rename: …".
  - Rename is disabled when the name is unchanged or empty.
- **Automation:** Covered by: Rust `services/mascot.rs` tests | Layer: api | Gap: no UI test for RenameMascot.
- **Needs relay:** no
- **Status:** Shipped

### ST-30 — Appearance › Companion settings

- **Duplicate:** tested under AI-17. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### ST-31 — Preferences persistence

- **What:** Theme, palette and language are saved twice. Portable values go to `~/.nandi/repo/.nandi/settings.yaml` (so they follow the repo and the remote). A startup copy goes to `<app data dir>/preferences.json`, written atomically (temp file and rename). Load returns defaults (system, default palette, system language) when the file is missing.
- **Where:** `src-tauri/src/commands/preferences.rs` (`load_preferences`, `save_preferences`), `services/preferences.rs`, `src-tauri/src/types.rs` (`AppPreferences`, `validate_theme`), `src/services/preferences.ts`; `GET` and `PUT /api/preferences`.
- **Preconditions:** None.
- **Steps:**
  1. Change the language and palette.
  2. Inspect both files.
  3. Delete `preferences.json` and relaunch.
  4. Call `PUT /api/preferences` with `{"theme":"blue","language":null}`.
- **Expected:**
  - (2) Both files hold the values.
  - (3) The values come back from `settings.yaml`.
  - (4) The error "Invalid theme: must be 'light', 'dark', or 'system'".
- **Edge / negative cases:**
  - A pull that changes `settings.yaml` from another machine.
  - A failed rename removes the temp file.
- **Automation:** Covered by: Rust `services/preferences.rs` (`theme_and_language_live_in_settings_yaml`) | Layer: api | Gap: no test for the invalid-theme error or the missing-file defaults.
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

- **Sign-in as identity is v2.** Signing in to Nandi itself (Entra ID, GitHub-org teams) is planned for v2 (`docs/tasks-todo/task-x-v2-sign-in.md`); AuthGate routes on local identity only. The optional GitHub sign-in in Settings › Account (ST-32) only supplies a token for github.com repositories.
- **Userguide steps still use the removed flow.** `docs/userguide/quickstart.md:56` and `docs/userguide/userguide.md:294,622` tell users to use "Sync now" in the account menu. Backup is now Settings › Backup, and "Back up now".
- **Entra ID sign-in is documented but not in the app.** `docs/developer/entra-setup.md` and `docs/userguide/userguide.md:504-506` describe Settings → Entra ID, Microsoft sign-in and "Sync from Entra ID" (the `docs/developer/README.md` index now marks it v2). No Settings section, command or UI exists for it. Only leftovers remain (for example in `db.rs` and `harness/gateway.rs`).
- **Untranslated text.** The Windows window-control aria labels are hard-coded English.
- **Minor code-comment mismatch.** `src-tauri/src/commands/remote.rs` says the default branch prefix is `nandi`. The code default is `nandi` (`DEFAULT_BRANCH_PREFIX`).
- **Custom themes** (`docs/tasks-todo/task-x-custom-themes.md`): not built. This covers theme files in `.nandi/themes/`, the editor in Settings › Appearance, import/export, sharing, and density and font scale. Only the seven built-in palettes exist.
- **Models and runtimes, Wave A leftover** (`docs/tasks-todo/task-x-models-and-runtimes.md`): a model picker per CLI runtime saved with `setHarnessDefault(id, model)`. Wave C, pulling prebuilt images from GHCR, is also not built.
- **Mascot as the assistant's name** (`docs/tasks-todo/task-x-nandi-alias.md`): not built. Ask's prompt, the companion, notifications, the CLI and i18n would use the mascot name; the setting would move to "Your assistant's name"; and "@alias" mentions plus extra aliases would be added.
- **Brand and trial follow-ups** (`docs/tasks-todo/task-x-followups-trial-brand-ask.md`):
  - The macOS Dock icon does not follow the palette.
  - There is no tray icon or monochrome mark.
  - The trial's mode is read from the keychain on every call (its tokens come from the trial gateway).
  - Checks still to run in the app: the palette changes the bull and taskbar icon, and Ctrl+K lists "New chat in Ask".
- **Network setup guidance**: onboarding does not walk through the networking opt-ins. Accepting delegation and trust are left for later, and only a note is shown.
- **Untested areas:**
  - No Back button or shortcut uses `goBack`; it is only called by views.
  - Sidebar and panel visibility and the last Settings section are not persisted across launches. This is by design, but worth knowing when testing.
  - There is no automated check that all four locale files have the same keys.
  - There are no tests for `services/recovery.rs`, `validate_filename`, the ErrorBoundary, the native menu, or ThemeProvider.

### ST-34 — Account: your email (the locked identity), read-only

- **What:** Settings › Account starts with **Your email**: "This Nandi's identity is the mailbox you connected first. The public relay and the free trial know you by it. It can't be changed here." and the email, the name and "From Gmail/Outlook". It's text, not a field. Before any mailbox: "No email connected yet. Connect Outlook or Gmail in Settings › Email." The identity is locked from the first mailbox's sign-in (its ID token: Gmail's verified `email`, Outlook's `preferred_username`); connecting another mailbox later doesn't change it.
- **Where:** `services/identity/mod.rs` (`lock_from_mail`, `identity`), `email/mod.rs` (`connect`), `config.json` `identity`, command `get_verified_identity`, HTTP `GET /api/identity/verified`, UI `components/settings/sections/AccountSection.tsx` (`VerifiedEmail`), `hooks/useIdentity.ts` (`useVerifiedIdentity`).
- **Preconditions:** none.
- **Steps:** 1. Fresh home: open Settings › Account. 2. Connect Gmail in Settings › Email, reopen Account. 3. Connect Outlook too.
- **Expected:** 1. The "No email connected yet" line. 2. The Gmail address, read-only. 3. Still the Gmail address.
- **Automation:** Covered by: `identity/mod.rs` tests (claims), `auth/oauth.rs` (`a_fresh_id_token_is_kept_and_an_old_one_renewed`), `identity/machine_id.rs` (hashes per OS, salts), `SettingsView.test.tsx` ('Account shows the locked email read-only', 'Account says when no email is connected yet') | Layer: unit | Gap: real Microsoft and Google ID tokens are manual.
- **Needs relay:** no
- **Status:** Shipped

### SH-35 — Home: connect your email to-do

- **What:** With no mailbox connected (skipped at setup, or disconnected later), Home shows a card "Connect your email so Chief of Staff can keep your notes." with **Connect Outlook** and **Connect Gmail** (each provider this build can connect) and **Email settings**. Connecting signs in from the card. The card goes away once a mailbox is connected; while the other provider is still free, one quiet line says "You can also connect <name>." with **Email settings**. Nothing shows when both are connected or the build can connect neither.
- **Where:** `src/components/home/EmailTodo.tsx`, `HomeView.tsx`, `settings/email/email-model.ts` (`emailTodo`), `useEmailStatus` (refreshes on `email://changed`).
- **Preconditions:** A build with at least one email client ID.
- **Steps:** 1. Skip the email step at setup; open Home. 2. **Connect Outlook** from the card and sign in. 3. Settings › Email › **Stop reading my email**; back to Home.
- **Expected:** Step 1: the card. Step 2: the card goes; the "You can also connect Gmail." line remains (if Gmail is available). Step 3: the card is back.
- **Edge / negative cases:** No dismiss: it is a to-do until a mailbox is connected. A mailbox that needs a new sign-in is handled in Settings › Email (Reconnect), not here.
- **Automation:** Covered by: `EmailTodo.test.tsx`, `email-model.test.ts` (`emailTodo`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-39 — Ask box answers from the built-in help guide with no model

- **What:** With no model at all (no key, no Claude Code sign-in, no Ollama default, and the free trial off, used up, paused or unreachable), the Ask box doesn't show an error: it answers from the built-in help guide ("No model is connected, so this answer is from Nandi's built-in help."). The guide covers: install Docker, connect a model, add an API key, Claude Code sign-in, Ollama, free tokens, join a relay, what agents, groups and the Inbox are, and where things are. Each answer is short, in the app's language (en, fr, ar, ja), with buttons to the right screen or page and other topics to open in place. Nothing matching: "I can't answer that without a model. These topics might help:" and every topic. When a model exists, the model answers; if its call fails (the trial unreachable), the guide answers with the reason underneath.
- **Where:** `services/help/` (`guide.json` content, `mod.rs` matcher), `services/ask/mod.rs` (`AskResponse::Help`, `AskLlm::has_model`), `components/help/HelpAnswerCard.tsx`, `HelpTopicView.tsx`, `lib/help.ts`; locales `help.*`. HTTP `POST /api/ask`.
- **Preconditions:** No model (remove keys, turn the trial off), or offline.
- **Steps:** 1. Ask "How do I install Docker?". 2. Click "Open This computer". 3. Ask "Comment rejoindre un relais ?". 4. Ask "bake me a cake".
- **Expected:** 1. The Install Docker topic with "Open This computer" and "Download Docker". 2. Settings › This computer. 3. Join your team's relay (in the app's language). 4. The no-match line and every topic.
- **Edge / negative cases:** Ask history shows the topic's title with a "Help" badge. The guide never calls a model.
- **Automation:** Covered by: `help/tests.rs` (every topic in four languages, word matching), `ask/tests.rs` (`with_no_model_the_help_guide_answers_without_calling_one`, `a_failed_model_falls_back_to_the_help_guide`), `offline_suite::with_no_model_the_ask_box_answers_from_the_help_guide`, `offline_suite::with_the_trial_gateway_unreachable_the_ask_box_still_helps`, `Help.test.tsx`, `help.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-40 — A chat sent with no model also gets the help guide

- **What:** In any chat box (an agent's page, Ask conversations, Chief of Staff threads) a message sent while no model can be called is sent as usual, and the help guide's answer to it appears above the box, dismissable.
- **Where:** `components/ask/AskBox.tsx` (chat mode, `useDefaultModel().effectiveModel === null`), command `help_answer` (HTTP `POST /api/help/answer`, body `{ text }`).
- **Preconditions:** No model.
- **Steps:** 1. Open an agent, type "install docker" in Chat and send.
- **Expected:** The message is sent; the Install Docker topic shows above the box. With a model, nothing extra shows.
- **Edge / negative cases:** Dismiss hides it; a link to a screen closes it.
- **Automation:** Covered by: `Help.test.tsx` ('a chat sent with no model also gets the guide', 'a chat with a model gets no guide'), `offline_suite` (`help/answer`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SH-41 — Help window from the no-model and Docker notices

- **What:** **How?** on the "No model connected" notice and on "Agents need Docker to act." opens a help window on that topic, with every topic listed to switch between. Links to a screen go there and close the window; web links open in the browser.
- **Where:** `components/help/HelpGuideDialog.tsx` (mounted in `MainWindow.tsx`), `useUIStore` (`openHelp`, `closeHelp`), `LlmKeyNotice.tsx`, `DockerSetupPrompt.tsx`, command `help_topics` (HTTP `GET /api/help/topics`).
- **Preconditions:** No model, or Docker stopped.
- **Steps:** 1. Click How? on the notice. 2. Pick "Install Docker". 3. Click "Open This computer".
- **Expected:** 1. The window on "Connect a model". 2. That topic. 3. Settings › This computer; the window closes.
- **Edge / negative cases:** Works offline (nothing is fetched from the internet).
- **Automation:** Covered by: `Help.test.tsx` ('the help window lists every topic and opens one') | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped
