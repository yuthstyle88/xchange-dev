# Reference: the shop's live customer kiosk

**Where these files come from.** The shop's live kiosk at `https://xc-th.work/kiosk`, captured on 2026-11-05. The owner saw these files run against our own API through a local preview and approved the look: *"Yes, start it"*. These are byte copies of the nine files as served that day:

| File | sha256 |
|---|---|
| `kiosk.html` | `d53040d2298fa98e97db5984faa05fd660b212a961c23b5612c573224b11e871` |
| `kiosk.js` | `6d4c91464b90c371cd583e5a3723d1bac1a74d0b7f751e89368abc6d73a02a90` |
| `kiosk-attract.js` | `c74457fa17e9f9aa8883335796ae6fd491c9be8358182d2162dd746eb2796a9c` |
| `kiosk-attract.css` | `b759b5e55a2918bff845bea6421ac5726c712a1ff2c59ed26b54d684e883b231` |
| `kiosk.css` | `eac4cb51808b7886d5edb7df65c1871c65a2487ad4098e6fa14d0cd4859bf019` |
| `kiosk-fonts.css` | `01f1f5d479ca350ca7cb01bcc3a3be64301f74a331fbd6a9164ddca8d88a69d9` |
| `kiosk-i18n.js` | `339a28f6f5c00d79736ed64de0d2aac1dc1805dbc03384b73d10bda127975b19` |
| `kiosk-language-extras.js` | `9f082081d9e54468b0934cdd331af3642dfe9fd6a387a98a38f01043109aa81d` |
| `xc-tokens.css` | `d78b31846eca81758b476abc34dd3463d7d22e65b7d70831c625ae8bbc213eef` |

**What they are for:** the look and the behaviour of our `/kiosk`: the 36 s attract loop, 16 languages and the mobile layout. They are a reference, not source:
- the served copy is `api/static/kiosk/`, compiled into the server (`api/src/presentation/pages.rs`);
- nothing builds from this folder and no test reads it.

**What the served copy changes** (each change carries an `xchange:` comment in `api/static/kiosk/`; the owner's words are quoted there):
- `kiosk.js` `loadRates()` reads only our `POST /api/kiosk` `rates` answer. We have no `GET /api/rate-board` and will not add one (`docs/design/blueprint/rate-display.md:314`).
- With no `?branch=`, Step 1 has a branch control beside the language control ("มี3 สเต็ป"). It reads `GET /api/public/host` and `GET /api/public/orgs/{org}/branches`. A paired branch tablet takes its own branch from `GET /api/branch-tablet/me`. An inactivity reset clears a picked branch and keeps a `?branch=` one.
- The amount area (final, "ผมว่ามัน ok"): the field at full width (code on the left, number on the right), one row of quick amounts (`+100 · +500 · +1,000 · +5,000`; a tap on an empty field sets, every later tap adds), the complete keypad, then the estimate. No browser spinner or validation bubble; `inputmode="none"`.
- Phones (≤ 600 px): Step 1 is drawn as their attract demo card (`kiosk-xchange.css`).
- The tiles: the branch's popular SHOW currencies first (server-side, from its trading history), up to the owner-set count; ASK currencies are listed by the server with `display: "ASK"`, found by search and word "Please contact staff"; HIDE never appears.
- The payment method is preselected (the owner-set default, else Cash, else the side's first enabled method).
- The owner-set values (chips, tiles, idle reset, review hold, refresh, popularity window, default payment method) come from the server (`api/src/application/kiosk_screen.rs`), never from constants in `kiosk.js`.
- `kiosk-attract.js`'s step-1 and step-2 scenes show this screen.
- `kiosk-language-extras.js` adds the three texts of the branch control in all 16 languages.
- `kiosk.html` drops the Cloudflare scripts their host injected, and adds a quieter style for the server's version line.

Their `api()` already sends `X-Xchange-Request: 1`, which our `/api/kiosk` requires (`api/src/presentation/extractors/origin.rs`).

Do not edit these files; a newer capture is a new copy with a new date.
