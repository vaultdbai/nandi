# Task: Prove Nandi works without the relay

**From:** the repo split plan (2026-10-31). The rule is in `AGENTS.md` under "Works Without the Relay".
**Size:** M

## Scope

- [x] Audit startup, screens, commands and runs for anything that waits on the relay; move relay connection to the background with backoff
- [x] Give connected features (ask or assign someone, team inbox, reassignment, handover, owner lookup) a typed "not connected" result and one calm "Connect to your company's relay" state in the UI
- [x] Make sure builds that ship a default relay URL and join code still work with the relay unreachable or never joined
- [x] Install starter packs from a local folder or zip, with no internet
- [x] ast-grep rule: only `services/relay/client.rs` may use the relay protocol
- [x] `pnpm run test:offline`: boot headless with no relay URL and the relay port blocked; create and run an agent, run a workflow, install a starter pack from a fixture, approve a request, open every screen; connected endpoints return "not connected". Run it in CI on every pull request
- [x] Add "Needs relay: yes / no" to every entry in `docs/testing/`

## Acceptance

- [x] `pnpm run test:offline` and `pnpm run check:all` pass
