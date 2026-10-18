# Design: Paperback

ABShare's visual design, as built. The decision and its reasons are SPEC D27; this page is the reference for anyone changing or adding a screen. The values here are taken from the code: [`web/app/src/lib/theme/themes.ts`](../web/app/src/lib/theme/themes.ts) (colours, fonts, shapes) and [`web/app/src/app.css`](../web/app/src/app.css) (type, layout and shared components). If they disagree, the code wins and this page should be fixed.

<p align="center">
  <img src="images/web/home-phone.png" alt="Home in the light theme" width="230">
  <img src="images/web/player-phone.png" alt="The player in the light theme" width="230">
  <img src="images/web/library-phone-dark.png" alt="The library in the dark theme" width="230">
</p>

## The idea

A private library should feel like one: warm, quiet and bookish, not like a streaming service. Paperback is paper and ink by day and reading by lamplight at night.

- **The books are the colour.** The interface is paper, ink and hairlines; covers carry the colour. One accent, vermilion, is kept for the play button and small highlights.
- **Rules, not boxes.** Group things with thin lines and space, the way a printed page does, not cards and shadows. Shadows are kept for covers, so books sit on the page like objects.
- **A serif for what you read, a sans for what you use.** Titles, book names and descriptions are in Fraunces; controls, labels and metadata are in Instrument Sans.
- **Calm first.** One main action per screen, in ink. Nothing animates except to explain a change.

## Colour

Colours are design tokens: CSS variables set by the theme at start-up (`applyTheme`), with the Paperback values also in `app.css` so the page is right before the script runs. Never write a colour into a component; use a token.

| Token | Use | Light | Dark (lamplight) |
|---|---|---|---|
| `--bg` | The page | `#f3eee4` paper | `#17140f` brown-black |
| `--bg-raised` | Sheets, inputs, cards | `#fbf8f2` | `#211d17` |
| `--bg-sunken` | Hover, selected rows, wells | `#e8e1d3` | `#2b261e` |
| `--fg` | Text, ink buttons, filled progress | `#1e2430` ink | `#ece4d4` cream |
| `--fg-muted` | Secondary text and icons | `#5d5548` | `#b8ac97` |
| `--line` | Hairlines, outlines, empty tracks | `#d9d0c0` | `#3a3329` |
| `--accent` | Play, links, eyebrows, the current chapter | `#b4442c` vermilion | `#e87a5d` |
| `--accent-fg` | Text on the accent | `#ffffff` | `#1c0d08` |
| `--accent-soft` / `--accent-soft-fg` | Banners and notes | `#f0dcd2` / ink | `#3d261d` / cream |
| `--danger` | Errors and destructive actions | `#a12a1e` | `#f08a7a` |
| `--shadow` | The few raised things (mini player, sheets) | soft, warm | deeper |
| `--shadow-book` | Covers drawn as books | a tight contact shadow and a soft drop | the same, darker |
| `--scrim` | Behind sheets | warm 45 % | black 60 % |

The dark palette is not an inversion: it is warm, the text is cream rather than white, and the accent is lighter so it still reads.

**High contrast** is the second theme, for maximum legibility: black and white, blue (light) or yellow (dark) accent, strong outlines, no shadows, and Instrument Sans for titles too.

**Contrast is tested, not eyeballed.** [`web/app/tests/theme.test.ts`](../web/app/tests/theme.test.ts) checks every theme in both modes (WCAG 2): body text at least 7:1 on every surface, muted text, errors and the accent (used as text) at least 4.5:1, text on the accent and on ink buttons, and that no token can carry CSS syntax. A new token or colour change must keep those tests passing.

**Inverted elements.** The mini player and ink buttons use `--fg` as their background and `--bg` as their text, so they are ink on paper by day and cream on brown-black at night, with no extra tokens.

## Type

| Face | Use | Files |
|---|---|---|
| **Fraunces** (variable, optical size, 300–700, roman and italic) | `h1`, `h2`, `.serif`: page titles, book titles, section headings, descriptions, the chapter you're on (italic), figures in stats | `fraunces.woff2`, `fraunces-italic.woff2` |
| **Instrument Sans** (variable, 400–700) | Everything else: body, buttons, labels, metadata | `instrument-sans.woff2` |

