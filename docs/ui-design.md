# UI design

This document owns Aercast's visual language, visual tokens, component
appearance, and accessibility rules; it does not own product state, page
business, development progress, or backend behavior.

## Visual language

Aercast is a compact Linux desktop utility: dense enough for experienced users,
calm enough to scan while sharing, and implemented with iced rather than
imitating GTK or a browser design system.

Use the compositor's ordinary system title bar. Do not draw a custom header bar,
duplicate Close control, or Host video preview. Build hierarchy from typography,
spacing, neutral surface contrast, and separators. Group related controls on
flat surfaces; do not nest every section and row inside another card.

Do not use gradients, glow, glass effects, large fields of brand color, custom
content shadows, or decorative animation. The compositor may draw its normal
window shadow.

## Brand image and icons

- `assets/aercast-icon.png` is the canonical application, tray, and Viewer
  favicon image. Its RGBA source uses a tight square canvas so the visible mark
  fills the available surface when scaled; do not redraw, recolor, duplicate,
  or add a second brand asset.
- UI actions use a bundled minimal symbolic SVG set so behavior does not depend
  on the installed icon theme. Symbolic icons use the current foreground color
  and remain recognizable at the control's rendered size.
- Overview, Viewers, and Settings each have one bundled symbolic navigation
  icon. They accompany text labels; they never replace them.
- An icon-only interactive control must have an accessible name and tooltip.
  State and meaning must never be conveyed by an icon alone.

## Theme and color

Aercast always renders a dark interface and ignores the system light/dark
preference. It still reads the standardized XDG Settings Portal accent,
contrast, and reduced-motion preferences independently.

The default neutral palette follows One Dark:

| Token | Value | Use |
| --- | ---: | --- |
| `bg` | `#282C34` | One Dark window background |
| `surface` | `#21252B` | sidebar and grouped content |
| `control` | `#2C313A` | inputs and neutral buttons |
| `hover` | `#3E4451` | hovered and raised control states |
| `border` | `white 10%` | ordinary boundaries and separators |
| `text` | `#ABB2BF` | primary text |
| `secondary` | `#9DA5B4` | labels, descriptions, inactive navigation |
| `muted` | `#7F848E` | unavailable and disabled controls |
| `danger` | `#E06C75` | destructive actions |
| `success` | `#98C379` | positive status |
| `warning` | `#E5C07B` | caution and mismatch status |

### Accent derivation

1. Prefer a literal `accent_bg_color #RRGGBB` from the active GTK 4 user
   stylesheet or one local stylesheet imported directly by it. This matches
   desktop themes whose Portal backend exposes only a generic color family.
2. Otherwise read `org.freedesktop.appearance` / `accent-color` as one sRGB
   `(r, g, b)` tuple. Every channel must be finite and within `[0, 1]`; treat an
   invalid value as absent.
3. Use the valid value as `accent-base`, or the One Dark blue `#61AFEF` when
   absent.
4. `accent-bg` equals `accent-base`.
5. `accent-fg` is whichever of `#ffffff` and `#1e1e1e` has the higher WCAG
   contrast ratio against `accent-bg`. If neither reaches `4.5:1`, use
   `#000000`.
6. `accent-standalone` begins at `accent-base` and moves only toward white until
   it reaches at least `4.5:1` against `bg`.
7. `accent-subtle` is a 15% alpha composite of `accent-base` over `surface`.

Color math operates in sRGB with WCAG relative luminance and contrast formulas.
All text and interactive states must meet WCAG AA. Error, warning, success,
focus, and selection always include text, iconography, border, or shape in
addition to color.

`accent-bg` is reserved for the one primary action in a view and compact
controls such as a checked box. Navigation and option selection use
`accent-subtle` with an `accent-standalone` border and normal text. Focus rings
and small active-status markers use `accent-standalone`; ordinary content
surfaces remain neutral even when the Portal supplies a saturated accent.

## Window and layout

The Host is one ordinary resizable layout, not separate wide and narrow
component trees. The compositor receives a `960×640` logical-pixel initial size
hint. The minimum height is `480`; given the current monitor's logical width
`W` and height `H`, the minimum width is:

```text
max(640, min(W, H × 16 / 9) / 4)
```

Projecting an ultrawide monitor through its height makes the quarter-width rule
match an equivalent 16:9 display. Query the current monitor when the window
opens, is shown again, or is resized, and update the minimum only when the
logical result changes. HiDPI scale factors must not be applied a second time.

- All spacing, padding, gaps, control dimensions, icon dimensions, and radii
  follow a `4px` grid. Font sizes and one-to-three-pixel strokes are optical and
  accessibility exceptions.
- Spacing scale: `4 / 8 / 12 / 16 / 20 / 24` logical pixels.
- Interactive control height: `36` logical pixels.
- Control and group corner radius: `8` logical pixels.
- Icon-and-text sidebar width: `192` logical pixels.
- The single-column content area has `20px` outer padding and a `960px` maximum
  width, centered in the space beside the sidebar. It remains usable without
  horizontal clipping at the `640×480` floor.
