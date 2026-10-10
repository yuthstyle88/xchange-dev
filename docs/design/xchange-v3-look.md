# v3 web look: v2's stylesheets, carried as they are (#627)

> designed-by: claude/b060e9 · 2026-10-09 · measured on `main` at `af865a3`, v2 at `xchange-v2-source@4d0bdbb`
> The owner decided this on 2026-10-09 (#627 issuecomment-6074938888): **"Carry v2's look as is"**, keeping v2's colours, and in this session
> *"หน้าตา เอาตามของเดิม"*. This section covers the look only, with no logic, no data and no text changes.

## Why the pages are bare (#627 §2)

v2 styled each page with the stylesheet that page linked, or with an inline `<style>`. v3 ported the markup and the behaviour
but not the CSS. No design named the stylesheets, so no screen ticket asked for them.

## v2's sources (measured at `4d0bdbb`)

v2 has **three palettes**, one per family of pages, and only some pages have a header.

| v2 page | Its look | `:root` palette | Header in v2 |
|---|---|---|---|
| `src/devices-public/devices.html` | links `/devices.css` | `--ink:#25232b;--muted:#706a77;--brand:#a90045;--line:#e5e1e7` | yes: `<header>` + `<nav>` (`devices.css` `header{…}`, `.brand*`, `nav`) |
| `src/devices-public/kiosk-currencies.html` | links `/kiosk.css` | `--xc:#a5003c;--ink:#19191d;--muted:#67646c;--line:#e7e2e6;--soft:#fff2f6` | **none**: `<body><main>` |
| `src/devices-public/kiosk-queue.html` | links `/queue-panel.css` | none: `queue-panel.css` uses literal values and no `var(--…)` | **none**: `<body><main class="queue-page">` |
| the admin page, `src/online/lib/admin-html.ts` | its inline `<style>`, plus `admin-workspace.css` (injected by `admin-workspace.js`) | `--brand:#a5003c;--ink:#17181b;--muted:#676a73;--line:#e5e6ea;--bg:#f5f6f8;--soft:#fbf0f4;--green:#187151` | yes: `<aside class="sidebar">`, `<nav>`, `<header>` in the inline `<style>` |
| `src/devices-public/staff-guide.html` | its own `<style>` | — | — (v3 already ships `guide.css`) |
| `src/devices-public/login.html` | its own `<style>` | — | — (#531, `auth.css`) |

## Rules

1. **Port, don't redesign.** Copy each v2 rule as it is: the same values, colours, sizes and media queries.
2. **Selectors follow v3's markup.** Where v3 renamed a class (v2 `.queue-tabs` is v3 `.xc-queue-tabs`), change the selector and leave the markup.
   Add a `class` to a v3 element only when it has none and a v2 rule needs one. The one other allowed markup change is the
   page root of rule 3: `AppShell`'s `look` prop and the root class it renders.
   - **Amended 2026-10-10:** where CSS alone cannot reproduce v2's layout, a look ticket may also move markup structurally.
     That covers reordering elements, adding or removing a wrapper, and moving content into the shared `Card` parts.
     - This applies only when the ticket's §2 shows the v2 structure the move copies, with file:line.
     - Owner, 2026-10-10: *"ดีไชน์ ยังไม่เหมือน V2"*. Look follows v2 exactly. Lane controller claude/608dff ruled option A.
   - **Still forbidden in a look ticket:**
     - changes to visible text or i18n keys;
     - changes to handlers, hrefs or data flow;
     - changes to any selector, role, id or text that a test pins. Tests must pass unedited.
   - Make no other logic change.
3. **Each screen declares its own v2 palette on one page root, set by `AppShell`. Nothing is declared on `:root`.**
   - **The element.** `AppShell` (`web/src/shared/ui/AppShell.tsx`, LK-1) gains an optional prop
     `look?: 'devices' | 'kiosk' | 'admin'` [required: name, type, optional]. It renders its outermost element as
     `<div class="xc-shell xc-<look>">`, or `<div class="xc-shell">` without a look. That element is the ancestor of the header,
     the main and the footer. Optional means a screen whose ticket has not landed still renders, with the shell's fallbacks.
   - **Who passes it** (in files those tickets already own):
     - LK-2: `look="devices"` in `web/src/screens/devices/DevicesScreen.tsx`.
     - LK-4: `look="kiosk"` in `web/src/screens/kiosk-currencies/main.tsx`, added to LK-4's scope.
     - LK-5: `look="admin"` in `web/src/screens/admin/AdminShell.tsx`.
     - kiosk-queue passes **no look**, so the shell's fallbacks apply (rule 5).
   - **What the root carries (amended 2026-10-09, lane controller ruling on #633 issuecomment-6078576748).** `.xc-<look>` itself
     carries **only** the palette (custom properties) and v2's `body`-equivalent values: `background`, `color`, `font-family`,
     `font-size`, `line-height`, `min-height:100vh` [required]. Nothing else is declared on the root.
   - **Why.** The root is also the ancestor of the shell's header and footer. A page rule written `.xc-<look> button` therefore matches
     the shell's account bar, language switch and nav too, and ties with `shell.css`. Measured on PR 643 at `312b5a7`: the logout
     button rendered white on white with `min-height:68px`.
   - **The content root.** Page content sits in `.xc-shell-main` (the shell's `<main>`), or in `.xc-content` when the screen passes
     `ownsMain` (`AppShell.tsx`: admin and receipt-sample do). Below, `<content>` means whichever of the two that screen renders.
   - **v2's out-of-root rules.**
     - v2's `body{…}` rule: its `body`-equivalent values go on `.xc-<look>` (above). Any other declaration in it goes on `.xc-<look> <content>`.
     - v2's `main{…}` becomes `.xc-<look> <content>`. v2's `footer{…}` becomes `.xc-<look> <content> footer` when the page has its own footer; the shell's footer keeps `shell.css`.
     - v2's `header{…}`: see rule 5. It targets the shell's own class `.xc-<look> .xc-shell-header`, never the element `header`.
     - v2's `*{box-sizing…}` becomes `.xc-<look>, .xc-<look> *` (it is harmless on the shell).
     - The browser's default `body` margin is reset once by the shell: `shell.css` has `body:has(.xc-shell){margin:0}`. This is the only rule outside `.xc-shell`.
   `.xc-devices{--brand:#a90045;…}`, `.xc-kiosk{--xc:#a5003c;…}`, `.xc-admin{--brand:#a5003c;…}`, with the values copied from the table above.
   The shell reads `--brand`, `--ink`, `--muted` and `--line`. A palette that uses other names (`kiosk.css` has `--xc`) also declares those four
   with the same values (`--brand:#a5003c` beside `--xc:#a5003c`), so the header reads one set of names on every page.
4. **Element and page-class rules are scoped to the content root, never to the page root** [required] (amended 2026-10-09,
   #633 issuecomment-6078576748). A v2 file's `h1`, `h2`, `p`, `a`, `label`, `input`, `select`, `button`, `table`… rules **and** its page
   classes (`.card`, `.primary`, `.sidebar`, …) become `.xc-<look> <content> <selector>`, e.g. `.xc-kiosk .xc-shell-main h1{…}` or
   `.xc-admin .xc-content button{…}`. Never `.xc-<look> button`. They then reach only that page's content, not the shell.
   Every element rule that v2's page uses is either carried this way or listed in the PR as left out, with the reason (rule 1).
   `receipt-sample` and `staff-guide` keep the CSS they already ship.
5. **The header takes each page's v2 header, or its palette when v2 had no header.** This is a v2 fact, read from each page.
   A rule meant for the shell's header names the shell's own classes (`.xc-<look> .xc-shell-header`, `.xc-<look> .xc-shell-nav`,
   `.xc-<look> .xc-shell-title`), never an element under `.xc-<look>`:
   - devices: `devices.css`'s `header`, `.brand*` and `nav` rules, in the devices palette;
   - admin: `admin-html.ts`'s header and sidebar rules, in the admin palette;
   - kiosk-currencies and kiosk-queue: v2 shows no header. v3's `AppShell` header stays, with the shell's layout and the page's palette
     (the kiosk palette on kiosk-currencies; on kiosk-queue the shell's fallbacks, which are the devices palette's values). This is a
     known difference, because v3 always has the shell.
6. **Import pattern.** The component or the entry imports its own CSS (`import './x.css'`), as `screens/admin/main.tsx:7` does.

## Map

| v3 | v2 source ported | v3 file | Ticket |
|---|---|---|---|
| Shell: `AppShell` header, brand, nav, `AccountBar`, `LangSwitch`, footer and lock, **its own elements only** (`.xc-shell-*` classes, no element selectors) | the layout of `devices.css`'s `header{…}`, `.brand*` and `nav` rules (the only v2 header that matches `AppShell`'s), with colours read through `var(--brand, #a90045)` etc. | `web/src/shared/ui/shell.css`, imported by `AppShell.tsx` | LK-1 #630 |
| `screens/devices/` | all of `devices.css`: its palette on `.xc-devices` and every rule scoped under it, the element rules included | `web/src/screens/devices/devices.css` | LK-2 #631 |
| shared `QueuePanel`/`QueueCard` and `screens/kiosk-queue/` | `queue-panel.css` (literal values, no palette) | `web/src/shared/ui/queue-panel.css`, imported by `QueuePanel.tsx` | LK-3 #632 |
| shared `QueueCard`, the floating queue notice on every screen | `account-bar.js`'s `.xc-queue*` rules + `admin-workspace.css`'s `.xc-workspace .xc-queue*` overrides (literal values; admin overrides scoped `body:has(.xc-admin)`, web design §12.18.12 QF6) | `web/src/shared/ui/queue-card.css`, imported by `QueueCard.tsx` | #786 |
| `screens/kiosk-currencies/` | the `kiosk.css` rules that v2's `kiosk-currencies.html` markup uses, with `kiosk.css`'s palette on `.xc-kiosk` and its element rules scoped under it | `web/src/screens/kiosk-currencies/kiosk-currencies.css` | LK-4 #633 |
| `screens/admin/` (sidebar, header, workspace) | **`admin-html.ts`'s inline `<style>`** (palette on `.xc-admin`, element rules scoped under it) **and** `admin-workspace.css` | `web/src/screens/admin/admin.css` (existing; v3-only rules stay where no v2 rule covers them) | LK-5 #634 |
| the per-screen gate (above) | — | `web/src/shared/ui/screen-look.test.ts` | LK-6 (after LK-2…LK-5) |
| `/login` | `login.html`'s `<style>` | #531 (`AuthLayout`, `auth.css`) | — |
| `receipt-sample`, `staff-guide` | already have their CSS | — | — |

## Gate

**LK-1 ships `web/src/shared/ui/shell-look.test.ts`.** It checks two things:
1. `AppShell.tsx` imports `./shell.css`.
2. Every page entry under `web/pages/*.html` reaches a module that imports `AppShell` or `AuthLayout`. The test reads each entry's `<script src>` and that module's source.

**LK-6 ships `web/src/shared/ui/screen-look.test.ts` after LK-2…LK-5 merge.** It is a separate file, so no LK ticket edits another's. It checks two things:
1. Every v3 screen whose v2 page had a stylesheet or a `<style>` ships its CSS. The list is in the test:

   | Screen | Required file |
   |---|---|
   | devices | `screens/devices/devices.css` |
   | kiosk-queue | `shared/ui/queue-panel.css` |
   | kiosk-currencies | `screens/kiosk-currencies/kiosk-currencies.css` |
   | admin | `screens/admin/admin.css` |
   | staff-guide | `screens/staff-guide/guide.css` |
   | receipt-sample | `screens/receipt-sample/receipt.css` |
   | login | `shared/ui/auth.css` |

2. No `.css` file under `web/src/` declares custom properties on `:root` (rule 3).
3. No selector in a screen's CSS is `.xc-<look>` followed directly by an element or a page class (`.xc-admin button`, `.xc-kiosk .card`).
   After `.xc-<look>`, the next part must be `*`, `.xc-shell-main`, `.xc-content` or a `.xc-shell-*` class (rules 3–5).

## Done criterion for every LK ticket

Attach screenshots of the v3 page next to v2's, at the same width (1280 px and 390 px), to the PR. Open v2's page as a static
file from `xchange-v2-source@4d0bdbb` with its stylesheet. The reviewer compares them. Also run
`cd web && npm run typecheck && npm test && npm run build`; all three must be green.

There are no up-front tests: the tickets are look-only, and the owner's 2026-10-07 rule is *"page-only tickets need no prep"*.
Behaviour does not change, and the existing tests must stay green without edits.

## Data migration

None.

## Flutter kiosk app (`app/`): v2's kiosk look

> designed-by: claude/5edf7bb5-subagent for lane controller claude/5edf7bb5 · 2026-10-10 · measured on `main` at `f4d6207`, v2 at `xchange-v2-source@4d0bdbb`
> The owner, 2026-10-10, in the controller's session (claude/5edf7bb5): *"ทำ แอป kiosk Flutter ให้เหมือน v2"* ("make the Flutter kiosk app look like v2").

The rules above are for the web pages. This section is for the Flutter kiosk in `app/`, which replaced v2's `kiosk.html`
(`docs/design/xchange-v3.md` §8). **The logic follows the Blueprint and §8. The look follows v2's kiosk exactly**: the
`kiosk.css` rules (`src/devices-public/kiosk.css`) that v2's `kiosk.html` markup uses.

**Rules.**
1. **Port, don't redesign.** Each v2 value (colour, size, weight, spacing, radius, shadow) becomes the same value in Flutter, in logical pixels.
   Where `kiosk.css` sets the same property more than once, the last rule that matches wins, as in the browser.
   - Example: `h1` is `42px` at `kiosk.css:4` and `clamp(32px,4vw,48px)` at `:18`, so `:18` applies.
2. **Only the look changes.** A look ticket changes no visible text, l10n key, `Key(...)`, handler, provider or controller.
   Every existing test passes unedited. That includes the ones that pin today's look:
   - `app/test/shared/kiosk_text_spacing_test.dart`: button labels 24/22, the dropdown value 20, no `letterSpacing`/`height`;
   - `app/test/shared/kiosk_theme_font_chain_test.dart` and `every_text_uses_the_bundled_fonts_test.dart`: the font family and its nine fallbacks;
   - `app/test/features/kiosk/kiosk_page_layout_test.dart`: the error line sits above the step.
   Where v2 differs from a pinned value, the pinned value stays. The ticket lists the difference as out of scope.
3. **The font family is not ported.** v2's `Arial,Tahoma,…` is replaced by the bundled Noto chain (#6, #30), which the tests pin.
4. **Where a value lives.**
   - Colours shared by every screen go in `KioskColors` (`app/lib/shared/theme/colors.dart`).
   - v2's `body`, `input` and `select` values go in `kioskTheme` (`app/lib/shared/theme/theme.dart`).
   - Page-level values go in the page file. Widgets in `kiosk_widgets.dart` are changed only by a ticket that owns that file.

**Map.**

| v2 (`kiosk.css` @ `4d0bdbb`) | What it sets | v3 file | Ticket |
|---|---|---|---|
| `:3` `:root{--xc;--ink;--muted;--line;--soft}` | the palette | `app/lib/shared/theme/colors.dart` (`KioskColors.brand`/`ink`/`muted`/`line`/`soft`, already equal) | — |
| `:16` `body{background:…#f7f7f9;font-size:18px;color:#212126}` | page colour, body text size and colour | `colors.dart` (`stage`, a body-text colour), `theme.dart` (`textTheme`) | LF-1 #934 |
| `:4` `select,input{border-color:var(--line);background:#fbfafb;border-radius:10px}` | the input box | `theme.dart` (`inputDecorationTheme`) | LF-1 #934 |
| `:16` the `radial-gradient(… #f7e6ee …)` layer | the page's top-right glow | `app/lib/features/kiosk/presentation/pages/kiosk_page.dart` (the `Scaffold` body) | LF-2 #935 |
| `:4`, `:17` `header`, `.kiosk-brand*`, `header p`; `:29`, `:31` `.kiosk-lang*` | header, logo, brand line, branch, language picker | `kiosk_page.dart` (`_Header`) | LF-2 #935 |
| `:18` `main`, `.step`, `h1`, `h1 span`; `:4` `#error:not(:empty)`; `:4`, `:24` `footer` | the frame around the four screens | `kiosk_page.dart` (`KioskPage.build`) | LF-2 #935 |
| `:4`, `:24` `#confirm`, `#confirm h2`, `#confirm>p`; `:1`, `:19` `.directions` | the review card | `app/lib/features/kiosk/presentation/pages/quote_page.dart` | LF-3 #937 |
| `:4`, `:24` `#done`, `#done h2`, `#done>p`, `.success`; `:11`, `:24` `.telegram-card*`; `:24` `#customer-receipt` | the sent card and the receipt | `app/lib/features/kiosk/presentation/pages/waiting_page.dart`, `receipt_page.dart` | LF-4 #938 |
| `:4`, `:19`–`:23`, `:27` `.directions button`, `form#form`, `.currency-pane`, `.currency-shortcuts`, `#rates`, `#amount`, `#receive`, `#estimate`, `#review`, `#payment-choice` | the select screen | `select_page.dart` | after RF-5 (#877), which reserves the file |
| `:1`, `:4`, `:9`, `:11`, `:24` `button`, `.primary`, `.quote`, `.order-identity`, `.order-progress`, `#queue` | shared buttons, quote rows, identity block, progress, queue number | `app/lib/features/kiosk/presentation/widgets/kiosk_widgets.dart` | after RF-5 (#877), which reserves the file |
| `:5`, `:6`, `:25`, `:29` `@media(max-width:…)` / `(max-height:850px)` | narrow and short screens | the files above | not yet ticketed |

Each LF ticket's §2 table quotes the v2 rule (`kiosk.css:N`) beside the v3 `file:line@sha` it changes.
LF-1…LF-4 own disjoint files, so they can run in parallel.

**Done criterion for every LF ticket.** The following run in `app/` and are all green:
- `flutter analyze --fatal-infos`
- `dart format --output=none --set-exit-if-changed .`
- `flutter test`, with the pinning tests unedited.

The PR also attaches screenshots at 1280×800 of the touched screen beside v2's `kiosk.html`, opened as a static file from `xchange-v2-source@4d0bdbb`.
There are no up-front tests: the tickets are look-only (the owner's 2026-10-07 rule, *"page-only tickets need no prep"*).
