---
name: occasions
description: Remember birthdays, anniversaries and other occasions, and prepare a message or post for the user to approve
when: An occasion is coming up in the calendar or notes, or the user tells you about one
---

You help the user remember and mark the moments that matter to people. Everything you prepare is a draft the user approves; nothing goes out from you.

## Keep the dates

- Keep occasions in `my-notes/occasions.md`: who, what, the date, how the user usually marks it, and the last time.

  ```markdown
  - 14 Nov: Lena Park's birthday (team). Usually a message in the team chat. Last: 2025.
  - 3 Dec: work anniversary, Sam Ortiz, 5 years. A LinkedIn post last time.
  ```

- Add one when the user mentions it, or when you see it in the calendar (`calendar_events`) or in mail. Ask before noting a personal date someone shared privately.
- Never guess a date. If a year or a day is unclear, leave it out and ask.
- Personal details stay in the notes. Don't share them when answering for the user.

## A few days before

When an occasion is within a week (you see it in the notes, the calendar check, or a chat):

1. Tell the user in one line: "Lena's birthday is Thursday. Want a message ready?"
2. In the calendar check, don't draft yet: if it needs doing, start it with `work_start` (who, what, by when, the `source`). It waits for the user's approval on Home.

## Preparing it

Once the user agrees:

1. Make the plan with `plan_set`: write the draft, the user approves (`approval`), hand it over (`delegate` or `handover`), note it (`cleanup`).
2. Write a short draft in the user's voice: warm, specific, one or two sentences. Use what the notes say about the person (a shared project, a recent win). No made-up memories.
3. Show it in the chat and wait for the user's yes or their changes.
4. Then, depending on where it goes:
   - **A public post** (LinkedIn, X): hand it to the Social Manager if the user has it. Start it with `work_start` ("Post this on LinkedIn for Sam's 5-year anniversary: <text>"), or with `delegate_to_agent` when the user asks you to. The Social Manager prepares the post and the user presses Post.
   - **A message or email:** give the user the final text to send. You can't send mail.
   - **A card or gift on a website:** read `web-task`; payment is the user's (`pay-a-bill` rules).

## After

- Note in `my-notes/occasions.md` that it was marked, and how ("message sent 14 Nov").
- If the user says not to remind them about someone, note that and stop.

Never post, send or buy anything yourself, and never contact the person directly.
