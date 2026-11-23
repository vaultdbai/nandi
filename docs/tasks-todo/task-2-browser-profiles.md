# Task: My Nandi Browser, One Profile per Browser Type

**From:** owner, 2026-11-22: "add ability to create a profile in browser so user can use saved passwords if they have one and save that profile. profile should be based on browser type ... login to profile and you can save cookies etc in that particular place. we need to make users life easier."
**Serves:** trust (the person signs in themselves, chooses which agents use it, can wipe it) and state (one kept profile). For 0.3.0.

## Today

- The visible `browser` pilot runs Chrome or Edge on the host, through the egress proxy, the allowed-sites check and per-call approvals.
- Each `(agent, pilot)` has its own persistent profile in `~/.nandi/local/browser/<agent>/<pilot>/profile` (`services/pilots/browser.rs`), with **Sign out everywhere** (`sign_out`). So the person signs in to each site again for every agent.

## What to Do

### 1. One profile per browser type

- [ ] A shared profile per installed browser type: `~/.nandi/local/browser/profiles/<chrome|edge>/`. Never under the repo (the gold copy), never in a container.
- [ ] **Settings › Browser:** for each installed browser, **Open my Nandi browser** opens a visible window on that profile with no agent attached, so the person signs in to the sites they want, at their own pace. **Sign out everywhere** wipes that profile.
- [ ] Plain words on the page: "Sign in to Chrome (or Edge) in this window to bring your saved passwords through the browser's own sync. Nandi never copies passwords or cookies from your everyday browser."

### 2. Agents choose to use it, the person decides

- [ ] An agent's browser pilot uses either its own profile (today) or **My Nandi browser (Chrome|Edge)**. Using the shared profile is a grant the person gives per agent, recorded like other grants, and shown on the agent's page.
- [ ] Everything else stays: allowed sites, per-call approvals, the visible window, the egress proxy. Headless browsers in containers never get the shared profile.
- [ ] One browser process per profile at a time: a second agent waits or is told the profile is in use.

### 3. Never copy the everyday profile

- [ ] No reading or copying of the person's own browser profile, cookies or password store. The only path for saved passwords is the browser's own sign-in and sync inside the Nandi profile.

### 4. Docs and tests

- [ ] User guide (Settings › Browser, the agent's browser choice), `docs/testing/`, tests for the profile paths, the grant, and "in use".
