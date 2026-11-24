# Own Devices: Your Nandi From Your Phone

A person pairs their phone with their own Nandi. The phone then sees the Inbox and approves or declines from anywhere, through sealed envelopes on the relay (protocol 8). The relay forwards them and sees who, when and how big, never what. This page is the contract the phone web app (vaultdb.ai) follows.

| Piece                     | Where                                                                                                            |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Wire types                | `nandi_protocol::devices` (nandi-spec), `RelayMessage::Device*`, `Own*`                                          |
| Relay calls               | `services/relay/client.rs`: `device_request`, `send_own_envelope`, `on_devices`                                  |
| The service               | `services/own_devices/mod.rs` (`OwnDevices`)                                                                     |
| What a phone may say      | `services/own_devices/wire.rs`                                                                                   |
| What stays on the desktop | `services/own_devices/desktop_only.rs`                                                                           |
| PIN                       | `services/own_devices/pin.rs` (Argon2id)                                                                         |
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

| Call                                   | Answer                                                      |
| -------------------------------------- | ----------------------------------------------------------- |
| `{"id","op":"inbox"}`                  | `{"re","outcome":"inbox","approvals":[…],"todos":[…]}`      |
| `{"id","op":"approve","itemId","pin"}` | `{"re","outcome":"decided","itemId","decision":"approved"}` |
| `{"id","op":"decline","itemId","pin"}` | `{"re","outcome":"decided","itemId","decision":"declined"}` |
| anything else with an `id`             | `refused`, `unknown_command`                                |

An approval: `{id, agent, title, detail, createdAt, expiresAt, desktopOnly}`; `desktopOnly` is the reason it can only be approved on the computer, or `null`. A to-do: `{id, title, kind, from, dueAt}` (read only). At most 50 of each; titles 120 characters, details 280.

A refusal is `{"re","outcome":"refused","code","message","triesLeft"?}`:

| Code              | When                                                                      |
| ----------------- | ------------------------------------------------------------------------- |
| `pin_wrong`       | The PIN was wrong; `triesLeft` before the lock                            |
| `device_locked`   | 5 wrong PINs in a row; everything is refused until the desktop unlocks it |
| `desktop_only`    | A risky approval: "Approve this on your computer."                        |
| `item_not_found`  | It no longer waits (decided, expired)                                     |
| `unknown_command` | Not a call a phone has                                                    |
| `failed`          | Allowed, but it didn't go through                                         |

### The PIN

The PIN travels only inside the seal, with each approve and decline, and is checked on the Nandi against the slow hash. It is never stored on the phone, never in the clear on the Nandi, and never seen by the relay. The check comes first: nothing is decided, or said about the item, before it. A right PIN resets the count. Unlocking (Settings › Your devices › Unlock) resets it too.

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

A web app served from vaultdb.ai is only as safe as the site: a compromised site could serve an app that steals a pairing. The fingerprint check at pairing and the PIN on every decision limit what such an app could do; the riskiest approvals never happen from a phone. Signed builds and Subresource Integrity are the web app's slice.
