# Own Devices: Your Nandi From Your Phone

A person pairs their phone with their own Nandi. The phone then sees the Inbox, approves or declines, and asks Nandi from anywhere, through sealed envelopes on the relay (protocol 8). The relay forwards them and sees who, when and how big, never what. This page is the contract the phone web app (vaultdb.ai) follows.

| Piece                     | Where                                                                                                            |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Wire types                | `nandi_protocol::devices` (nandi-spec), `RelayMessage::Device*`, `Own*`                                          |
| Relay calls               | `services/relay/client.rs`: `device_request`, `send_own_envelope`, `on_devices`                                  |
| The service               | `services/own_devices/mod.rs` (`OwnDevices`)                                                                     |
| What a phone may say      | `services/own_devices/wire.rs`                                                                                   |
| What stays on the desktop | `services/own_devices/desktop_only.rs`                                                                           |
| Asking Nandi              | `services/own_devices/chat.rs` (`PhoneChat`, `AppChat`)                                                          |
| PIN                       | `services/own_devices/pin.rs` (Argon2id), `session.rs` (the ask session)                                         |
| Rows                      | `services/own_devices/store.rs`, migration `010_own_devices.sql`                                                 |
| Screen                    | Settings › Your devices (`DevicesSection.tsx`, `devices-model.ts`)                                               |
| Tests                     | `services/own_devices/tests.rs` (a real relay and a phone), `docs/testing/teams-network-relay.md` OD-01 to OD-06 |

## Works Without the Relay

Pairing, listing and revoking need the relay: without one the calls answer `RelayUnavailable` and the screen shows `RelayConnectNotice`. A relay at protocol 7 has no devices: `RelayFeature::OwnDevices` (message `DeviceRequest`) is in `missingFeatures`, the calls answer `RelayNeedsUpdate`, and nothing is sent. Unlocking a locked device is local and works offline. Nothing else in the app depends on a phone.

## Pairing

1. **Settings › Pair a phone.** The Nandi sends `DeviceOp::Pair`. The relay answers a device ID and a single-use code, valid for `PAIRING_TTL_MS` (5 minutes). The Nandi keeps a `pairing` row and shows a QR code.
2. **The QR code** holds `https://vaultdb.ai/pair#` + base64url (no padding) of the `PairingInvite` JSON:

   ```json
   {
     "relayUrl": "wss://relay.example/relay",
     "homeInstance": "desktop-ada-4f2c",
     "homeKeys": { "signing": "<base64url>", "sealing": "<base64url>" },
     "deviceId": "device.<32 hex>",
     "code": "<code>",
     "expiresAt": 1795000000000
   }
   ```

   The invite rides in the fragment, so it never reaches the site's server. `relayUrl` never carries a token.

3. **The phone** makes its own keys (`nandi-seal`: Ed25519 + X25519), and claims the code: `POST <https base of relayUrl>/relay/devices/claim` with `{"code", "publicKeys"}`. It gets `{"deviceId", "homeInstance", "token"}` and keeps the token.
4. **The phone shows its fingerprint**: `nandi_seal::fingerprint` of its public keys. That is SHA-256 over the signing key's 32 bytes then the sealing key's 32 bytes; the first 8 bytes in upper-case hex, in groups of four (`3F2A 91C0 7B44 E1D9`).
5. **The relay pushes `DeviceClaimed`** to the Nandi. Settings shows the same fingerprint. The person confirms only if the two match, and chooses a PIN (6 to 12 digits; not one digit repeated, not a straight run).
6. **The Nandi** hashes the PIN (Argon2id, 19 MiB, 2 passes), sends `DeviceOp::Confirm` with the keys the person checked, and marks the row `active`. The relay activates the device only for the keys it was claimed with.

Before step 6 the relay refuses the phone's `RegisterDevice` with `device_pending`, and the Nandi drops anything it sends.

## Envelopes

The phone connects to `relayUrl` and sends `RegisterDevice {deviceId, token, protocolVersion}`, then `OwnEnvelope {envelope}` frames and `DeviceHeartbeat {}` every 30 s. The Nandi's answers come as `OwnDelivery`. Nothing is stored at the relay: an envelope for a side that isn't connected is refused with `device_offline`.

Every envelope is a `nandi-seal` envelope whose `sealedFor` is:

