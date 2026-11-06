# One look for every xchange web page (staff, admin, sign-in, customer)

> designed-by: claude/022a7453 · 2026-11-06 · measured on `main` at `f5a0932`
> Owner, 2026-11-06 between ~08:40 and 09:05 ICT, while preparing to key 4–5 Nov on xc-v3:
> - *"หน้าแรกก็ไม่สวย"* (about the sign-in page);
> - *"ธีม ไม่เข้ากัน"*;
> - *"แก้ไข ธีม ใหม่"*;
> - *"คือให้ เรียบงาน"*;
> - *"แค่ input text ก็ไม่สวยแล้ว"*;
> - *"เอา มาตรฐาน งาน back end"*.
>
> This replaces rule 3 of `xchange-v3-look.md` ("each screen declares its own v2 palette"). The owner now asks for **one** theme,
> held to one standard the way the backend is: one source of values, shared parts, and a gate that fails when a screen goes its own way.

## 1. Why the pages don't match (root cause)

Symptom: the sign-in page, the desk, the admin and the customer app look like four products.

- **Three v2 palettes were carried on purpose.** `xchange-v3-look.md` rule 3 (owner, 2026-10-09: "Carry v2's look as is") gives each
  family its own `:root`-like palette: `devices`, `kiosk` and `admin`. Pages without a look fall back to literals.
- **The customer app's kiosk tokens are scoped away from staff on purpose.** `web/src/screens/customer/_shell/xc-customer-tokens.css:4-5`
  scopes them to `.xc-customer`, "so no staff screen picks them up".
- **Most screen CSS hard-codes colours.** At `f5a0932`, 86 of the 104 `.css` files under `web/src/` contain hex literals. Per area:
  - admin: 41 of 43;
  - desk: 5 of 5;
  - staff: 7 of 7;
  - `shared/ui`: 8 files, including `auth.css` (18 literals; Tahoma, `#183044` on `#eef2f5`) and `shell.css` (25 literals).
- **No shared value layer exists for staff pages.** The look gate `web/src/shared/ui/screen-look.test.ts` rule 2 forbids custom
  properties on `:root`. Nothing replaced them with a class-scoped layer.
- **The inputs are not shared parts.** `TextField.tsx` renders a bare `<div class="xc-field"><label><input>`, and every family
  styles it again. Only `auth.css` (`.xc-auth input`, line 91) and seven customer files do. Staff and admin inputs fall back to
  browser defaults, which is the owner's "แค่ input text ก็ไม่สวยแล้ว".

Root cause: **the system has no single source of look values, and no gate that requires one.** Fixing page by page would leave
the next screen free to hard-code again.

## 2. The standard

### 2.1 One value source

- New file `web/src/shared/ui/xc-theme.css`. It holds every colour, font, radius, shadow and space value, copied from the kiosk the
  owner approved: `api/static/kiosk/xc-tokens.css` (brand wine `#a5003c`, warm neutrals, `XC Noto Sans Thai`).
- The values hang off the class `.xc-theme`, never `:root`, so look-gate rule 2 still holds.
- Semantic names that screens use:
  - `--xc-bg`, `--xc-surface`, `--xc-ink`, `--xc-muted`, `--xc-line`;
  - `--xc-brand`, `--xc-brand-ink`, `--xc-brand-soft`;
  - `--xc-ok`, `--xc-warn`, `--xc-danger`;
  - `--xc-focus-ring`;
  - plus the radius, shadow and space scales from `xc-tokens.css`.
- `xc-customer-tokens.css` stops declaring its own copy and reads `xc-theme.css`, which makes one source (§4, wave 2).

### 2.2 One root

- `AppShell` (`web/src/shared/ui/AppShell.tsx:24`) always renders `xc-theme` on its outer `div`, next to `xc-shell`.
- `AuthLayout` (`web/src/shared/ui/AuthLayout.tsx:11`) does the same on `xc-auth`.
- The shell-less pages (`/c`, `/q`, the queue display) add it on their own root.
- The `look` prop stays for layout only. Its palette declarations are removed as each area is swept (§4).

### 2.3 Shared parts, styled once in `web/src/shared/ui/`