- The fonts are Latin subsets, bundled into the app by Vite and served by ABShare itself (`font-src 'self'`; there is no font CDN). Both are under the SIL Open Font License; the licences sit beside the files in [`web/app/src/lib/fonts/`](../web/app/src/lib/fonts).
- Sizes: body 16 px / 1.5; `h1` 2 rem (Home's greeting up to 2.6 rem, set light with the name in semibold italic); `h2` 1.3 rem; book titles in tiles 1 rem semibold; metadata 0.82–0.9 rem.
- **Eyebrows** (`.eyebrow`): 0.72 rem, semibold, uppercase, tracked 0.14 em, muted, or `.eyebrow.accent` in vermilion. They sit above a title to say what it is: "Currently reading", "Series", "Books by".
- Numbers that change (times, counts) use `font-variant-numeric: tabular-nums` so they don't jitter.

## Layout

- **Phone first.** Pages are `.page`: up to 1040 px wide, centred, with a side gutter of `--gutter` (1.25 rem; 2.5 rem on wide screens) and bottom padding that clears the mini player and tabs. Forms and settings use `.narrow` (520 px).
- **Breakpoints.** 720 px: the book page and Home's hero go two-column. 1024 px: the tab bar becomes a 248 px sidebar (`--side-w`) with the logo, the tabs drop to zero height (`--tabs-h`), and the mini player floats at the bottom of the content area.
- **Shelves** scroll sideways on phones, bleeding to the screen edge, with snap points; grids (`auto-fill`, 140 px columns, 160 px from 1024 px) are used where a whole list is shown.
- Touch targets are at least 44 × 44 px.

## Components

Shared classes live in `app.css`; anything used by one screen stays in that screen's `<style>`.

| Component | How it looks | Where |
|---|---|---|
| `.btn` | A pill in ink (`--fg` on `--bg`): the one main action | Resume, Play, Save |
| `.btn.accent` | Vermilion pill | Rare: reserved for play-like actions |
| `.btn.soft` | Transparent with a hairline outline: secondary actions | Download, Cancel, sheet menus |
| `.btn.danger` | Outlined in `--danger` | Remove, Clear |
| `.icon-btn` | A 44 px round, borderless button; `--bg-sunken` on hover | Back, More, Search |
| `.chip` / `.chips` | Small outlined pills; pressed or current ones fill with ink | Genres, speeds, sleep times, admin tabs |
| `.segmented` | Radio buttons as one outlined pill; the chosen one fills with ink | Light/dark, the library's status filter |
| `label.field`, `.input`, `.select` | 46 px fields on `--bg-raised` with a hairline; the border turns ink on focus. Selects draw their own chevron in the text colour | Forms, Sort |
| `.rules` | A list whose rows are separated by hairlines | Settings, recently added |
| `.card` | `--bg-raised` with a hairline and no shadow: use sparingly | Code boxes, a few admin panels |
| **Cover** (`Cover.svelte`) | Square artwork. With `book`, it is drawn as a book: a shaded spine strip on the left, 3 px corners on the spine side, and `--shadow-book`. Without artwork, a plain jacket in a muted colour with the title in Fraunces | Tiles, Home's hero, the player, the book page |
| **BookTile** | Cover as a book, a 2 px progress line (ink on `--line`), the title in Fraunces (two lines at most), one muted line: time left, author or episodes | Shelves and grids |
| **Mini player** | An inverted ink card floating above the tabs, with a 3 px accent progress line along its bottom edge | Everywhere a book is open |
| **Player** | Full screen on `--bg`: the cover as a book, the title in Fraunces, people in muted sans, the chapter in vermilion italic; a hairline scrubber that fills with ink and has a vermilion thumb ringed in the page colour; a 78 px vermilion play button; Speed, Sleep, Chapters and Bookmark on a hairline-topped row | `PlayerView.svelte` |
| **Sheet** (`Sheet.svelte`) | A bottom sheet on phones (centred dialog on wide screens) on `--bg-raised`, with a grab handle, a Fraunces title over a hairline and `--scrim` behind | Menus, download, speed, sleep |
| **Stats** | Figures in Fraunces under uppercase labels, divided by hairlines rather than boxed | History, admin overview, jobs |
| **Contents list** | Name in Fraunces, dotted leader, count on the right | Genres |

## Motion

Little, quick and purposeful (`--ease`: `cubic-bezier(0.2, 0.7, 0.2, 1)`): pages rise 6 px as they appear, the player and sheets slide up, buttons dip to 97 % when pressed, tiles lift 2 px on hover. Everything is switched off under `prefers-reduced-motion`.

## Writing

British English, sentence case, plain words. Labels say what happens ("Resume from 29:36", "Download"), not what the control is. Errors say what to do next.

## Adding or changing a screen

1. Start from the shared pieces: `.page`, an `.eyebrow` and `h1`, `.btn` for the one main action, `.btn.soft` for the rest, `.rules` for lists, `Cover` with `book` for any book.
2. Use tokens only. If a new colour role is truly needed, add it to every theme in `themes.ts` (and the defaults in `app.css`) and extend the contrast tests.
3. Check it in light, dark and High contrast, at phone width and at 1280 px, and with the keyboard (focus rings use `--accent`).
4. The strict CSP still applies: no inline scripts, no `{@html}`, styles only from the bundle (dynamic values through `style:` directives), and fonts only from the app.
5. Retake any README screenshot the change affects, from the demo library ([`BUILDING.md`](BUILDING.md#a-local-demo)).

## History

Paperback was chosen on 2026-10-18 from three mocked-up directions: *Cover-led* (dark, immersive, colour taken from the cover), *Paperback* and *Studio* (dense, near-black with a lime accent). The owner chose Paperback and asked for a dark mode, which became lamplight. It replaced the Material 3 and Classic themes of 0.9.0.
