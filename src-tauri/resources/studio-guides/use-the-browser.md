---
id: use-the-browser
title: Use the browser
when: Agents opening web pages, clicking, filling forms, screenshots.
---

Agents use a real browser through the **browser pilots**: Browser (a window the user watches) and Browser (headless) (in a sandbox, no window). The user installs and starts one in Library › Tools, and allows it for the agent on first use. The agent's skill.md lists it under `pilots`.

## Only allowed sites open

The browser opens only the sites in the agent's Internet access (`network: allow:`), narrowed by policies. Every other site is refused, and the agent should say so instead of looking for a way around it. Add every site the task needs, including the ones the page loads from (a CDN, a sign-in host); `*.example.com` covers subdomains.

## What waits for the user

Reading pages, screenshots and opening allowed pages never ask. Clicking and typing, submitting a form, typing a secret, files and paying wait for the user's approval unless the agent's pilot entry allows them:

```yaml
pilots:
  - id: browser-headless
    allow: [interact, submit]
    values: { email: dev@acme.test }
    secrets: { SITE_PASSWORD: nandi.pilot.<agent>.site-password }
```

- `allow` picks from interact, submit, secrets, upload, pay. Allow only what the task needs; pay is never implied.
- `values` are plain values the agent may type (it reads them).
- `secrets` are keychain entries: the agent types the name and the browser fills in the value. The user saves the value on the agent's Permissions tab. Never put a password in skill.md.

## Writing the instructions

Say the steps in order: open the page, what to click, what to read, what to screenshot. The browser works from the page's accessibility tree, so name controls by their visible label ("the Sign in button", "the Email field"). Ask for a screenshot of the result; it is saved with the run's files.