- Sections use `16–24px` vertical gaps and one-pixel separators. Do not create
  hierarchy with nested outlines.
- Content overflow keeps its scrollbar hidden. Keyboard, wheel, touchpad, and
  focus-reveal scrolling remain available.

## Typography

Use the first available system font from `Adwaita Sans`, `Cantarell`,
`Noto Sans`, then the toolkit sans-serif fallback. Do not bundle a font.

| Role | Size | Treatment |
| --- | ---: | --- |
| Body and controls | about `14px` | regular |
| Supporting text | `12–13px` | secondary or muted, not low-contrast |
| Page title | `20px` | bold, primary text, uppercase not used |

Use sentence case. Prefer short labels over reduced font size. Numeric telemetry
may use tabular figures when the active system font provides them.

## Components

- **Primary button:** `accent-bg`, `accent-fg`, and bold text. Each view exposes
  at most one primary operation; navigation and option selection stay neutral.
- **Selected button:** `accent-subtle`, normal text, and a one-pixel
  `accent-standalone` border.
- **Neutral button:** `control`, subtle border, `hover` on hover.
- **Destructive button:** `danger` background with explicit destructive wording;
  it appears only after confirmation is requested. Inline confirmation replaces
  the original control in place rather than adding a modal or expanded panel.
- **Icon button:** square control matching standard height, symbolic icon,
  tooltip, and accessible name. Link refresh and copy icons are `14px` and
  centered on both axes.
- **Sidebar navigation:** fixed left sidebar with an icon and text for every
  item. The active page uses `control`, primary text, and a two-pixel accent
  indicator; inactive items use `secondary` on transparent background with
  `control` on hover. A compact **Changed** label accompanies Settings while
  Draft differs from Saved.
- **Text input:** `control`, subtle border, two-pixel accent focus ring;
  invalid input adds an icon and message.
- **Grouped surface:** one flat `surface` container with `8px` radius and subtle
  border. Separators divide related rows; rows do not become nested cards.
- **Status:** compact text and symbolic icon. Reserve accent for the small
  portion that benefits from emphasis. Success, warning, and danger always pair
  color with visible wording or an icon.

Focus is always visible and uses a ring at least `2px` thick with sufficient
contrast against both the control and its surrounding surface.

## Page composition

The sidebar contains Overview, Viewers, and Settings navigation. The current
share state and package version remain compact secondary information rather than
competing with page content.

- **Overview:** one vertical operations dashboard. Stage, source type, and the
  single share action form the lead group; link controls, Viewer health, and
  Active media are flat groups separated below it. Loopback status uses visible
  **This device only** wording beside the Network shortcut. Saved/Active mismatch
  uses warning wording, not color alone.
- **Viewers:** one grouped surface contains compact two-line rows with separators.
  State, IP, and Block occupy the first line. Connected begins below the first
  IP character, while Online/Offline and Lag share a right-aligned status
  column; Block has a separate action column. Rows never switch to a wide table
  at larger window widths.
- **Settings:** Quality, Audio, Network, and Notifications are visible as fully
  expanded groups in one scroll area. A footer outside that scroll area contains
  local status or errors, Revert, and Apply. Field errors sit with their field or
  group. Active/Saved mismatch and its current-share action remain one clear
  operation rather than another settings card.

## Browser Viewer

The square-cornered native video fills the viewport with `contain` fitting and
keeps the browser's native playback, volume, and fullscreen controls. Aercast
adds no custom play button, spinner, transport control, or performance overlay.

The sole non-playing overlay is a centered passive status element with
`pointer-events: none`, `role="status"`, `aria-live="polite"`, and
`aria-atomic="true"`. It uses primary text on the dark background and never
captures input. While playing it is hidden visually and removed from the
accessibility tree; the exact state text is owned by the
[development contract](development.md#viewer-management-and-playback).

## Motion and accessibility preferences

The Host adds no animated transitions. Timed feedback such as **Copied** or an
in-place confirmation changes state immediately, so reduced motion requires no
alternate animation. Native browser controls remain browser-owned.

When the Settings Portal reports higher contrast, strengthen borders and
separators, raise secondary and muted-text contrast, and use at least a `3px`
focus ring. High contrast does not change the fixed dark theme. High contrast
and reduced motion are independent flags; every combination must work.

Every Host control must be reachable in forward and reverse keyboard order,
support its ordinary Enter or Space activation, retain a visible focus ring,
and scroll into view when focused. Text and controls meet WCAG AA. State uses
visible wording or shape in addition to color.

Host accessibility acceptance covers keyboard and visual access only. Iced
0.14 does not provide a basis for claiming AT-SPI screen-reader support here.
The browser Viewer status is separately exposed through the live-region rules
above.
