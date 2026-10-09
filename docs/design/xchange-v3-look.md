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
   page root of rule 3: `AppShell`'s `look` prop and the root class it renders. Make no other markup, text or logic change.
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
   - **v2's out-of-root rules.**
     - v2's `body{…}` rule goes on `.xc-<look>` itself, plus `min-height:100vh`.
     - v2's `header{…}`, `main{…}` and `footer{…}` become `.xc-<look> > header` and so on, where `AppShell` renders them as children of the root; otherwise `.xc-<look> header`.
     - v2's `*{box-sizing…}` becomes `.xc-<look>, .xc-<look> *`.
     - The browser's default `body` margin is reset once by the shell: `shell.css` has `body:has(.xc-shell){margin:0}`. This is the only rule outside `.xc-shell`.
   `.xc-devices{--brand:#a90045;…}`, `.xc-kiosk{--xc:#a5003c;…}`, `.xc-admin{--brand:#a5003c;…}`, with the values copied from the table above.
   The shell reads `--brand`, `--ink`, `--muted` and `--line`. A palette that uses other names (`kiosk.css` has `--xc`) also declares those four
   with the same values (`--brand:#a5003c` beside `--xc:#a5003c`), so the header reads one set of names on every page.
4. **No global element rules.** A v2 file's `*`, `body`, `h1`, `h2`, `p`, `a`, `label`, `input`, `select` and `button` rules are scoped under
   that screen's root (`.xc-devices h1{…}`). They then reach only the pages whose v2 page used that file.
   `receipt-sample` and `staff-guide` keep the CSS they already ship.
5. **The header takes each page's v2 header, or its palette when v2 had no header.** This is a v2 fact, read from each page:
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

## Done criterion for every LK ticket

Attach screenshots of the v3 page next to v2's, at the same width (1280 px and 390 px), to the PR. Open v2's page as a static
file from `xchange-v2-source@4d0bdbb` with its stylesheet. The reviewer compares them. Also run
`cd web && npm run typecheck && npm test && npm run build`; all three must be green.

There are no up-front tests: the tickets are look-only, and the owner's 2026-10-07 rule is *"page-only tickets need no prep"*.
Behaviour does not change, and the existing tests must stay green without edits.

## Data migration

None.
