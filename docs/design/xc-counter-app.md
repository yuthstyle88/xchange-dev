# XC Counter: a Mac app that wraps the staff desk and drives the counter's devices

> designed-by: claude/022a7453 · 2026-11-06 · measured on `main` at `f5a0932`
> Owner, 2026-11-06 (in order):
> - *"รูปพาสปอร์ต เชื่อม เครื่อง สแกนเลยได้ไหม"*, *"ให้เปิดเครื่อง สแกน เลย"*, *"USB"*, *"Mac เปลี่ยน"*;
> - *"kiosk ฝั่งพนัก งาน"*;
> - *"ให้ มี config ว่า รอง รับลิ้นได้"*, *"ใช่ ลิ้นชักเงิน"*;
> - *"มันต้อง ครอบ ด้วย แอป"*, *"มันถึงจะคุมลิ้น ชักได้ หรือ ผ่านเว็บได้"*;
> - then, picking option A over "web + the background agent": *"แอปครอบ"*.
>
> This replaces the first version of this file (a scan job through the server). The owner's approval of that version's migration
> ("อนุมัติ migration", 09:23 ICT) covered §4 of that version. That §4 is withdrawn; §5 below is a new migration and needs its own approval.

## 1. Why an app

- **A browser cannot drive the counter's devices.**
  - The cash drawer opens through the receipt printer it is wired to.
  - The scanner is USB.
  - No web API reaches CUPS, a USB scanner or ImageCaptureCore, and Safari has no WebUSB.
- **Today a background program does the drawer.**
  - `api/crates/xc-device-agent` pairs one machine to its branch and claims `device_jobs(kind='DRAWER_REQUEST')` from the server
    (`client.rs`, `runner.rs`). It kicks the drawer over CUPS or TCP (`cups_transport.rs`, `tcp_transport.rs`).
  - **One agent per branch**: `device_agents_one_active_per_branch_idx` (`20261008120000_device_agents.sql`).
  - So a branch cannot have a scanner-only staff kiosk and a drawer counter side by side.
- **Nothing scans.** The desk only takes an attached file (`web/src/screens/desk/ocr/PassportOcr.tsx:171-184`).

The app is **one install per counter**. It is the counter's identity. It shows the staff desk full-screen, so staff cannot browse away.
It owns the counter's devices: the drawer, and the scanner when the owner says the counter has one.

## 2. What the app is

**A thin shell, never a second UI (owner: *"เราใช้ แอฟ ครอบแล้วมันง่าย"*, *"แต่pos ผมทำ แอป เพียว เลย มันเลยยาก"*).**
`pos108-terminal` rebuilt every screen natively in Slint, and that made it hard. XC Counter draws **no screen of its own** beyond
the pair-code field and the manager-PIN exit. Every desk and kiosk screen stays the web app, so there is one codebase and one look
(`xchange-one-look.md`). The app adds only what a browser cannot do: device access, kiosk lock and counter identity.
From `pos108-terminal` it reuses only the release machinery: signed bundles and self-update.

- **Tauri 2, Rust, macOS first.**
  - Rust matches the fleet. The drawer code is the existing agent crate, used as a library, not rewritten.
  - The webview is WKWebView, the system's own; no Chromium ships.
  - It builds for `aarch64-apple-darwin` and `x86_64-apple-darwin`, the targets `device-agent.yml:59-62` already builds.
- **Stable only (owner: *"เอาที่เสถียร์ นะ"*).**
  - Tauri 2 from its stable release line, with an exact version pinned in `Cargo.toml` and `Cargo.lock` (`=2.x.y`). No beta, RC or nightly.
  - Only official `tauri-plugin-*` crates; no community plugin. ImageCaptureCore comes through the `objc2` framework crates, also pinned.
  - The fleet's pinned Rust toolchain is used.
  - Upgrades happen only in a release commit, never in a feature PR.
- **IPC pinned (review item 4).**
  - The Tauri capability for the bridge commands names **one exact `https` origin** from the app's config, scoped to the paths `/desk*` and `/kiosk-staff*`.
  - Only the app's own commands are exposed (`info`, `scanPassport`, `deskBinding`), and no core or plugin command is reachable from the page.
  - `on_navigation` refuses every other URL, and `new-window` is refused.
  - The agent token and the Keychain stay in Rust; no command returns them.
