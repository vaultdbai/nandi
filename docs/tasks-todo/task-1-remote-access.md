# Task: Use my Nandi from my phone or another browser, safely

**From:** owner, 2026-11-07, high priority. "How do I make sure my Nandi is accessible so I can interact with my phone or another browser and have it do things while the machine is in the office or at home? We need a passcode or something to make it safe."
**Size:** L. App only for steps 1 to 5; step 6 touches the relay and goes to the workspace's `tasks/` when it starts.
**Works without the relay:** yes. Remote access to your own Nandi never needs the relay (steps 1 to 5); the relay tunnel in step 6 is an option.

## What exists

- The web transport serves the whole app (`/`), the API (`/api/*`) and events (`/ws`): `src-tauri/src/http/`, `docs/developer/transports.md` › Web access.
- Default bind `127.0.0.1:<random>`; `--server [--port N]` binds `0.0.0.0:17463` and prints an access link.
- Auth is always on: one static token (`~/.nandi/web_token`), exchanged for an `HttpOnly; SameSite=Strict` cookie, or sent as `Authorization: Bearer`.

## Why that isn't safe for a phone yet

1. Plain HTTP: on shared Wi-Fi the token and everything after it can be read.
2. One permanent token: whoever has it controls agents, approvals, mail and the browser; no per-device revoke, no record of which device did what.
3. Off the home or office network it isn't reachable without opening a router port, which we must never ask users to do.
4. Approvals have no second factor: a stolen cookie can approve queued work.
5. The UI is a desktop layout.

## Today, without code (document it in the user guide)

Tailscale (or the company VPN) on the machine and the phone, Nandi started with `--server`, open `http://<machine-name>:17463/?token=…`. Encrypted, private to the user's devices, works from anywhere. Never port-forward.

## 1. Remote access in Settings

- [ ] **Settings › Remote access**, off by default, replacing the need for `--server`: on binds the LAN address (or only the VPN interface the user picks), shows the address, and survives restarts. Off closes the listener and ends remote sessions.
- [ ] Keeps `127.0.0.1` for the app's own use either way.

## 2. Devices and a passcode

- [ ] **Pair a device by QR code** (or a one-time link) shown in Settings: a short-lived pairing code becomes a **per-device key**, stored hashed. The static `web_token` stays for local scripts only and never works from another address.
- [ ] **A PIN or passcode** set at pairing (6+ digits, or the platform's passkey/WebAuthn where the browser supports it). Asked when a session starts and after an idle timeout.
- [ ] **Lockout:** a few wrong PINs lock that device until it's re-approved on the desktop; rate limits per address.
- [ ] **Device list** in Settings: name, last seen, address; revoke one device, or all.
- [ ] Secrets in the OS keyring; nothing secret in the URL after pairing.

## 3. Approvals need the PIN again

- [ ] From a remote device, every approval asks for the PIN: the intake queue's Approve, tool approvals, sends, payments, pilot hand-overs, trust changes, adding auto-approve rules or devices.
- [ ] Settings › Remote access lists what a remote device may never do (e.g. turn remote access off for the desktop, read the keyring, export diagnostics) and the desktop-only actions.

## 4. Encrypted transport

- [ ] HTTPS on the remote listener: a per-install certificate, its fingerprint pinned at pairing (shown in the QR), so there's no CA and no warning dialog after pairing. Behind Tailscale or a VPN, HTTP stays allowed only on that interface.
- [ ] `Secure` cookies and HSTS on the remote listener.

## 5. A phone layout

- [ ] What people do on a phone: Home's "Waiting for your OK" and "Since you were away", intake threads (chat with the Chief of Staff, Approve with the PIN, Mark done), Inbox, run status, and the companion's ask box.
- [ ] **An installable app (PWA)** served by the user's Nandi: web app manifest (Nandi name, icons, splash, `display: standalone`), a service worker, and install from the browser (iPhone: Share › Add to Home Screen; Android: Install). It updates itself with the user's Nandi; no app store.
- [ ] **Notifications** for new queue items and threads (Web Push; installed web apps on iOS 16.4+, Android Chrome).
- [ ] **Needs a trusted HTTPS address**: install and push don't work on a self-signed certificate on iPhone. Support Tailscale's HTTPS certificates (`*.ts.net`) first, and the relay route's certificate (step 6). The pinned self-signed certificate of step 4 still protects plain browser use.
- [ ] **Icons** come from the brand kit at build time, like the desktop app's icons, so the Apache repo still ships without the brand (`TRADEMARKS.md`).
- [ ] The branded store apps (iPhone, Android, watches, Siri and Gemini) are a separate task in the Nandi workspace; they are paired devices of this task, so steps 1 to 4 come first.

## 6. Later: through the company relay (optional)

- [ ] For people who can't install a VPN: the phone reaches the user's own Nandi through the company relay, end-to-end sealed (the relay sees metadata only, as for tasks), with the same device pairing and PIN. Spec first (`nandi-spec`), then the relay, then the app.
- [ ] Nandi must keep working, and remote access over a VPN must keep working, with no relay.

## Tests

- Remote access off: nothing listens on the LAN; on: only paired devices with a valid PIN get in; the static token is refused from another address.
- Lockout after wrong PINs; a revoked device is refused at once, including its open WebSocket.
- Every approval route refuses a remote session without a fresh PIN.
- Pairing code expiry and single use; the certificate fingerprint check.
- `pnpm run test:offline`: still boots with remote access on and no relay.
- Manual: iPhone Safari and Android Chrome over Tailscale; pairing, approvals, threads.
