# XCHANGE System Blueprint v3 (owner's requirements, 2026-10-10)

> written-by: claude/c954feda · owner's order 2026-10-10: *"เก็บ blueprint เข้า repo เป็นเอกสารออกแบบ"*

The blueprint itself is [`xchange-system-blueprint-v3.html`](xchange-system-blueprint-v3.html), stored
exactly as the owner sent it (SHA-256 `ea2f36b1a2fbf569…`). GitHub shows HTML as source; download it or
open it locally to read it. This page says what it is, so nobody has to open it to find out.

## What it covers

The document's own subtitle: **from the SCB rate to the counter price and the branch screen**. Its
sections:

| § | Topic |
|---|---|
| 01 | Scope and status |
| 02 | Business rules and pricing: tiers by THB value at the SCB base rate (≤10,000 · ≤50,000 · ≤200,000 · above), with margins 0.80 / 0.60 / 0.40 / 0.25 %; shop buy = SCB buy × (1 − margin), shop sell = SCB sell × (1 + margin); buy rounds down, sell rounds up |
| 03 | System structure: SCB source → pricing service (server API + D1) → Telegram Bot / Mini App and a read-only branch board; Google Sheet; XC Unified |
| 04–05 | Screens S01–S08 and their wireframes (Mini App rate table, price edit, `/confirm`, publishing and board link, branch board, bot, history, Unified quote) |
| 06 | Job order and state control (refresh, price change with pending → `/confirm`, overrides, board, scheduler, Unified quote) |
| 07–08 | Data model (4 reported tables + proposed tables) and the unit dictionary (`rate_thb_per_1`, `display_multiplier`, `instrument_key`) |
| 09–10 | API contract, Telegram commands, permissions and security |
| 11 | Acceptance criteria and UAT |
| 12 | Delivery plan and decision points |

## Status of what it says

These are the document's own caveats, not new ones:

- It is the owner's requirements as of 10 October 2026. The running state it reports ("enabled",
  "checked") was passed on by the owner. **The document did not check the live system or its source.**
- The system it describes (React/Vinext on Sites, a D1 database, a Telegram Mini App) is **not this
  repository's v3 server**. Section 03 proposes linking to "XC Unified" through an API, not sharing a
  database.
- Column names and extra tables in § 07 are **proposals**. They must be checked against real
  migrations and APIs before any schema change.
- § 02 lists what must be decided before a pricing engine is written: the price and THB precision/step,
  how a multi-currency transaction is valued, and how a THB-entered amount is converted.

## How it relates to this repository's designs

Nothing here has been mapped to v3's own rate design ([`blueprint/rates.md`](blueprint/rates.md)) yet.
Where the two overlap, examples include tiers, rounding, "per 1" storage (`rate_thb_per_1` compared with
v3's per-1 rates) and published lines compared with board revisions. Comparing them is open work, not a
decision recorded here.