- It opens the configured server (`https://xc-v3.spacing.cloud` on staging) at `/desk`, and external links open in Safari. The scope is stated once, under "IPC pinned" above.
- **Kiosk lock**, owner-set per counter (§4):
  - full-screen with no window chrome;
  - Cmd+Q, Cmd+W and Cmd+Tab are intercepted while locked;
  - an "ออก" action needs a manager PIN, checked by the server.
- **Pairing reuses the agent's.**
  - On first run the app shows a pair-code field, `POST /api/device-agent/pair`, exactly as today.
  - The token is kept in the macOS Keychain, never in a file.
  - The counter's name, for example "เคาน์เตอร์ 1", is set by the owner when creating the pair code.
- **Updates.** Signed and notarised. It updates itself from the release feed, as `pos108-terminal` does for its bundles.

## 3. The bridge (what the desk can ask the app)

The desk detects the app through `window.xcCounter`, which is present only inside the app.

| Call | Does | Answers |
|---|---|---|
| `xcCounter.info()` | who this counter is | `{ agentId, counterName, branchId, capabilities: { cashDrawer, scanner, kioskLock }, appVersion, build }` |
| `xcCounter.scanPassport()` | scans one page: colour, 300 dpi, JPEG, whole bed, 60 s timeout | `{ ok: true, file: Blob }`, or `{ ok: false, reason: 'noScanner' \| 'busy' \| 'timeout' \| 'coverOpen' \| 'device' }` |
| `xcCounter.openDrawer()` | not offered | the drawer stays server-driven (below) |
| `xcCounter.bindCounter()` | gets a one-time nonce through the app's Rust side and calls `deskBindCounter(nonce)` (§5.4) | `{ ok }` |

- **Scan.** The desk puts the returned file into `store.passport()`, exactly as an attached file. From there the **existing** upload
  and OCR run unchanged. Nothing new crosses the server, so the scan needs no migration of its own.
- **Drawer.** It stays on the existing, audited path: server-created `DRAWER_REQUEST` jobs, claimed by the agent runner that now runs
  **inside the app**. The only change is that a job targets a counter (§5). The drawer opens at the counter that made the sale, not
  at any agent in the branch.
- **Outside the app** (a plain browser): `xcCounter` is absent. The desk hides "สแกนพาสปอร์ต" and keeps the file field. Drawer jobs
  that target a counter with no app running stay QUEUED and expire as today.

## 4. Per-counter config (owner-set, rule #5993)

| Setting | Default ("not set") | Effect |
|---|---|---|
| `cashDrawer` (รองรับลิ้นชักเงิน) | off | On: the desk may make cash sales at this counter, and its drawer jobs target it. Off: the counter can take customers, key, scan and issue queue tickets, but a cash payout is refused here, worded: "เคาน์เตอร์นี้ไม่มีลิ้นชักเงิน — ส่งไปชำระที่เคาน์เตอร์ที่มีลิ้นชัก" |
| `scanner` (มีเครื่องสแกนพาสปอร์ต) | off | On: the desk shows "สแกนพาสปอร์ต" and calls `scanPassport()` |
| `kioskLock` (ล็อกเต็มจอ) | on | Full-screen lock as in §2 |

- Only the Owner saves these. The screen is Admin → สาขา → เคาน์เตอร์. It lists the branch's paired counters and their last-seen time,
  with the three toggles and the counter name.
- The app reads them in `info()` from the server at start and on every claim, so a change applies without reinstalling.
- **Decided (owner, "อนุมัติ + กฎ A", 2026-11-06 09:29 ICT): rule A.** At a counter with `cashDrawer` off, a cash sale is refused with the
  worded message above. The teller sends the customer to a drawer counter, where the sale is keyed. No new transaction state is added.

## 5. Data (owner approved the first version, "อนุมัติ + กฎ A", 2026-11-06 09:29 ICT; §5.4's column was approved separately; deploy is a separate order)

