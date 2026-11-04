# Task: Make the auto-updater work for v1

**From:** the feature test catalog review (2026-10-22). Tested as PL-08 and PL-09 in `docs/testing/data-platform-quality.md`.
**Size:** M
**Target:** v1

## Problem

The updater is switched on but can never find an update:

- `src-tauri/tauri.conf.json` → `plugins.updater.endpoints` points to `https://www.vaultdb.ai/downloads/nandi/latest.json` (and the same URL without `www`). Both are old Nandi URLs.
- `.github/workflows/release.yml` builds with `includeUpdaterJson: true`, so `latest.json` and the `.sig` files are attached to a **draft GitHub release**. Nothing publishes `latest.json` to the endpoint above.
- The app checks with `check()` in `src/store/update-store.ts` and in `src/components/companion/CompanionWindow.tsx`. The check silently finds nothing.
- The download names in the release are still `VaultDB-Nandi-*`.

## Scope

- [x] Pick the update source: the GitHub release (`https://github.com/vaultdbai/nandi/releases/latest/download/latest.json`), decided 2026-11-03. No upload step is needed.
- [x] Update `endpoints` in `tauri.conf.json` (2026-11-03). The `pubkey` is unchanged; the `TAURI_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets exist and match it (key ID EF7A9B2E00F238F8).
- [ ] Rename the release assets from `VaultDB-Nandi-*` to `Nandi-*` and update the website download links (see `task-x-relay-downloads-on-website.md`).
- [ ] Decide how the app behaves on a draft or unpublished release, and when it checks (startup, companion, a menu item).
- [ ] Update `docs/developer/releases.md` and the PL-08/PL-09 entries in `docs/testing/`.

## Acceptance

- [ ] An app built from version N finds, downloads, verifies and installs N+1 on Windows and macOS.
- [ ] A bad signature is refused.
- [ ] Offline, the check fails quietly and the app keeps working.
