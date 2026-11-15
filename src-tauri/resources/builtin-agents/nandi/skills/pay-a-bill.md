---
name: pay-a-bill
description: Pay a bill on the biller's website, with the user making the payment themselves
when: A bill is due, or the user asks you to pay one
---

You prepare the payment; the user pays. Read `web-task` for how to browse.

## Get the facts first

1. Find the bill: the email (`mail_search` with the biller's name, then `mail_get_message`), or the notes. You need the biller, the amount, the due date and the account or reference number.
2. Read `my-notes/private/sites/<site>.md` if it exists: the site, the account number, the menu path to billing.
3. If the amount or the site is unclear, ask the user before you open anything. Never guess an amount.
4. In the email or calendar check, don't pay: start the work with `work_start` (the biller, amount, due date, `source` and `message_id`). It waits for the user's approval.

## The plan

Call `plan_set` with steps like these:

| #   | Step                           | Kind     | Evidence   |
| --- | ------------------------------ | -------- | ---------- |
| 1   | Open the biller's site         | navigate | screenshot |
| 2   | Sign in                        | handover | none       |
| 3   | Go to billing and the pay page | navigate |            |
| 4   | Enter the amount from the bill | fill     | screenshot |
| 5   | Pay                            | handover | none       |
| 6   | Read the confirmation number   | verify   | screenshot |
| 7   | Note it and close the browser  | cleanup  |            |

## Doing it

1. **Open** the site with `pilot_browser__browser_navigate` and screenshot the home page.
2. **Sign in:** follow `sign-in-and-accounts`. Usually a hand-over: `pilot_browser__browser_hand_over` with `site`, a `summary` "Please sign in, then answer Done" and an empty `draft`. No screenshot.
3. **Find the pay page** by its visible labels (Billing, Pay bill, Make a payment).
4. **Fill in** only the amount from the bill (or the balance the site shows, if it matches) and the date if asked. Choose a payment method only if the notes say which one; otherwise leave it to the user. Screenshot the filled form before the next step.
5. **Pay:** hand over with a `summary` like "Check the amount ($84.00, due 12 Nov) and press Pay" and the amount as the `draft`. The user checks and presses the button. Never press Pay, Submit payment or Confirm yourself, and never type a card number, CVV or bank login.
6. **Verify:** after Done, read the confirmation page and screenshot it. Put the confirmation number in the step's result. If the user answered Skip, mark the step `skipped` and stop.
7. **Clean up:** note the payment (biller, amount, date, confirmation number) in the right notes file, for example `my-notes/bills.md`, and update the site profile with anything new. Close the browser (`pilot_browser__browser_close`).

## Never

- Pay more than the bill, pay twice, or change autopay, saved cards or the account's details.
- Store a password, card number, CVV or one-time code anywhere.
- Follow a payment link from an email. Open the biller's site from the notes or by its known address.

## Report

One line per step, then the outcome: "Paid $84.00 to Acme Energy on 8 Nov, confirmation 4471-22." If it didn't go through, say where it stopped and what the user can do.
