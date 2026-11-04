# UI Patterns

## Overview

This app uses a modern CSS stack optimized for Tauri desktop applications:

- **Tailwind CSS v4** with CSS-based configuration
- **shadcn/ui v4** component library
- **OKLCH color space** for perceptually uniform colors
- **Desktop-specific defaults** for native app feel

## Tailwind v4 Configuration

Tailwind v4 uses CSS-based configuration instead of `tailwind.config.js`.

### File Structure

```
src/
├── App.css              # Main window styles + Tailwind imports
└── theme-variables.css  # Shared theme variables (colors, radii)
```

**Multi-window theming**: Both windows (`src/main.tsx` and `src/companion-main.tsx`) import `App.css`, which imports `theme-variables.css`, so they share the same theme tokens. When adding new color variables, add them to `theme-variables.css`.

### Structure

```css
@import 'tailwindcss'; /* Core Tailwind */
@import 'tw-animate-css'; /* Animation utilities */

@custom-variant dark (&:is(.dark *)); /* Dark mode variant */

@theme inline {
  /* Map CSS variables to Tailwind tokens */
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  /* ... */
}

:root {
  /* Light mode values */
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
}

.dark {
  /* Dark mode overrides */
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
}

@layer base {
  /* Global base styles */
}
```

### Key Concepts

| Directive              | Purpose                                              |
| ---------------------- | ---------------------------------------------------- |
| `@theme inline`        | Maps CSS variables to Tailwind's design token system |
| `@custom-variant dark` | Enables `dark:` prefix based on `.dark` class        |
| `@layer base`          | Base styles that apply globally                      |

### Adding Custom Colors

To add a new semantic color:

```css
@theme inline {
  --color-success: var(--success);
  --color-success-foreground: var(--success-foreground);
}

:root {
  --success: oklch(0.7 0.15 145);
  --success-foreground: oklch(1 0 0);
}

.dark {
  --success: oklch(0.6 0.15 145);
  --success-foreground: oklch(1 0 0);
}
```

Then use with Tailwind: `bg-success text-success-foreground`

## Dark Mode

### How It Works

1. **ThemeProvider** (`src/components/ThemeProvider.tsx`) manages theme state
2. Adds `.dark` class to `<html>` element when dark mode is active
3. CSS variables in `.dark` override `:root` values
4. Tailwind's `dark:` variant applies styles conditionally

### Theme Options

- `light` - Force light mode
- `dark` - Force dark mode
- `system` - Follow OS preference (default)

### Palettes and contrast

A palette (`data-theme` on `<html>`, `src/themes/palettes.ts`) is a brand hue and chroma. Every surface, the primary colour and the sidebar are derived from those two numbers in `theme-variables.css`, in light and dark. `high-contrast` also restates its text, border and status tokens in an `html[data-theme=…]` block, with a `html.dark[data-theme=…]` block for dark.

`src/themes/contrast.test.ts` evaluates the CSS for every palette in both appearances and checks WCAG AA. Text pairs (foreground on background, card, sidebar and primary, and so on) must reach 4.5:1, and status colours on the page 3:1. A new palette or token change that breaks this fails the test with the failing pairs listed.

Components use tokens only: status (`success`, `warning`, `info`, `destructive`, `highlight`) and kinds (`kind-agent`, …). The `no-raw-palette-colors` ast-grep rule rejects raw Tailwind palette classes and hex values.

### Using in Components

```tsx
// Access theme in components
import { useTheme } from '@/hooks/use-theme'

function MyComponent() {
  const { theme, setTheme } = useTheme()

  return <button onClick={() => setTheme('dark')}>Current: {theme}</button>
}
```

## Palettes

Appearance (light/dark/system) and palette are independent. The palette is
`data-theme="<id>"` on `<html>`. The default (`nandi`) sets no attribute.