| Part | Look (values from §2.1) |
|---|---|
| Text input, select, textarea (`.xc-field`, plus bare `input`, `select`, `textarea` under `.xc-theme`) | height 44px; padding 10px 12px; `--xc-radius-md`; 1px `--xc-line`; background `--xc-surface`; 16px `--xc-font-sans`. On focus: border `--xc-brand` with a 3px `--xc-focus-ring`. Invalid: border `--xc-danger`. Label 14px/600 `--xc-ink`, 6px above. Error text 13px `--xc-danger` |
| Button (`button`, `.xc-btn`) | min-height 44px; `--xc-radius-md`; 600 weight. Primary: `--xc-brand` fill with white text. Secondary: `--xc-surface` with 1px `--xc-line` and `--xc-brand` text. Disabled: 50% opacity. Focus: the same ring |
| Card (`Card.tsx`, `card.css`) | `--xc-surface`; `--xc-radius-xl`; 1px `--xc-line`; `--xc-shadow-sm`; padding 20px |
| Page | background `--xc-bg`; text `--xc-ink`; font `--xc-font-sans` 15px/1.6 |
| Shell header | `--xc-surface`, bottom 1px `--xc-line`, brand mark in `--xc-brand` |
| Sign-in card | centred and 420px wide above 560px, with the brand mark at the top, the same input and button parts, and background `--xc-bg`. **At 560px or less it fills the screen** (owner: *"ขยายให้ เต็มจอ"*): no card border, radius or shadow; full width and height; 52px inputs and sign-in button; the version line at the bottom |
| Language switch | One segmented control `ไทย | EN` (owner: *"สลับ ภาษา ไทย en"*). The current language is filled `--xc-surface` with `--xc-brand` text; the other is `--xc-muted` on `--xc-gray-75`. It sits at the top right of the sign-in card and in the shell header, beside the menu button on a phone. It replaces the single `EN` toggle of `LangSwitch` in look only: the same i18n call, keys and stored choice |
| Mobile menu (≤760px) | The header keeps the brand mark and a 40px menu button only. The button **expands a panel under the header and collapses it again** (owner: *"ทำเมนู มือ ถือ"*, *"ย่อ ขยาย"*). The panel holds the user and branch, the shift badge, every nav item as a 48px row (the current one in `--xc-brand-soft`), and sign-out. The icon swaps ☰ ↔ ✕, `aria-expanded` follows, and Esc closes it. There is no horizontal scroll at 375px. Above 760px the nav stays inline in the header |

**Font: open question for OL-1.** `api/static/kiosk/xc-tokens.css:6-8` declares `XC Noto Sans Thai` from `fonts/noto-sans-thai-*.woff2`.
At `f5a0932` no such file is in the repo, and `/kiosk/fonts/…` answers 404 on xc-v3. The kiosk therefore falls back to Arial/Tahoma today.
OL-1 ships the font through the web build's own fonts path, and the API's page tests already know one:
`api/tests/fixtures/pages/web/fonts/`. If a woff2 cannot be added in scope, OL-1 keeps the `--xc-font-sans` stack and says so in the PR.

### 2.4 The gate (the "backend standard")

`web/src/shared/ui/one-look.test.ts` fails when:

| # | Case | Required |
|---|---|---|
| G1 | A `.css` file under `web/src/` other than `xc-theme.css` contains a hex colour, `rgb(`, `rgba(`, `hsl(` or a named colour in a colour property | fails, naming the file and line, **unless** line 1 of the file is the marker `/* xc-look: legacy-colours */` |
| G2 | A file carries the marker but has no literal colour left | fails: "remove the marker" (the ratchet only shrinks) |
| G3 | `AppShell` or `AuthLayout` renders without `xc-theme` on its root | fails |
| G4 | `xc-theme.css` declares a custom property on `:root` | fails (look-gate rule 2) |

Exempt:
- `screens/receipt-sample/receipt.css`, the byte-pinned receipt fixture;
- `api/static/kiosk/**`, which is not under `web/src`.

`transparent`, `currentColor`, `inherit` and `none` are not literals.

### 2.5 The approved mock-up

The owner reviewed a live mock-up of §2.3 in the browser pane and answered *"ok"* (2026-11-06 ~09:07 ICT). It shows the sign-in page and the backdated-keying desk page, including the mobile menu, at phone and desktop width.
The file is `docs/design/one-look-mock/index.html`. It uses only the kiosk tokens (`one-look-mock/xc-tokens.css`, copied from `api/static/kiosk/xc-tokens.css` with the `@font-face` lines removed), and no colour literal sits outside them.
**This mock-up is the visual reference for every OL ticket.** Where the mock-up and a table cell above disagree, the mock-up wins, and the ticket says so.

## 3. What does not change

- No text, i18n key, handler, href, data flow, selector, role or id that a test pins. Tests pass unedited, except the look gate
  rule 3 tests the design supersedes, which the ticket author restates.
- No backend and no migration.

## 4. Rollout: waves, one PR each, previewed before merge

Every wave ticket's done line: the gate is green, its area's files carry no marker, and the controller shows the owner screenshots
of its area, before and after, before the merge.

| Wave | Ticket | Files | Done when |
|---|---|---|---|
| 1 (foundation) | OL-1 | `xc-theme.css`, `AppShell.tsx`, `AuthLayout.tsx`, the 8 `shared/ui/*.css`, `one-look.test.ts` (up-front, by the controller), plus one marker line on every other current offender | the sign-in page, shell, inputs, buttons and cards use §2.3; G1–G4 are green |
| 2 | OL-2 desk | `screens/desk/**`, `screens/desk-transaction/**`, `screens/desk-backdated/**` (css only) | no marker left in those folders |
| 2 | OL-3 staff | `screens/staff/**`, `screens/staff-report/**`, `screens/staff-guide/**` | the same |
| 2 | OL-4 admin | `screens/admin/**` (43 css) | the same; may split per sub-folder if it runs long |
| 2 | OL-5 other | `screens/devices`, `documents`, `kiosk-currencies`, `kiosk-queue`, `operations`, and `customer/**` reading `xc-theme.css` | the same; `xc-customer-tokens.css` reduced to aliases |

Wave 2 tickets touch disjoint folders, so they run in parallel. The marker is per file, so no shared list is locked.
