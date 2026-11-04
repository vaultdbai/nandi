# Task: Company sign-in (GitHub, Microsoft Entra ID), moved to v2

**From:** the feature test catalog review (2026-10-22).
**Target:** v2. Not in v1.
**Size:** L

## Where things stand

v1 has no sign-in. The first-run gate (`src/components/layout/AuthGate.tsx`) routes on the local identity in `NANDI.md`. Teams can be any git URL. GitHub auth was removed in `91c8fad`. Agents sync through Settings › Backup (git push and pull). See ON-01 and TM-02 in `docs/testing/`.

The sign-in docs are kept for v2 and marked with a banner: `docs/developer/github-auth.md` and `docs/developer/entra-setup.md`.

## v1 cleanup (before the v1 release)

- [ ] User guide: remove "Sync now", "Log out", Entra ID and GitHub-org teams from `docs/userguide/userguide.md` (§3, §5 "Editing an agent", §9 "GitHub-org teams" and "Entra ID groups", §12 reset steps) and `docs/userguide/quickstart.md`.
- [ ] Remove leftover Entra code paths, or mark them dormant (`db.rs`, `services/harness/gateway.rs`).

## v2 scope

- [ ] Company identity first (`docs/developer/sign-in.md`): Entra ID / GitHub device flow, tokens in the OS keyring.
- [ ] Teams from a GitHub org or an Entra group, alongside the plain git URL.
- [ ] Decide whether sign-in gates the app or stays optional. v1 users must keep working without it.
- [ ] Add catalog entries in `docs/testing/` (`ON-` for the sign-in flow, `TM-` for org and group teams).