One forward-only migration. It is revised after review (PR 6361, claude/f8fcb767, five must-fixes at `26fda56`); §5.4 asks the owner about the parts that go beyond his 09:29 approval.

### 5.1 Several counters per branch
- Drop `device_agents_one_active_per_branch_idx`, and add `device_agents.counter_name text NOT NULL DEFAULT ''`.
- **Code that ships in the same change (CA-0), because it relies on that index:**
  - `insert_agent`'s `ON CONFLICT (branch_id) WHERE revoked_at IS NULL` (`pg_agent_repository.rs:189-194`) becomes a plain insert. Creating a pair code no longer replaces the branch's active agent; it adds a counter.
  - `has_active_agent_in` (`:147-158`) becomes `active_agents_in`, which lists the counters.
  - `branch_tablet_store_test.rs:335`, which pins the index, is restated by the controller.

### 5.2 Per-counter config, "not set" is safe (rule #5993)
- Add `device_agents.cash_drawer boolean NULL`, `device_agents.scanner boolean NULL` and `device_agents.kiosk_lock boolean NULL`.
- **Each value is the owner's, and nothing is inferred** (owner: *"ว่าจะมี ลื้นช้กหรือไม่ม่ก็ได้"*, a counter may have a drawer or not, as he sets it).
  - NULL means not set, and the not-set value is the safe one:
    - `cash_drawer` NULL = **off**: no cash payout and no drawer jobs at that counter;
    - `scanner` NULL = **off**;
    - `kiosk_lock` NULL = **on**.
  - The Admin screen shows "ค่าเริ่มต้น" while a value is NULL.
- **No backfill, and no derivation from `drawer_transport`.** On xc-v3, the drawer agent paired today stops claiming drawer jobs until the owner turns `cash_drawer` on for it. That takes one click on the new screen (CA-2), and nothing is in production. `drawer_transport` and `drawer_target` stay as the agent's report of *how* it would kick the drawer, not *whether* the counter has one.
- **Wire change, so a counter without a drawer can claim (review round 2, item 1).**
  - `ClaimRequest.drawer` becomes `Option<DrawerTargetDto>`, in both `api/src/presentation/dto/device_agent.rs:26-41` (the server) and
    `api/crates/xc-device-agent/src/client.rs` (the agent).
  - The app sends `None` when the counter has no drawer configured locally.
  - The claim writes `drawer_transport = $8` and `drawer_target = $9` as sent, NULL included (`pg_agent_repository.rs:241`, `:270`; today `COALESCE`).
  - Today's agent always sends a drawer, so its behaviour is unchanged.
  - Ownership: CA-1 owns the server DTO and the claim SQL. CA-3 owns the agent's `client.rs`. The shape is pinned here.

### 5.3 Drawer jobs target a counter
- Add `device_jobs.target_agent_id uuid NULL REFERENCES device_agents (id)`.
- **Claim:** agent X may claim a job when `target_agent_id = X`, or when `target_agent_id IS NULL` and X's `cash_drawer` is on. When several such agents exist, the one claiming wins under the lease, as today; the claim SQL prefers agents whose `last_seen_at` is within the claim window, so a closed app does not stall NULL-target jobs.
- **Revoke** (`pg_agent_repository.rs:452-456`, `:573-577`, review item 2): revoking counter X fails only the QUEUED or LEASED jobs with `target_agent_id = X`. NULL-target jobs are failed only when no other active agent with `cash_drawer` on remains in the branch; otherwise they stay QUEUED for that agent.
- The other claim queries (`:302,321,348`) take the claim rule above.
- Revoke now fails LEASED jobs too; today they are left to lapse. A late report under that lease cannot land, because the report requires `status = 'LEASED'` (`:414-415`). That is accepted.

