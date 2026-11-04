# Task: Custom themes (follow-up to the themes and design refresh)

**From:** `docs/tasks-done/task-*-themes-and-design-refresh.md` (§2 "A theme is data", §4)
**Size:** L

Built-in palettes, the token set, the contrast checks and the no-raw-colour rule shipped with the themes task. What's left is themes as user data:

- [ ] **Theme files**: `~/.nandi/repo/.nandi/themes/<id>.json` (a portable definition, so a file in the gold copy), validated in `services/domain/theme.rs` (pure), written only through `services/repos`, projected like any other entity.
- [ ] **Injection**: a validated custom theme becomes a `<style id="nandi-theme">` element; `checkContrast` (reuse `src/themes/contrast.ts`) refuses unreadable ones, listing the failing pairs.
- [ ] **Editor** in Settings › Appearance: built-ins are read-only ("Duplicate"); tokens grouped by meaning (Surfaces, Text, Status, Kinds, Charts); a live preview; contrast warnings inline; Save writes the file.
- [ ] **Import / export** of theme JSON.
- [ ] **Sharing**: `kind: theme` in the registry schema, so packs and team registries can carry themes.
- [ ] **Density and font scale** preferences (`--density`), with bundled fonts only.

## Acceptance

- [ ] A custom theme made in the editor survives a restart, syncs through git, and is refused when it fails contrast.
- [ ] `pnpm run check:all` passes.
