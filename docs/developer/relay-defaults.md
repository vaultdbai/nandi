# Relay Defaults

Which relay an instance connects to, what the Relay screen suggests, and what changes when a public relay exists. How to run and test a relay is in `docs/relay-testing.md` of [vaultdbai/nandi-relay](https://github.com/vaultdbai/nandi-relay), where the relay server lives.

## Current behavior

A fresh install connects nowhere and enrolls nowhere until the user joins a relay. A plain build ships **no relay URL**. A build made with `NANDI_DEFAULT_RELAY_URL` (the release workflow, for a demo or company relay) only **offers** that relay as "Join the Nandi network"; see "A build's own relay" below. Without one:

- The Relay screen's URL field is prefilled with `ws://127.0.0.1:8787/relay` (`nandi-relay` on this machine, its default port). The prefill is a frontend constant, `DEFAULT_RELAY_URL` in `src/hooks/useRelay.ts`, mirrored by `relay::DEFAULT_RELAY_URL` in Rust. It is a suggestion only: nothing dials it until the user clicks **Join**.
- The screen shows a banner: "Running on the local network. Public relay coming soon." (`relay.lanBanner.*`), with a line saying relays are open and unencrypted.
- **Join** (`join_relay`, see "Joining a relay" below) writes the URL to `~/.nandi/config.json` (`relay_url`), so every later launch connects on its own. **Disconnect** clears it.

## Protocol version

`nandi_protocol::PROTOCOL_VERSION` (2 since the member directory and
people search) goes out on every `RegisterInstance`; the relay's `/health`
reports its own as `protocol_version`. A relay on another version refuses the
registration with the code `version_mismatch`. The client then keeps its
socket but counts as not connected (`RelayStatus.connected` false,
`versionMismatch` true, state Error, no heartbeats), so person-to-person
actions answer `RelayUnavailable`. Connection shows "Update Nandi or the
relay"; no other screen does. The next connection (the relay restarted
after its update) registers again. App and relay land the same version
together; nothing translates between versions.

## Precedence

Resolved by `services::relay::resolve_url` at launch and on every status read:

| Priority | Source                                                     | Notes                                                                                                                     |
| -------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| 1        | `NANDI_RELAY_URL` environment variable                     | For testing two instances. `RelayStatus.relayUrlSource = "env"`; the screen says so                                       |
| 2        | `relay_url` in `config.json` (`NANDI_HOME` moves the file) | Written by Join. Joining takes effect immediately even when the env var is set; the env var wins again at the next launch |
| 3        | none                                                       | Disconnected. The screen suggests `DEFAULT_RELAY_URL`. The build's relay is never used until joined                       |

A saved URL that is the build's relay reports `relayUrlSource = "builtIn"` (the Connection details say "This build of Nandi").

There is no command-line flag for the relay URL; `NANDI_RELAY_URL` covers that case and is listed under "Environment" in `nandi --help`. An instance token (`?token=…`) is split off the URL and kept in SQLite `relay_state` (`relay_token`, with the relay it is for in `relay_token_relay`), never in `config.json`. Disconnect keeps it, so joining that relay again reuses the seat; saving another relay drops it.

## Joining a relay

`services/relay/join.rs` (`join_relay`, the `join_relay` command, `POST /api/relay/join {url, joinCode}`) is the only place an install enrolls. Onboarding's network step (on **Join** or **Continue**) and Network › Connection (**Join a company relay**) call it. It never runs at startup.

1. `GET /health` on the relay's host: unreachable or not a relay → `unreachable`; another `protocol_version` → `versionMismatch`.
2. Then, by the relay:

| The relay                    | What joining does                               |
| ---------------------------- | ----------------------------------------------- |
| open (`auth: open`)          | Saves the URL; no code needed                   |
| secured, URL with `?token=`  | Saves it (an operator-issued token)             |
| secured, a token kept for it | Reuses it (joined before, then disconnected)    |
| secured, a join code         | `POST /relay/enroll`, saves the token, connects |
| secured, no code             | `codeNeeded`                                    |

