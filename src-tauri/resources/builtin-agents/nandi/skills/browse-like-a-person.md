---
name: browse-like-a-person
description: Get past pop-ups, cookie banners and ads, wait for pages, and check from the page that it really worked
when: Any time you use the browser, with web-task
---

Browse the way a careful person does: look, act, look again.

## What kind of job is it?

Be smart about it. Fit the plan, the screenshots and the ending to what the user wants:

| The job                                          | Plan                              | Screenshots                           | At the end                               |
| ------------------------------------------------ | --------------------------------- | ------------------------------------- | ---------------------------------------- |
| A task with steps (a form, a bill, a booking)    | Every step, as below              | Only where something changed for good | Close the browser                        |
| Something to watch or hear (a video, music)      | Open it, start it, check it plays | None                                  | **Leave it open**; say so in your report |
| Browsing for the user (find, compare, show them) | Short                             | None                                  | Leave the page they'll want open, or ask |

If you're unsure whether the user still wants the page, ask before you close it. Never close a page that's playing.

## Look, act, look again

- **Look before you act.** Take a fresh `pilot_browser__browser_snapshot` after every navigate or click. If it still shows "Loading", a spinner or an empty page, `pilot_browser__browser_wait_for` a second or two and look again. After a page changes, old element refs are stale: look again, don't reuse them.
- **Pop-ups first.** A dialog in the snapshot usually blocks the page behind it. Close it with "Close", "No thanks", "Not now", "Continue on web" or Escape, then look again: they often come in a row. Never sign up, give an email, sign in or install an app to get rid of one.
- **Cookie banners:** choose "Reject all", "Necessary only" or "Decline optional". If there's only "Manage", turn the optional ones off and save. Accept only if there's no other way past it, and say that you did.
- **Ads:** never click an ad, a sponsored result or "Learn more". On a video, an ad shows as "Ad", "Sponsored", "Ad 1 of 2" or a countdown. Wait for "Skip" (`pilot_browser__browser_wait_for` the text, about 8 seconds), then click it. If there's no Skip, wait for the ad to end and touch nothing.
- **Long pages:** press End or PageDown, wait a moment, look again. Stop when nothing new appears or you have enough.
- **Don't loop.** If the same thing fails three times, try another way (search, another link) or say where you're stuck.
- **Check it really worked,** from the snapshot, not a guess. A video plays when the player shows Pause, not Play, and its time moves between two looks a few seconds apart, with no "Ad" or "Video unavailable". Say what you saw. If you're not sure, say so.
- **Never get past a CAPTCHA, a "verify you're human" check or a bot wall.** Hand it over. Go one step at a time; never try to look like someone else.