| Field        | Phone → Nandi                    | Nandi → phone                   |
| ------------ | -------------------------------- | ------------------------------- |
| `taskId`     | the device ID                    | the device ID                   |
| `seq`        | increasing, kept across restarts | increasing (clock microseconds) |
| `from`, `to` | device ID → `homeInstance`       | `homeInstance` → device ID      |
| `purpose`    | `ownDevice`                      | `ownDevice`                     |
| `capability` | empty                            | empty                           |

The phone seals to `homeKeys` and signs with its own key; it opens the Nandi's answers with its keys and checks them against `homeKeys`. `sealedFor` is bound into the seal, so the relay can't change `seq` or the route.

**The Nandi drops, unanswered,** an envelope that is from an unknown, unconfirmed or revoked device, that doesn't open with the confirmed keys, that is sealed for someone else, or whose `seq` is at or below the last one it took from that device. The phone must keep its `seq` increasing (for example, the clock in microseconds, or a stored counter).

## The Content: Calls and Answers

Inside the seal, JSON, camelCase. Each call has the phone's own `id`; the answer echoes it as `re`.

| Call                                      | Answer                                                                         |
| ----------------------------------------- | ------------------------------------------------------------------------------ |
| `{"id","op":"inbox"}`                     | `{"re","outcome":"inbox","approvals":[…],"todos":[…]}`                         |
| `{"id","op":"approve","itemId","pin"}`    | `{"re","outcome":"decided","itemId","decision":"approved"}`                    |
| `{"id","op":"decline","itemId","pin"}`    | `{"re","outcome":"decided","itemId","decision":"declined"}`                    |
| `{"id","op":"todo_done","itemId","pin"}`  | `{"re","outcome":"decided","itemId","decision":"done"}`                        |
| `{"id","op":"todo_hand","itemId","pin"}`  | `{"re","outcome":"asked","chat","messageId","queued"}`                         |
| `{"id","op":"ask","text","chat"?,"pin"?}` | `{"re","outcome":"asked","chat","messageId","queued"}`                         |
| `{"id","op":"chat","chat","after"?}`      | `{"re","outcome":"chat","chat","messages":[…],"working","waitingForApproval"}` |
| anything else with an `id`                | `refused`, `unknown_command`                                                   |

An approval: `{id, agent, title, detail, createdAt, expiresAt, desktopOnly}`; `desktopOnly` is the reason it can only be approved on the computer, or `null`. A to-do: `{id, title, kind, from, dueAt, working}`; `working` is true while an agent's run works on it. At most 50 of each; titles 120 characters, details 280.

A refusal is `{"re","outcome":"refused","code","message","triesLeft"?}`:

| Code              | When                                                                      |
| ----------------- | ------------------------------------------------------------------------- |
| `pin_wrong`       | The PIN was wrong; `triesLeft` before the lock                            |
| `pin_needed`      | An ask with no PIN and no open session: ask again with the PIN            |
| `chat_not_found`  | Not one of this device's chats (or it's gone): start a new one            |
| `ask_invalid`     | An empty ask, or one over 4000 characters                                 |
| `device_locked`   | 5 wrong PINs in a row; everything is refused until the desktop unlocks it |
| `desktop_only`    | A risky approval: "Approve this on your computer."                        |
| `item_not_found`  | It no longer waits (decided, expired), or the to-do isn't open            |
| `unknown_command` | Not a call a phone has                                                    |
| `failed`          | Allowed, but it didn't go through (or an agent is on that to-do already)  |
| `asks_off`        | `todo_hand` or `ask` from a phone whose asks are off                      |

### To-dos From the Phone

`todo_done` and `todo_hand` take the PIN every time, like approvals, and the Decisions entry names the device (`gateway::todos::done`, `handed`).

- **`todo_done`** marks an open to-do done, as its Done button does.
- **`todo_hand`** hands it to Nandi as the phone's own ask: a new chat of the device's with "Please do this: <the to-do>". So the device's Phone Controls hold for that run, unlike a hand-off on the computer. The to-do follows the run (`working`), and the phone polls the chat as for an ask. It is refused while an agent works on it, and with `asks_off`.

### Asking Nandi

`ask` is the person's own message (their paired device, behind their PIN; unlike email, which stays untrusted). Without `chat` it starts a new chat with the Nandi agent: a conversation `phone:<device ID>:<uuid>`, named by its first message, one of the person's chats on the computer (`db::is_chat`, `db::is_phone_chat`: Recent shows it "From your phone"; Nandi gets no build tools in it). With `chat` it continues that one. The run's record is the person's (`RunSource::own_device`: source type `user`, source ID the device, label "Asked from <device name>"). A busy Nandi queues it (`queued: true`).