### 5.4 A counter the server can trust (review item 5): approved by the owner ("อนุมัติ", 2026-11-06 09:59 ICT)
The desk must not name its own counter, since any page could claim to be the drawer counter.
- **Binding.** The app's Rust side, holding the agent token that JS never sees, calls `POST /api/device-agent/desk-binding`.
  - The server answers a nonce: an **HMAC-signed `{agentId, branchId, exp: now+60s}`**, signed with the server's existing secret. It is not stored, so there is no table and it works with several API instances.
  - A replay inside the 60 s only binds the same counter again, which is harmless.
  The app hands the nonce to the desk page once, through the bridge. The desk calls the command `deskBindCounter(nonce)` with its staff session.
  - `deskBindCounter` **refuses** unless the signature and `exp` are valid, the agent is active (not revoked), and the agent's `branch_id` equals the staff session's `branch_id` (the column `api/crates/xc-auth/src/application/session/session_service.rs:402` writes).
  - The app re-binds after every sign-in and on each desk load. `refresh` updates the row in place (`session_service.rs:778`), so the binding survives a refresh; a new sign-in writes a new row.

  The server then records `user_sessions.counter_agent_id = X` on the staff session row that `create_session_for_user` writes (`api/crates/xc-auth/src/application/session/session_service.rs:354`) (a new nullable column, approved). It is cleared on sign-out and on revoke.
- **Server-side use:** drawer jobs from that session get `target_agent_id = counter_agent_id`. Rule A reads the bound counter's `cash_drawer`.
- **A session with no bound counter** (a plain browser) follows **a branch setting the owner sets**, not a rule the system infers.
  Owner: *"ก็ผมมีให้ มี config แล้วไง"*.
  - The setting is `cashFromBrowser` (จ่ายเงินสดจากเบราว์เซอร์ได้), stored in `branch_settings`, which needs no migration. Only the Owner saves it, on Admin → สาขา → เคาน์เตอร์.
  - On: an unbound session may pay out cash, as today. Its drawer jobs get `target_agent_id NULL`.
  - Off: cash payout needs a session bound to a counter whose `cash_drawer` is on; rule A's message is shown otherwise.
  - "Not set" = **on**, today's behaviour. That keeps every current desk working until the owner turns it off, and the screen shows "ค่าเริ่มต้น (เปิด)".

## 6. Shape (♻️ one core, one part per platform)

```mermaid
classDiagram
  direction LR
  class CounterApp { <<counter-app/src-tauri, Tauri 2>> window, origin lock, kiosk lock, pairing, bridge }
  class Scanner { <<trait, counter-app/src-tauri/src/scan/mod.rs>> +probe() +scan(ScanRequest) }
  class MacScanner { <<scan/mac.rs, ImageCaptureCore via objc2-image-capture-core>> }
  class FakeScanner { <<scan/fake.rs, tests>> }
  class AgentRunner { <<xc-device-agent, as a library>> claim → kick → report }
  CounterApp --> Scanner
  CounterApp --> AgentRunner
  Scanner <|.. MacScanner
  Scanner <|.. FakeScanner
```

A later Windows (WIA) or Linux (SANE) counter is one more `Scanner` file, plus a build target.

## 7. Rollout

| Ticket | What | Who |
|---|---|---|
| CA-0 | migration §5 plus the code that relied on the dropped index (§5.1), and the revoke and claim rules (§5.3); approved | Gemini (guarded); up-front migration test by the controller |
| CA-1 | server: counter config view/save (owner-only), `info` answer, claim by target, desk cash-sale refusal at a no-drawer counter (rule A) | Gemini (logic); up-front tests |
| CA-2 | Admin → เคาน์เตอร์ screen: list, rename, three toggles | Gemini (logic); up-front tests |
| CA-3 | app shell: Tauri window, origin lock, kiosk lock, pairing, Keychain token, the `xcCounter.info` bridge, the agent runner inside | Gemini (logic) |
| CA-4 | app: `Scanner` trait, `FakeScanner`, `MacScanner`, `scanPassport` bridge; proven on the real Mac with the real scanner | Gemini (logic) |
| CA-5 | desk: detect `xcCounter`, the "สแกนพาสปอร์ต" button, refusals, hand the file to the existing OCR | Gemini (logic); up-front tests |
| CA-6 | release: signing, notarisation and update feed (needs the Apple developer account the owner already uses for `pos108-terminal`) | controller and owner |

**Still needed from the owner:** the scanner model. The check is to open Image Capture on the counter Mac and see whether the scanner is listed.
§5 and rule A are decided.
