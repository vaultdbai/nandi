# Dependency Alerts

Open Dependabot alerts that can't be cleared by a normal update, and the pins that clear the rest. Re-check after each Tauri release.

## Open

| Package | Alert               | Status                | Reason                                                                                                                                                                                                                                                                                                                                                    |
| ------- | ------------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `glib`  | #1 (medium), < 0.20 | Documented, not fixed | Locked at 0.18.5. It comes from Tauri's GTK3 stack on Linux only (`tauri` > `tao` and `muda` > `gtk` > `glib`). Every gtk-rs 0.18 crate is pinned to it, so it can't move alone. The bug is in `glib::VariantStrIter`. Nandi never uses glib or `glib::Variant` directly, so the code path isn't reachable. Fix: a Tauri release on gtk-rs 0.20 or later. |

## Fixed by a pin

Pins live in `pnpm-workspace.yaml` (`overrides`). Remove one when the parent package requires a patched version itself.

| Package           | Pin                                                      | Why                                                                                                                                                           |
| ----------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `seroval`         | 1.6.11                                                   | `solid-js` (pulled in by the TanStack Query devtools) asks for `~1.5.4`. The pin crosses that range. The devtools are dev-only, and the tests and build pass. |
| `seroval-plugins` | 1.6.11                                                   | Must match `seroval`.                                                                                                                                         |
| `source-map-js`   | 1.2.2                                                    | `css-tree` asks for 1.2.1.                                                                                                                                    |
| `minimatch`       | 11.0.1 (for `@typescript-eslint/typescript-estree` only) | `typescript-estree` asks for 9 or 10. Lint-time only, and lint passes.                                                                                        |

## Fixed by an update

| Package                                           | Change                                                                             |
| ------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `zip` 4.6.1 (Rust)                                | `tauri-plugin-updater` 2.13.2 to 2.14.1 drops zip 4. The app's own `zip` is 9.2.2. |
| `vite`                                            | 8.3.1 to 8.3.8 (`package.json` range raised)                                       |
| `vitest`, `@vitest/mocker`, `@vitest/coverage-v8` | 4.1.11 to 4.1.12                                                                   |
| `undici`                                          | 7.30.0 to 7.30.2 (through `jsdom`)                                                 |