- **A palette is two numbers.** `--brand-h` and `--brand-c` set the brand
  hue and chroma. The surfaces, primary, accent, ring, sidebar, and
  `chart-1` in `:root` and `.dark` are all derived from them, so a new
  palette is one `:root[data-theme='id'] { --brand-h; --brand-c }` block plus
  an entry in `src/themes/palettes.ts`. `palettes.test.ts` checks that the
  two stay in sync.
- **Status and kind colours don't change with the palette:** `success`,
  `warning`, `info`, `highlight`, `destructive`, and `kind-agent`,
  `kind-workflow`, `kind-tool`, `kind-pack`, `kind-trigger`. Use them with
  opacity modifiers, for example a pill is
  `border-success/30 bg-success/10 text-success`. **Don't use raw Tailwind
  palette classes** (`text-emerald-600`, `bg-amber-500/10`) for status; they
  ignore the theme and need a `dark:` twin. The only places that keep raw
  colours are the agent avatar palette and the macOS traffic lights.
- **`.surface-brand`** (sidebar and title bar) points the base tokens at the
  `--sidebar-*` ones. Components inside it use the normal classes
  (`bg-accent`, `text-muted-foreground`) and render correctly on the dark
  brand surface. It always uses the dark status shades.
- **Persistence:** the palette is the portable `palette` setting in
  `settings.yaml` (gold copy), saved through `AppPreferences.palette`. It is
  also mirrored to `localStorage['ui-palette']`, so the companion and quick
  pane windows can apply it on `theme-changed`.
- **Run status language:** green means done, blue means active, amber means
  waiting on the user, and red means failed. `RUN_STATUS_CLASSES`
  (`src/lib/run-status.ts`) and `STEP_STATUS_STYLES`
  (`workflow-builder/run-status-styles.ts`) are the shared maps.

### Why `.dark` Class (Not `light-dark()`)

This app uses the `.dark` class approach rather than CSS `light-dark()` because:

- Standard pattern for shadcn/ui ecosystem
- JavaScript control over theme switching
- Supports "system" preference detection
- Compatible with all shadcn components

## OKLCH Colors

All colors use the OKLCH color space for perceptual uniformity.

### Format

```css
oklch(lightness chroma hue)
oklch(0.7 0.15 250)  /* L: 0-1, C: 0-0.4, H: 0-360 */
```

### Why OKLCH

- **Perceptually uniform** - Equal steps in values = equal perceived change
- **Wide gamut** - Access to P3 display colors
- **Intuitive** - Lightness is predictable (unlike HSL)

### Color Palette Structure

| Token                                    | Purpose                   |
| ---------------------------------------- | ------------------------- |
| `--background` / `--foreground`          | Page background and text  |
| `--card` / `--card-foreground`           | Card surfaces             |
| `--primary` / `--primary-foreground`     | Primary actions           |
| `--secondary` / `--secondary-foreground` | Secondary actions         |
| `--muted` / `--muted-foreground`         | Subdued elements          |
| `--accent` / `--accent-foreground`       | Highlights                |
| `--destructive`                          | Destructive actions (red) |
| `--border` / `--input` / `--ring`        | Borders and focus rings   |

## Desktop-Specific Styles

The `@layer base` section includes styles that make the app feel native on desktop.

### Text Selection

```css
body {
  user-select: none; /* Disable by default */
}

input,
textarea,
[contenteditable='true'] {
  user-select: text !important; /* Enable in editable areas */
}
```

**Why:** Desktop apps typically don't allow selecting UI text, only content.

### Cursor

```css
* {
  cursor: default; /* Arrow cursor everywhere */
}

input,
textarea {
  cursor: text !important;
}

.cursor-pointer {
  cursor: pointer !important;
}
```

**Why:** Native apps use arrow cursor, not text cursor on labels.

### Scroll Behavior

```css
body {
  overscroll-behavior: none; /* Prevent bounce/refresh */
  overflow: hidden; /* Prevent body scroll */
}
```

