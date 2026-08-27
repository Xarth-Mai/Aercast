# UI design

This document owns Aercast's visual language, visual tokens, component
appearance, and accessibility rules; it does not own product state, page
business, development progress, or backend behavior.

## Visual language

Aercast follows a **dense desktop utility** visual language inspired by LACT's
information architecture and Linear's visual treatment. It does not attempt to
reproduce Libadwaita or any browser-based design system.

The layout uses a fixed sidebar for navigation and a centered content area with
a reasonable `max_width`. The interface is dark, layered through subtle surface
contrast rather than visible borders, and uses one restrained accent color. All
hierarchy is conveyed through spacing, typography weight, and surface luminance.

Use the compositor's ordinary system title bar. Do not draw a custom header
bar. Do not use gradients, glow, large fields of brand color, glass effects, or
stacked shadows.

## Brand image and icons

- `assets/aercast-icon.png` is the canonical application, tray, and Viewer
  favicon image. Its RGBA source uses a tight square canvas so the visible mark
  fills the available surface when scaled; do not redraw, recolor, duplicate,
  or add a second brand asset.
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
| `bg` | `#0B0D10` | window background |
| `surface-1` | `#11151A` | sidebar, cards, grouped content |
| `surface-2` | `#171C22` | inputs, neutral buttons, hover surfaces |
| `surface-3` | `#20262E` | raised/hover states |
| `border` | `white 7%` | subtle boundaries (not for hierarchy) |
| `text` | `#F4F6F8` | primary text |
| `text-secondary` | `#929BA7` | labels, descriptions, inactive nav |
| `text-muted` | `#5C646E` | unavailable and disabled controls |
| `danger` | `#E54D4D` | destructive actions |

### Accent derivation

1. Prefer a literal `accent_bg_color #RRGGBB` from the active GTK 4 user
   stylesheet or one local stylesheet imported directly by it. This matches
   desktop themes whose Portal backend exposes only a generic color family.
2. Otherwise read `org.freedesktop.appearance` / `accent-color` as one sRGB
   `(r, g, b)` tuple. Every channel must be finite and within `[0, 1]`; treat an
   invalid value as absent.
3. Use the valid value as `accent-base`, or the cool blue default `#4C9AFF` when
   absent.
4. `accent-bg` equals `accent-base`.
5. `accent-fg` is whichever of `#ffffff` and `#1e1e1e` has the higher WCAG
   contrast ratio against `accent-bg`. If neither reaches `4.5:1`, use
   `#000000`.
6. `accent-standalone` begins at `accent-base` and moves only toward white until
   it reaches at least `4.5:1` against `bg`.
7. `accent-subtle` is a 15% alpha composite of `accent-base` over `surface-1`.

Color math operates in sRGB with WCAG relative luminance and contrast formulas.
All text and interactive states must meet WCAG AA. Error, warning, success,
focus, and selection always include text, iconography, border, or shape in
addition to color.

`accent-bg` is reserved for the one primary action in a view and compact
controls such as a checked box. Navigation and option selection use
`accent-subtle` with an `accent-standalone` border and normal text. Focus rings
and small active-status markers use `accent-standalone`; ordinary content
surfaces remain neutral even when the Portal supplies a saturated accent.

## Geometry

- All layout spacing, padding, gaps, control dimensions, icon dimensions, and
  corner radii follow a `4px` grid. Font sizes and one-to-three-pixel strokes
  are optical and accessibility exceptions.
- Spacing scale: `4 / 8 / 12 / 16 / 20 / 24` logical pixels.
- Interactive control height: `34` logical pixels.
- Control corner radius: `8` logical pixels.
- Card and section corner radius: `10` logical pixels.
- Sidebar width: `220` logical pixels.
- Content max width: `700` logical pixels.
- Section gap: `16–24` logical pixels.
- The centered main Share action uses a `16px` pill radius.
- Borders: one logical pixel normally; never create hierarchy with multiple
  nested outlines. Prefer surface contrast over visible borders.
- Shadows: at most one subtle compositor-independent shadow for an elevated
  transient surface; ordinary grouped content uses borders and luminance only.

The main window content must fit `920×520` logical pixels without horizontal
overflow. It is not resizable, but layout must tolerate normal font metrics and
the compositor's server-side decoration size.

## Typography

Use the first available system font from `Adwaita Sans`, `Cantarell`,
`Noto Sans`, then the toolkit sans-serif fallback. Do not bundle a font.

| Role | Size | Treatment |
| --- | ---: | --- |
| Body and controls | about `14px` | regular |
| Supporting text | `12–13px` | secondary or muted, not low-contrast |
| Page title | about `16px` | bold, secondary color, uppercase not used |

Use sentence case. Prefer short labels over reduced font size. Numeric telemetry
may use tabular figures when the active system font provides them.

## Components

- **Primary button:** `accent-bg`, `accent-fg`, and bold text. The Share page's
  Start, Cancel, Stop, and stopping states retain this treatment; never use it
  to indicate navigation or option selection.
- **Selected button:** `accent-subtle`, normal text, and a one-pixel
  `accent-standalone` border.
- **Neutral button:** `surface-2`, subtle border, brighter hover surface.
- **Destructive button:** `danger` background with explicit destructive wording;
  it appears only after confirmation is requested.
- **Icon button:** square control matching standard height, symbolic icon,
  tooltip, and accessible name. Link refresh and copy icons are `14px` and
  centered on both axes.
- **Close button:** a compact neutral button with a centered symbolic icon in
  the content area's top-right; it uses the same hide-window behavior as the
  compositor close action.
- **Sidebar navigation:** fixed left sidebar with text items; the active page
  uses `surface-2`, primary text, and a two-pixel accent indicator; inactive
  items use `text-secondary` on transparent background with `surface-2` on
  hover.
- **Text input:** `surface-2`, subtle border, two-pixel accent focus ring;
  invalid input adds an icon and message.
- **Card:** one grouped `surface-1` container with `10px` radius and subtle
  border. Used for status areas, viewer lists, and settings sections. Do not
  wrap every row in another card.
- **Status:** compact text and symbolic icon. Reserve accent for the small
  portion that benefits from emphasis.
- The Viewer has no custom controls, overlay, or visible status text. Its
  square-cornered video fills the browser viewport with `contain` fitting and
  uses only the browser's native playback, volume, and fullscreen controls.

Focus is always visible and uses a ring at least `2px` thick with sufficient
contrast against both the control and its surrounding surface.

## Page composition

The Host UI uses a fixed sidebar for navigation (Share, Viewers, Settings) with
a status indicator at the bottom-left, the Cargo package version at the
bottom-right, and a scrollable content area. Scrollbars stay hidden while
wheel, touchpad, and keyboard scrolling remain available. Each content page
uses `24px` outer padding, a section title in `text-secondary`, and `surface-1`
card containers.

The Share page groups status, approved source, link, and confirmations in one
card while keeping its share action centered at the bottom. The Viewers page
places its online-count capsule immediately after the title and uses one card
with separators rather than per-row cards. Settings keeps existing application
rows visible while refreshing, then replaces them with the completed scan; it
uses one card for each of Quality, Audio, Network, and Notifications, and
controls within a section do not add another container outline.

## Motion and accessibility preferences

Normal transitions last `120–180ms` and use simple opacity or position changes.
When the Settings Portal reports reduced motion, remove non-essential
transitions and animate only feedback required to understand a state change.

When the Portal reports higher contrast, strengthen borders and separators,
raise muted-text contrast, and use at least a `3px` focus ring. High contrast
does not change the fixed dark theme. High contrast and reduced motion are
independent flags; every combination must work.