- **A device reaches only its own chats** (`chat::owns`): never the person's other chats, nor another device's.
- **The answer is polled.** The phone calls `chat` with `after` set to the last message ID it has, about every 2 seconds while `working` is true. `messages` are the person's (`user`), Nandi's (`assistant`) and a failed run's (`error`), oldest first: at most 20, each at most 4000 characters. Tool calls and Nandi's own continuations are left out. `waitingForApproval` is true while the run waits in the Inbox.
- **Every action still goes through approvals**, exactly as for a message typed on the computer. What a phone may approve is unchanged ([below](#what-stays-on-the-desktop)).
- **An ask is at most 4000 characters**, under the same seal, size and replay rules as every call.
- **The computer must be on.** The relay keeps nothing: when this Nandi isn't connected the phone gets `device_offline` and says "Your Nandi is offline".

### Phone Controls

Each device has an ask level (`controls::AskLevel`, `own_devices.ask_level`, migration 013): `confirm` (the default), `answer` or `off`, set with `set_own_device_ask_level`. `ask` answers `asks_off` at `off`. Every tool call of a run asked from a device (`controls::asked_from`: a `user` run with a device as its source) passes `call_gate`'s check 0, `controls::verdict`, with the level read at that call: reads (`only_reads`) run; at `confirm` everything else waits for an approval whatever `requires_approval` says; at `answer` it's refused; a device gone or locked refuses everything. `never_from_a_phone` (saves and build tools, `connector_access`, the profile and playbook proposals) is refused at every level, and `lock_box::release` never unlocks a key that asks first or is locked for such a run.

### The PIN

The PIN travels only inside the seal, with each approve and decline, and is checked on the Nandi against the slow hash. It is never stored on the phone, never in the clear on the Nandi, and never seen by the relay. The check comes first: nothing is decided, or said about the item, before it. A right PIN resets the count. Unlocking (Settings › Your devices › Unlock) resets it too.

**The ask session.** The first `ask` needs the PIN; asks after it don't, until the device goes 10 minutes without an ask (`session::IDLE_MS`). Then the next ask is refused `pin_needed`. Any right PIN opens or renews the session; reading the Inbox or a chat doesn't, so a phone left polling still locks. A wrong PIN, Unlock and Revoke end it. Approvals and declines still carry the PIN every time. Sessions live in the Nandi's memory only: after a restart every device needs its PIN again. A wrong PIN on an ask counts toward the lock like any other.

WebAuthn or Face ID on the phone can come later as a second check; the PIN stays the Nandi's own proof.

### What Stays on the Desktop

`desktop_only::reason` fails closed. A phone may approve only the agent's own sandbox tools (writing a file, running a command, a report). It may never approve:

- an access request (`connector_access`): new access;
- starting or delegating work (`work_start`, `delegate_to_agent`);
- a browser call that isn't reading or moving between pages;
- any other connector or pilot tool (`<id>__<tool>`): it acts outside the sandbox, whatever it's called;
- a tool whose name, word by word, says it sends, replies, forwards, posts, publishes, shares, invites, grants, pays, transfers, buys, orders, refunds, or changes access, permissions, trust or devices.

A phone may decline any of them. Pairing, revoking, settings, grants and keys aren't calls at all.

## The Record

A decision from a phone runs inside `inbox::audit::on_device`, so its entry in `.nandi/decisions/<yyyy-mm>.md` gets `- **Device:** <name> (<device ID>)`. `DecisionRecord.device` reads it back. Older entries have no such line.

## Revoking

Revoke removes the row first, so the device's envelopes are dropped at once, then sends `DeviceOp::Revoke` (its token stops working, its connection closes). Revoke all does the same for every device. When the list is read, a device the relay has active and this Nandi doesn't is revoked at the relay, and a row the relay no longer has goes here.

## The Risk to Name

A web app served from vaultdb.ai is only as safe as the site: a compromised site could serve an app that steals a pairing. The fingerprint check at pairing and the PIN on every decision limit what such an app could do; the riskiest approvals never happen from a phone. An ask can start work, but only as a message: whatever that work does waits for the same approvals, and the PIN opens a session of at most 10 minutes idle. Signed builds and Subresource Integrity are the web app's slice.
