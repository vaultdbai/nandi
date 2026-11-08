# Talk to Nandi: voice in and out

Status: idea, not started. Post-v1 work unless chosen for the demo (demo scope rule: noted here, not built now).

Original idea (owner, 2026-11-07): Add voice to Nandi. The Nandi icon talks back and shows the text, and it wakes on "Nandi yip yip".

Related: `task-x-personal-knowledge.md` (what Chief of Staff knows when it answers) and `task-x-companion-bison.md` (the bison companion window that voice builds on).

## The idea

Voice is a new way in and out of Chief of Staff. Agents, approvals and the record stay as they are.

```
mic → end of speech → speech to text → Chief of Staff → short spoken reply + full text on screen
```

Build it in stages: push-to-talk first, then the talking icon, then a wake word. Push-to-talk with fast replies gives most of the value; always-on listening is the riskiest part and the least needed.

## 1. Pieces

| Part           | Proposed                                                           | Why                                                                        |
| -------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------- |
| Mic capture    | The app window's browser microphone API                            | Echo cancellation built in, so the user can interrupt Nandi while it talks |
| Speech to text | Local `whisper.cpp` (Rust bindings), model downloaded on first use | Works offline, audio never leaves the machine, covers ar, fr and ja        |
| Text to speech | Local voices (Piper or Kokoro); a cloud voice as an option         | Offline by default; cloud voices sound more natural if the user allows it  |
| Later          | Live speech-to-speech cloud models                                 | Natural back-and-forth, but audio goes to the cloud, so off by default     |

- Fits the existing rules: works without relay, nothing sent out by default. Speech is handled in the desktop app; the agent still runs in its container.
- Check microphone permission on each platform: WebView2 on Windows, and the macOS entitlement for WKWebView.

## 2. The talking icon

- **The bison companion is the voice window.** No new window: the `companion` window already sits on top in a corner as the bison, and its shortcut (`COMPANION_SHORTCUT`, Ctrl/Cmd+Shift+Space) can double as push-to-talk. Captions show in its expanded card.
- **Bison states**, extending today's poses (`nandi-bull-{rest,wait,carry,done}.png`):
  - Idle: rest.
  - Listening (new): ears up, a ring reacting to voice volume.
  - Thinking: wait.
  - Speaking (new): head movement tied to speech volume.
  - Working on it: carry. Done: done.
  - Needs you: a gentle bounce for an approval.

  The still poses are enough for push-to-talk. For smooth movement, Rive fits because its animations switch between states on command; Lottie also works.

- **Captions:** the user's words appear live while they speak, so mishearing is visible. The answer's text appears in step with the speech.
- **Short spoken answers, full text on screen:** "Three things need you today; I've put them on screen," with the cards shown. Long plans are never read out.
- **Everything lands in chat:** each voice exchange is saved as a normal conversation, searchable and auditable.

## 3. Approvals by voice

- A spoken "yes" can approve low-risk actions.
- Risky actions (the ones that already ask first) still need a click or an on-screen confirm. Voice is easy to mishear, and someone else in the room could say "yes".

## 4. Wake word

**"Nandi yip yip"** (owner, 2026-11-07; replaces "yep yep"):

- "Yip" is rare in everyday speech, unlike "yep, yep", so it triggers by accident far less.
- "Nandi" alone is a common name in India; the full phrase avoids that.
- It fits the bison: "yip yip" is what makes Appa, the flying bison in _Avatar: The Last Airbender_, take off. Use the phrase only; no Appa imagery or show references in marketing without a legal check.
- Four syllables with distinct sounds, which wake-word detectors handle well.

Proposed:

- **"Nandi yip yip" is the signature phrase.** Nandi can answer with a short "yip yip!" when it wakes.
- **"Hey Nandi" as a plain alternative** in settings, for people who'd rather not say "yip yip" in an office.
- **Opt-in, with a clear mic indicator.** Pauses automatically during calls and screen sharing. The listener only looks for the phrase and keeps no audio.

Engines, both local:

| Engine                | Licence and cost          | Custom phrases                      |
| --------------------- | ------------------------- | ----------------------------------- |
| openWakeWord          | Open source, free         | Trained yourself; needs more tuning |
| Porcupine (Picovoice) | Commercial, paid at scale | Very easy                           |

## Order

1. Push-to-talk (hotkey or click the icon), local speech to text, text answer, optional spoken reply.
2. Voice in the bison companion: listening and speaking poses, captions in its card.
3. Interrupting while it talks, and the voice-approval rules.
4. "Nandi yip yip" wake word (opt-in), with "Hey Nandi" as the plain alternative.
5. Optional cloud speech-to-speech.

## Things to decide

- **Demo or post-v1:** a push-to-talk demo with the talking icon is far less work than a wake word.
- **Voice:** one Nandi voice as part of the brand, or a choice of voices?
- **Bison animation:** new still poses for listening and speaking are enough for v1; is smooth animation (Rive) worth it later?
- **Wake-word engine:** openWakeWord (free, more tuning) or Porcupine (paid, easier).
- **Cloud voice and speech-to-speech:** which providers, and whether they're allowed for company installs.

## Test

1. Push-to-talk: a spoken question gets a spoken short answer, the full text on screen, and the exchange in chat.
2. With the network off, speech to text and text to speech still work.
3. Captions show the user's words live and the answer in step with the speech.
4. Talking over Nandi stops its speech and takes the new question.
5. A spoken "yes" approves a low-risk action; a risky action still waits for a click.
6. With the wake word on, "Nandi yip yip" wakes it; an hour of normal conversation and calls (including "Nandi" as a name and "yep, yep") doesn't.
7. The wake word pauses during a call or screen share, and the mic indicator shows when it's listening.
8. Arabic, French and Japanese questions are transcribed and answered in the same language.