**Why:** Prevents pull-to-refresh and elastic scrolling that feels wrong in desktop apps.

### Drag Regions

```css
*[data-tauri-drag-region] {
  -webkit-app-region: drag;
  app-region: drag;
}
```

Apply `data-tauri-drag-region` to elements that should drag the window (like title bars).

## Component Organization

```
src/components/
├── layout/           # App structure
│   ├── MainWindow.tsx
│   ├── AppShell.tsx      # Two panes: navigation and the current view
│   ├── Sidebar.tsx       # Navigation tree
│   ├── MainPane.tsx      # Renders the current View
│   ├── DockedInput.tsx   # Message box, routed by view
│   └── LeftSideBar.tsx
├── views/            # One component per navigation View
├── titlebar/         # Window chrome
│   ├── TitleBar.tsx
│   ├── MacOSWindowControls.tsx
│   └── WindowsWindowControls.tsx
├── ui/               # shadcn primitives
│   ├── button.tsx
│   ├── dialog.tsx
│   └── ...
├── command-palette/  # Command palette feature
├── preferences/      # Preferences dialog
├── ThemeProvider.tsx
└── ErrorBoundary.tsx
```

### Conventions

- **layout/** - Structural components that define app regions
- **titlebar/** - Platform-specific window controls
- **ui/** - shadcn/ui primitives (don't modify directly)
- **Feature folders** - Group related components together

## shadcn/ui Usage

### Adding Components

```bash
pnpm dlx shadcn@latest add button
pnpm dlx shadcn@latest add dialog
```

Components are copied to `src/components/ui/` and can be customized.

### Customizing Components

shadcn components are yours to modify. Common customizations:

```tsx
// src/components/ui/button.tsx
const buttonVariants = cva('...', {
  variants: {
    variant: {
      default: 'bg-primary text-primary-foreground',
      // Add custom variant
      success: 'bg-success text-success-foreground',
    },
  },
})
```

### Available Components

This app includes commonly needed components. Run `pnpm dlx shadcn@latest add [component]` to add more from [ui.shadcn.com](https://ui.shadcn.com/docs/components).

## The `cn()` Utility

All components use the `cn()` utility for conditional classes:

```tsx
import { cn } from '@/lib/utils'

function MyComponent({ className, disabled }) {
  return (
    <div
      className={cn(
        'base-styles here',
        disabled && 'opacity-50',
        className // Allow overrides
      )}
    >
      ...
    </div>
  )
}
```

**Pattern:** Always accept `className` prop and merge with `cn()` for flexibility.

## Component Patterns

### Layout Components

Layout components should:

- Accept `children` and `className` props
- Use flexbox with `overflow-hidden` to prevent content bleed
- Not set external margins (let parent control spacing)

```tsx
interface SideBarProps {
  children?: React.ReactNode
  className?: string
}

export function LeftSideBar({ children, className }: SideBarProps) {
  return (
    <div className={cn('flex flex-col h-full overflow-hidden', className)}>
      {children}
    </div>
  )
}
```

### Visibility with CSS

For panels that toggle visibility, prefer CSS over conditional rendering:

```tsx
// Good: Preserves component state
;<ResizablePanel className={cn(!visible && 'hidden')}>
  <SideBar />
</ResizablePanel>

// Avoid: Loses component state on hide/show
{
  visible && <SideBar />
}
```

This preserves scroll position, form state, and resize dimensions.

## Best Practices

### Do

- Use semantic color tokens (`bg-background`, `text-foreground`)
- Accept `className` prop on components
- Use `cn()` for conditional classes
- Keep desktop UX conventions (cursor, selection, scroll)
- Follow existing patterns in codebase

### Don't

- Use raw color values (`bg-white`, `text-gray-900`)
- Hardcode light/dark specific values
- Override shadcn components in place (copy and modify instead)
- Add `cursor-pointer` everywhere (only for actual clickable elements)
- Use viewport-based responsive design (this is a fixed-size desktop app)
