# Relay Defaults

Which relay an instance connects to, what the Relay screen suggests, and how people join the public relay with their connected email. How to run and test a relay is in `docs/relay-testing.md` of [vaultdbai/nandi-relay](https://github.com/vaultdbai/nandi-relay), where the relay server lives.

## Current behavior

A fresh install connects nowhere and enrolls nowhere until the user joins a relay. A plain build ships **no relay URL**. A build made with `NANDI_DEFAULT_RELAY_URL` (the release workflow, for a demo or company relay) only **offers** that relay as "Join the Nandi network"; see "A build's own relay" below. Without one:

- The Relay screen's URL field is prefilled with `ws://127.0.0.1:8787/relay` (`nandi-relay` on this machine, its default port). The prefill is a frontend constant, `DEFAULT_RELAY_URL` in `src/hooks/useRelay.ts`, mirrored by `relay::DEFAULT_RELAY_URL` in Rust. It is a suggestion only: nothing dials it until the user clicks **Join**.
- Every build also offers the public relay ("Join the public relay", see "The public relay" below). Nothing dials it or joins there until the user chooses it.
- **Join** (`join_relay`, see "Joining a relay" below) writes the URL to `~/.nandi/config.json` (`relay_url`), so every later launch connects on its own. **Disconnect** clears it.

## Protocol version

`nandi_protocol::PROTOCOL_VERSION` (4 since every payload is sealed) goes out on
every `RegisterInstance`; the relay's `/health`
reports its own as `protocol_version`. A relay on another version refuses the
registration with the code `version_mismatch`. The client then keeps its
socket but counts as not connected (`RelayStatus.connected` false,
`versionMismatch` true, state Error, no heartbeats), so person-to-person
actions answer `RelayUnavailable`. Connection shows "Update Nandi or the
relay"; no other screen does. The next connection (the relay restarted
after its update) registers again. App and relay land the same version
together; nothing translates between versions.

## Device keys

Each Nandi (each device) has its own keys: an Ed25519 signing key and an
X25519 sealing key, from the `nandi-seal` crate in nandi-spec.

| What | Where                                                                                               |
| ---- | --------------------------------------------------------------------------------------------------- |
| Made | On first relay use: the first `connect` to a relay, or enrolling with a join code. Never at startup |
| Kept | OS keyring, entry `nandi.instance.keys` (86 characters: one entry, no chunking)                     |
| Code | `services/relay/device_keys.rs` (`DeviceKeyring`, `load_or_create`)                                 |
| Sent | `EnrollRequest.publicKeys` and `RegisterInstance.publicKeys` (`ClientOptions::device_keys`)         |
| Used | Every person-to-person payload is sealed and opened with them (`services/relay/sealing.rs`)         |

- A keyring entry that can't be read is an error. Nandi never makes new keys over it: the relay would refuse them.
- The relay pins the keys for the instance ID. Other keys for the same ID are refused with `key_mismatch`. The client then counts as not connected (`RelayStatus.keyMismatch`, state Error), and Connection says to ask the relay's operator to rekey the device (`POST /relay/rekey`). Same handling as `version_mismatch`.
- Content never crosses the relay in the clear: see "Sealed content" in `a2a-delegation.md`.

Other members' keys are pinned too: see "Their key changed" in
`network-and-reassignment.md`.

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

| The relay                    | What joining does                                       |
| ---------------------------- | ------------------------------------------------------- |
| open (`auth: open`)          | Saves the URL; no code needed                           |
| secured, URL with `?token=`  | Saves it (an operator-issued token)                     |
| secured, a token kept for it | Reuses it (joined before, then disconnected)            |
| secured, a join code         | `POST /relay/enroll`, saves the token, connects         |
| secured, no code             | `codeNeeded`                                            |
| public (`/health` `public`)  | `publicRelay`: join it with the connected email (below) |

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

## The public relay

VaultDB.ai runs one public relay (nandi-relay `--public`, README "A public relay") for people without a company relay. There is no sign-in screen: a Nandi joins with its identity, the Outlook or Gmail mailbox connected in Settings › Email (`services::identity`, below), with "Join the public relay" in onboarding's network step and on Network › Connection (while no relay is set). Without a connected email it says "Connect your Outlook or Gmail in Settings › Email to join groups on the public relay" with a button there, and offers no other path. One account per person (the same verified email on several Nandis is one account), several Nandis; limits per person: 10 groups, 50 people per group, 500 messages a day.