The code is the one the user typed, or the build's `NANDI_RELAY_JOIN_CODE` when the URL is the build's relay. Refusals map to `JoinOutcome`: 401 `wrongCode`, 402 `relayFull` (the relay's message, shown verbatim), 403 `removed`, 404 `noEnrollment`, 409 `alreadyEnrolled`, other 4xx `refused`. Anything but `joined` changes no setting.

- **Never shown or logged:** the join code is a password field and is cleared once used; the token never reaches the UI. Logs name the relay only.
- **Invites:** a typed code is kept in the OS keychain (`nandi.relay.join_code`, `{relay, code}`) only for **Copy invite message** (`get_relay_invite`, `RelayInvite`). The invite line is `<relay url>#join=<code>` (`components/network/relay-invite.ts`); it never holds the instance token or the build's own code. **Paste invite** and pasting a message into the address field fill both fields.

## What a LAN relay is today

Say this plainly wherever the relay is mentioned to users:

- **Closed by default.** `nandi-relay` refuses to start without `NANDI_RELAY_SECRET` unless it's given `--open`. With the secret, only instances with a token can register and see who is on it, and the operator can revoke one at once (`/relay/revoke`).
- **TLS when configured.** `--tls-cert` and `--tls-key` serve `wss://`; without them it's plain `ws://`, for a trusted LAN or behind a TLS-terminating proxy. The app warns before it connects to a plain `ws://` relay on another machine.
- **Some persistence, basic limits.** Enrolments and revocations survive a restart; presence doesn't (instances re-register when they reconnect). Each connection may send 600 messages a minute.

See "A company relay" in nandi-relay's `docs/relay-testing.md` for running one. A relay serves 20 users free (more with a licence file); past that, enrolling is refused with the relay's reason, which the app shows on the Connection view.

## A build's own relay

So people who download Nandi start on a shared relay (a demo, or a community relay like a Discord server), the release workflow builds the app with two values (`.github/workflows/release.yml`):

| Build variable            | GitHub setting                                                            | What it does                                                                       |
| ------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `NANDI_DEFAULT_RELAY_URL` | repository **variable**                                                   | The relay a fresh install joins (`relay::built_in_url`), precedence 3 above        |
| `NANDI_RELAY_JOIN_CODE`   | repository **secret** (not a secret in the app: anyone can read a binary) | The code installs enroll with at that relay, once, to get their own instance token |

**Never build `NANDI_RELAY_SECRET` into the app.** It's the relay's master key: anyone could pull it out of the binary, mint a token for any instance ID, and impersonate people. The join code is the safe stand-in:

- The relay operator runs the relay with `NANDI_RELAY_SECRET` **and** `NANDI_RELAY_JOIN_CODE` (plus `--data-dir` / `NANDI_RELAY_DATA_DIR`, default `./relay-data`, where `relay.db` keeps the users and the revoked IDs).
- **Only when the user joins** ("Join the Nandi network" in onboarding, or Join on Network › Connection), the install calls `POST <relay>/enroll {"instanceId", "joinCode"}` (see "Joining a relay"). A fresh install that says **Not now**, or never opens onboarding, takes no seat. The relay checks the code and issues `HMAC(secret, instance_id)`, and the install keeps it in `relay_state`. Installs that joined reconnect at startup with that token, in the background.
- **Once per instance ID:** the first install to enroll an ID keeps it, and the relay records it in `relay.db`. A leaked join code lets strangers join, but it doesn't let them take over anyone's ID. Rotate the code on the relay (and the GitHub secret for the next build) whenever you like; issued tokens keep working.
- An install that loses its token keeps its ID and can't re-enroll it. The operator issues the token by hand (`/relay/token`), or the user resets their instance ID.

## Plan: a public relay

When a public relay is deployed, this file gets its URL, and:

1. `DEFAULT_RELAY_URL` (both copies) becomes the public `wss://` URL, still as a suggestion. Whether the app connects to it without a click is a product decision to record here at that point.
2. The banner goes away (or changes to name the relay in use).
3. The public relay runs with `NANDI_RELAY_SECRET` and TLS, and sign-in issues instance tokens, so the "no auth, no TLS" list above no longer applies to it. Self-hosted LAN relays keep working through `config.json` and `NANDI_RELAY_URL`, unchanged.

Until then: LAN only. Onboarding's "Join the Nandi network" is the build's relay (`RelayStatus.builtInUrl`), so a release built with `NANDI_DEFAULT_RELAY_URL` offers it by its host, and a build without one offers only a team network.
