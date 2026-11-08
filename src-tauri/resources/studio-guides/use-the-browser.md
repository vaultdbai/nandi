---
id: use-the-browser
title: Use the browser
when: Agents opening web pages, clicking, filling forms, screenshots.
---

Agents use a real browser through the **browser pilots**: Browser (a window the user watches) and Browser (headless) (in a sandbox, no window). The user installs and starts one in Library › Tools, and allows it for the agent on first use. The agent's skill.md lists it under `pilots`.

## Only allowed sites open

The browser opens the sites in the agent's Internet access (`network: allow:`), narrowed by policies. For another site, Nandi asks the user (Allow once, Always allow, Don't allow), showing the `reason` the agent gives when it opens the page. IP addresses, local names, other ports and sites a policy rules out are refused without asking; the agent should say so instead of looking for a way around it. Add every site the task needs, including the ones the page loads from (a CDN, a sign-in host); `*.example.com` covers subdomains.

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

## Every run plans first

Before a run changes anything (clicking, typing, submitting, starting work, writing files outside its notes), the agent sets a step-by-step plan with `plan_set` and keeps it current with `plan_update`. The plan shows as a live checklist in the chat and in Run Detail. Submitting, paying, typing a secret and handing over run only while the step being done is a `submit`, `handover` or `approval` step; starting work or delegating needs a `delegate` or `approval` step. Approvals still apply. Before the run ends, every step is done or skipped, screenshots are in the log and the browser is closed; Nandi closes leftover pages when a run ends. Screenshots are saved per step with the run's files, kept 30 days and never backed up, and never taken during a hand-over.

## Hand over the last click

When a person must make the final click (posting on a social site, sending a form the agent may not submit), the agent fills the page and calls `pilot_browser__browser_hand_over` with `site`, a one-line `summary` and the `draft` it typed. Only the visible Browser has it. Nandi brings the window forward and asks the user; they press the button themselves and answer Done, Skip or Changed it (with the final text), which is the call's result. Tell the agent to record that answer.

A policy can refuse controls outright (`browser: { refuse: [{ controls: [post], sites: [linkedin.com], say: ... }] }`): the agent is told why and should hand over instead of looking for another way.

## Writing the instructions

Say the steps in order: open the page, what to click, what to read, what to screenshot. The browser works from the page's accessibility tree, so name controls by their visible label ("the Sign in button", "the Email field"). Ask for a screenshot of the result; it is saved with the run's files.
