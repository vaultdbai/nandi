# Lock box

Keys agents can use but never see. **An agent can use a key, but never sees it, and a key works only at the place it belongs to.** The rules are enforced in code when a key is released, never in a prompt.

## Where things live

| What                                                                           | Where                                                                   |
| ------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| The value                                                                      | The OS keychain only, entry `nandi.lockbox.<name>` (`services/secrets`) |
| Kind, sites, agents, lock level, unlock time, header, last use                 | `state.db` table `lock_box_keys` (migration 012)                        |
| Open unlocks of Locked keys                                                    | Memory only (`LockBoxState`): a restart locks every key                 |
| Every add, change, delete, unlock, release and refusal (key name, never value) | The decision record, kind `keyRelease` (`inbox::audit`)                 |

The rules stay on this computer with the value, so they are not in the gold copy. A key's name is checked (`[a-z0-9][a-z0-9._-]{0,63}`) before it names a keychain entry.

## Code map

| Module                           | What it does                                                                               |
| -------------------------------- | ------------------------------------------------------------------------------------------ |
| `services/domain/lock_box.rs`    | Pure: `KeyRules`, `LockLevel`, `KeyHeader`, bound host normalizing and matching            |
| `services/lock_box/mod.rs`       | The service (add, update, delete, unlock), `release`, the record, the agent's prompt lines |
| `services/lock_box/approvals.rs` | Ask each time: the approval card and the waiting request                                   |
| `services/lock_box/unlock.rs`    | `SystemUnlock`: Windows Hello (`UserConsentVerifier`); none on macOS and Linux yet         |
| `services/lock_box/store.rs`     | The `lock_box_keys` rows                                                                   |
| `services/egress/http_proxy.rs`  | `POST /fetch` with `"key"`: the release, the header, the reply scrub                       |
| `rpc/lock_box.rs`                | The Settings commands                                                                      |

## A release

```text
POST /fetch { url, key } ─▶ proxy token ─▶ network.allow ─▶ not an internal address
  ─▶ lock_box::release(agent, key, url):
       known key?                               no ─▶ refused
       agent on its list?                       no ─▶ refused, recorded
       https, port 443, no user info, a name,
       covered by a bound host?                 no ─▶ refused, recorded
       a header to put it in?                   no ─▶ refused
       Open    ─▶ released
       Ask     ─▶ card (key, agent, host), waits ≤ 10 min ─▶ yes ─▶ released
       Locked  ─▶ unlocked now ─▶ released
                  else the OS prompt ─▶ unlocked for its time ─▶ released
                  no OS prompt here ─▶ the card, and `noSystemUnlock: true`
       value read from the keychain only now ─▶ recorded ─▶ last use
  ─▶ the agent's headers of that name dropped, the key's header added (sensitive)
  ─▶ sent with no redirects followed ─▶ the value replaced by `[key hidden]` in the reply
```

## Host matching

`normalize_bound_host` keeps a site as lowercase ASCII (punycode for an international name, as a URL carries it), with no scheme, port, path, user info or trailing dot. `*.example.com` covers one or more whole labels before `example.com`, never `example.com` itself. `*`, `*.com` and IPs are refused.

A `*.<base>` rule is also refused when `<base>` is itself a public suffix (`*.co.uk`, `*.com.au`, `*.github.io`, `*.s3.amazonaws.com`): every name under it belongs to someone else. The check uses the [`psl`](https://crates.io/crates/psl) crate, pinned with `=` in `Cargo.toml`. It compiles Mozilla's Public Suffix List (ICANN and private sections) into the binary, so it needs no file or network at run time, and it follows the list closely. `publicsuffix` reads the list at run time instead, which would mean shipping and updating a data file. Update the pin on purpose, with the domain tests. Exact hosts are not checked against the list: binding `myorg.github.io` is fine.

| ✅ Bound `api.github.com` matches | ❌ Never matches                                                        |
| --------------------------------- | ----------------------------------------------------------------------- |
| `https://api.github.com/user`     | `api.github.com.evil.com`, `api-github.com`, `x.api.github.com`         |
| `https://API.GitHub.com/`         | `http://api.github.com`, `:8443`, `user@api.github.com`, `140.82.112.6` |
| `https://api.github.com:443/`     | `api.github.com.` (trailing dot), Cyrillic `аpi.github.com` (punycode)  |

The URL parser lowercases and punycode-encodes the destination first; matching is exact string comparison after that, never "contains" or "ends with" without a label boundary.

## Lock levels

| Level         | Enforced by                                                                                                                                                                                                           |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Open          | The rules alone                                                                                                                                                                                                       |
| Ask each time | `approvals::ask`: an approval row (`tool_call_id` `lockbox:<id>`, tool `lock_box_key`) in the agent's active run. `chat::approve_action` hands the decision to the waiter, never to the orchestrator. No run: refused |
| Locked        | `LockBoxState::unlocked` (memory, with its end time), else `SystemUnlock::verify` (one prompt at a time). A cancelled prompt refuses and never falls back. No prompt on this computer, or it can't be shown: the card |

Changing a key's rules, Lock now and a restart end an unlock.

### Each run's memory

`lock_box::end_run`, called from `EventBus::turn_done`, clears what the Lock box keeps for a run (in memory, `RUNS`):

| What                             | In the run                                                             | At its end                                        |
| -------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------- |
| A card declined or expired       | That key and host are refused without a card (from the approval rows)  | A new run asks again                              |
| An OS unlock cancelled or failed | That key and host are refused without raising the prompt again         | A new run may prompt again                        |
| The same refusal again           | Recorded once; repeats (same key, agent, host and reason) only counted | One summary line: "the same refusal N more times" |

Outside a run nothing is remembered, so each refusal is recorded. The card is desktop only (`own_devices::desktop_only`). The card's args hold the key's name, the agent and the host: never the value.

## The agent's side

The agent's system prompt lists the keys it may use (name and sites only, `lock_box::prompt`) and how to call `POST $NANDI_FETCH_PROXY` with `"key"`. A key with no site or no header isn't offered. `CONNECT` tunnels can't carry a key: the proxy can't add a header inside TLS.

## Testing

- `domain/lock_box.rs`: names, bound hosts, matching (lookalikes, IDN, ports, case, trailing dots), headers.
- `lock_box/tests.rs`: every rule at release time, each lock level (with a fake `SystemUnlock` and a clock that moves), relock, the record, the prompt.
- `http_proxy.rs`: the header is replaced, a redirect isn't followed, a refused key sends nothing, and a whole run's release leaves the value nowhere (the reply, every `state.db` table, the decisions file, every gold-copy file, the logs through `test_log`).
- `secrets_suite.rs`: the value is never in the repo or its git history.
- Windows Hello itself: `cargo test system_unlock_here -- --ignored --nocapture` (is it set up) and `real_system_unlock -- --ignored` (shows the prompt).
