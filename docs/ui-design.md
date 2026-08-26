# UI design

This document owns Aercast's visual language, visual tokens, component
appearance, and accessibility rules; it does not own product state, page
business, development progress, or backend behavior.

## Visual language

Aercast follows the **Compact GNOME / Dark / System Accent / Native Linux**
visual language. It does not attempt to reproduce Libadwaita. It uses the same
restrained visual language: neutral dark surfaces, rounded controls, symbolic
icons, clear type hierarchy, boxed lists, and accent color only where it
communicates focus, selection, status, or the primary action.

Use the compositor's ordinary system title bar. Do not draw a custom header
bar. Do not use gradients, glow, large fields of brand color, glass effects, or
stacked shadows.

## Brand image and icons

- `assets/aercast-icon.png` is the canonical application and tray image. Scale
  it from the supplied RGBA source; do not redraw, recolor, duplicate, or add a
  second brand asset.
- UI actions use a bundled minimal symbolic SVG set so behavior does not depend
  on the installed icon theme. Symbolic icons use the current foreground color
  and remain recognizable at the control's rendered size.
- An icon-only interactive control must have an accessible name and tooltip.
  State and meaning must never be conveyed by an icon alone.

## Theme and color

Aercast always renders a dark interface and ignores the system light/dark
preference. It still reads the standardized XDG Settings Portal accent,
contrast, and reduced-motion preferences independently.

Base neutral tokens:

| Token | Value | Use |
| --- | ---: | --- |
| `window-bg` | `#242424` | window background |
| `surface` | `#2c2c2c` | grouped content and inputs |
| `surface-raised` | `#343434` | hover and selected neutral surface |
| `border` | `#484848` | one-pixel boundaries |
| `text` | `#f6f5f4` | primary text |
| `text-muted` | `#c0bfbc` | secondary text |
| `text-disabled` | `#9a9996` | unavailable controls with a non-color cue |

### Accent derivation

1. Read `org.freedesktop.appearance` / `accent-color` as one sRGB `(r, g, b)`
   tuple. Every channel must be finite and within `[0, 1]`; otherwise treat the
   setting as absent.
2. Use the valid value as `accent-base`, or `#bd425a` when absent.
3. `accent-bg` equals `accent-base`.
4. `accent-fg` is whichever of `#ffffff` and `#1e1e1e` has the higher WCAG
   contrast ratio against `accent-bg`. If neither reaches `4.5:1`, use
   `#000000`.
5. `accent-standalone` begins at `accent-base` and moves only toward white until
   it reaches at least `4.5:1` against `window-bg`.
6. `accent-subtle` is a 15% alpha composite of `accent-base` over the current
   surface.

Color math operates in sRGB with WCAG relative luminance and contrast formulas.
All text and interactive states must meet WCAG AA. Error, warning, success,
focus, and selection always include text, iconography, border, or shape in
addition to color.

## Geometry

- Spacing scale: `4 / 8 / 12 / 16 / 24` logical pixels.
- Interactive control height: `34–36` logical pixels.
- Control corner radius: about `8` logical pixels.
- Grouped surface corner radius: about `12` logical pixels.
- Borders: one logical pixel normally; never create hierarchy with multiple
  nested outlines.
- Shadows: at most one subtle compositor-independent shadow for an elevated
  transient surface; ordinary grouped content uses borders and luminance only.

The main window content must fit `480×640` logical pixels without horizontal
overflow. It is not resizable, but layout must tolerate normal font metrics and
the compositor's server-side decoration size.

## Typography

Use the first available system font from `Adwaita Sans`, `Cantarell`,
`Noto Sans`, then the toolkit sans-serif fallback. Do not bundle a font.

| Role | Size | Treatment |
| --- | ---: | --- |
| Body and controls | about `13px` | regular |
| Supporting text | `11–12px` | muted, not low-contrast |
| Group title | about `15px` | medium weight |

Use sentence case. Prefer short labels over reduced font size. Numeric telemetry
may use tabular figures when the active system font provides them.

## Components

- **Primary button:** `accent-bg`, `accent-fg`, one clear action per view.
- **Neutral button:** `surface`, one-pixel border, brighter hover surface.
- **Destructive button:** neutral by default; destructive color and explicit
  wording appear in confirmation or active destructive state.
- **Icon button:** square control matching standard height, symbolic icon,
  tooltip, and accessible name.
- **Text input:** `surface`, one-pixel border, two-pixel accent focus ring;
  invalid input adds an icon and message.
- **Boxed list:** one grouped surface with single separators between rows. Do
  not wrap every row in another card.
- **Status:** compact text and symbolic icon. Reserve accent for the small
  portion that benefits from emphasis.
- The Viewer's initial Play action is centered over the video surface. After
  activation, use the browser's native playback, volume, and fullscreen controls.

Focus is always visible and uses a ring at least `2px` thick with sufficient
contrast against both the control and its surrounding surface.

## Motion and accessibility preferences

Normal transitions last `120–180ms` and use simple opacity or position changes.
When the Settings Portal reports reduced motion, remove non-essential
transitions and animate only feedback required to understand a state change.

When the Portal reports higher contrast, strengthen borders and separators,
raise muted-text contrast, and use at least a `3px` focus ring. High contrast
does not change the fixed dark theme. High contrast and reduced motion are
independent flags; every combination must work.
