---
name: sign-in-and-accounts
description: Sign in to websites safely, and keep the non-secret details of each site in the notes
when: A website task needs the user signed in, or the user tells you about an account
---

Signing in is the user's. You never see, type, ask for or store a password, card number, CVV or one-time code.

## Is the user already signed in?

Your browser keeps its own profile, so a site may remember the user. Open the site and look (`pilot_browser__browser_snapshot`): an account name or a Sign out link means you're in. Then skip the sign-in step (`plan_update` with `skipped`, "already signed in").

## Signing in

Plan it as its own step, before anything that needs the account.

- **The usual way: a hand-over.** Plan a `handover` step. Call `pilot_browser__browser_hand_over` with the `site`, a `summary` like "Please sign in to acme-energy.com, then answer Done", and an empty `draft`. Nandi brings the browser forward; the user signs in, including any code or CAPTCHA. Never take a screenshot during it.
- **A saved secret.** If "What your browser may type" lists a secret for this site, plan the sign-in as a `submit` step. Type the username from the notes or the listed values, and type the secret's name in the password field: the browser fills in the value and you never see it. The user approves it. If a code or CAPTCHA follows, hand over.
- **It failed.** Don't try again with other details. Hand over, or tell the user what the site said.

Never click "Remember this device", change security settings, or add a recovery address unless the user asks.

## Site profiles

Keep what a site needs, without secrets, in `my-notes/private/sites/<site>.md`, one file per site (`acme-energy.com.md`). Read it before the task and update it after. Write the whole file back.

```markdown
# acme-energy.com

- Account: electricity, customer number 55-1234
- Sign in: hand-over (the user has a passkey)
- Secret name: none
- Billing: Account › Billing › Pay bill
- Pays by: the saved card the user picks
- Last updated: 2026-11-08
```

- Good to keep: customer or account numbers, the billing address, the menu path, which secret name to use, quirks ("the Pay button is under More").
- Never keep: passwords, full card numbers, CVVs, one-time codes, security answers, recovery codes. If the user sends you one, say you won't keep it, and don't repeat it.
- `my-notes/private/` is for the user only. Never share it when answering for the user.

## Signing out

Stay signed in only if the user wants the site remembered. If they ask you to sign out, do it on the site and note it in the profile. Close the browser at the end of every task (`pilot_browser__browser_close`).