| What     | Where                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| URL      | `public::PUBLIC_RELAY_URL` (`wss://relay.vaultdb.ai/relay`), or `NANDI_PUBLIC_RELAY_URL` at runtime (staging, tests). `BuiltInRelay::of_this_build` carries it as `public_url`; tests set their own with `with_public` (or none). `RelayStatus.publicRelayUrl` reports it, events included (`ClientOptions::offered`)                                                                                                                                                                                                                                                   |
| Offer    | `get_public_relay_offer` / `GET /api/relay/public`: the relay's `/health` (5 s). `ready {relay, email}` (the locked identity's email), `needsEmail {relay}` (no email connected), `unreachable {detail}` (no answer, not a relay, not `public`, no `identity` providers), `versionMismatch`, `none` (no public relay offered)                                                                                                                                                                                                                                           |
| Join     | `join_public_relay` / `POST /api/relay/public/join`: a fresh ID token of the identity's mailbox (`identity::fresh_id_token`, renewing the mail sign-in when the kept one has under 5 minutes left), this device's keys (made on first use, pinned by the relay) and the machine hash (`identity::machine_id`, the relay's salt) → `POST <relay>/public/join {instanceId, publicKeys, idToken, machineHash}` → `joined {relay, account, email, name}`, `needsEmail {message}` (no email, or its mailbox is disconnected or needs reconnecting), `refused`, `unreachable` |
| Joined   | The token is kept exactly like an enrolled one (`set_relay_url` with `?token=`: `relay_state`, never `config.json`), the account in `relay_state.relay_account` (`{relay, account, email, name}`), and the app connects. The token never reaches the UI or the logs; the ID token is sent once and never kept by the relay                                                                                                                                                                                                                                              |
| Account  | `get_public_relay_account` / `GET /api/relay/public/account`: `GET <relay>/account?instanceId=` with the token as bearer → `account {relay, thisInstance, account: {account, provider, email, name, instances, limits, usage}}`; `unavailable` (the kept account and why) while the relay doesn't answer; `notPublic` on any other relay                                                                                                                                                                                                                                |
| Refusals | `{error, code}` → `PublicRelayRefusal` (`idTokenInvalid`, `idTokenExpired`, `emailUnverified`, `identityUnavailable`, `accountSuspended`, `alreadyLinked`, `keyMismatch`, `deviceLimit`, `machineLimit`, `signupLimit`, `revoked`, `invalidRequest`, `unauthorized`, `noAccount`, `notPublic`, `other`); the UI words each (`publicRelay.refusal.*`) with the relay's words under it. A 5xx without a code is `unreachable`                                                                                                                                             |
| Trust    | `team::trust::on_a_secured_relay` is false on the public relay (`public::on_public_relay`: the kept account is for the token's relay), so "Trust everyone on this relay" doesn't apply there (`RelayTrust.public`)                                                                                                                                                                                                                                                                                                                                                      |
| Code     | `services/relay/public.rs` (tests: `public_tests.rs`, the real `nandi-relay` in public mode trusting its fake identity provider, `nandi_relay::testing::FakeIdp`), UI `components/network/PublicRelayJoin.tsx`, `public-relay-model.ts`, `PublicRelayAccountCard.tsx`                                                                                                                                                                                                                                                                                                   |

Nothing here runs at startup, and onboarding never waits on it: an unreachable relay is a calm line ("join later from Network › Connection"), and Continue goes on. `offline_suite.rs` (`an_unreachable_public_relay_never_blocks_onboarding`) checks it.

**Limits per person** (`services/relay/limits.rs`): past the daily message limit the relay sends an `Error` coded `daily_limit` naming the reset (00:00 UTC) and then refuses the message (a request's `refused` event, a reassignment's `ReassignResult`, a group change's `GroupReply` refused). The client retells each as `daily_limit@<reset ms>: <the relay's words>` (`Shared::retell`, as `no_shared_group`), `friendly-error.ts` maps it to `errors.dailyLimit` with the reset in the user's clock (`friendlyErrorParams`), and the place the action was taken shows it calmly (the ask composer, a request's outputs, `toastGroupError`). `group_limit` says the limits count per person. A clarification past the limit gets only the `Error` frame, which is logged.

**Before release, server side:** DNS for `relay.vaultdb.ai` and its TLS certificate; `NANDI_RELAY_GOOGLE_CLIENT_ID` and `NANDI_RELAY_MICROSOFT_CLIENT_ID` set to the app build's own mail client IDs (`NANDI_GOOGLE_CLIENT_ID`, `NANDI_MICROSOFT_CLIENT_ID`: the ID token audiences; no new OAuth apps, no secrets); `NANDI_RELAY_SECRET`, `--public`, backups.
