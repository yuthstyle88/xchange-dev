# XCHANGE System Blueprint v3 — mapped against v3's rate design

> mapped-by: claude/c954feda · 2026-10-10 · draft for #986 · measured on `main` at `9bab231`

| | |
|---|---|
| **Source** | The owner's [`xchange-system-blueprint-v3.html`](xchange-system-blueprint-v3.html) (2026-10-10; cover page [`xchange-system-blueprint-v3.md`](xchange-system-blueprint-v3.md)). "BP §N" is its section |
| **Compared with** | v3's rate design [`blueprint/rates.md`](blueprint/rates.md), which follows the earlier XC Desk Blueprint v2. "`rates.md:N`" is a line at `9bab231` |
| **What this is** | A mapping, **not a decision**. For each difference it records what each side says, what v3 must do where the two meet, and what only the owner can decide |

## The owner's ruling this rests on

| # | Question | Answer, verbatim | Date |
|---|---|---|---|
| R-1 | Does Blueprint v3 replace v3's rate design (A), or is it a separate system that v3 reads rates from (B)? | *"B เดินต่อ #757 ได้"* (B; #757 continues) | 2026-10-10, #986 |

So:
- Blueprint v3 describes **another system**: SCB source → pricing service (server API + D1) → Telegram Bot / Mini App and a read-only branch board (BP §03).
- v3 is the side BP §03 calls "XC Unified", which reads that system's rates through an API.
- BP §03 itself says the two do not share a database: *"ไม่ย้ายฐานข้อมูลร่วมกันทันที"*.
- `rates.md` stays v3's own design, including the per-`unit_size` work of #757.
- Every row below is therefore a difference **between two systems that meet at an API**. The question each row asks is what v3 does with what it receives, not which design wins.

## 1. Where the two meet in v3 today

| Piece | Where | What it is (code where the path is a source file, design where it is `rates.md`) |
|---|---|---|
| `RateSource` port | `api/crates/xc-rate/src/domain/rate/entities/rate_source.rs:14` | `fetch() -> ReferenceRates { source, fetched_at, kind, rows }`. One production implementation: `BotRateSource` (`bot_rate_source.rs:95`) |
| Reference tables | `rates.md:232-275` (§4.1) | `rate_reference_batches` / `rate_reference_lines`: each provider's rows **in the provider's own unit**, with `raw`, never rewritten |
| AUTO base | `rates.md:305-331` (§4.2), A6 `rates.md:29` | BoT: buy = buying transfer, sell = selling. The lookup is pinned to `reference_type = 'BOT'` in RE-7 (`rates.md:298-299`) |
| A non-BoT provider | `rates.md:284-303` (§4.1 "Partner cost") | a `RateReferenceBatch` with `provider = <name>` and lines with `reference_type = 'PARTNER'`. **Never an AUTO base**; copied into `source_snapshot.partner` at publish |
| v3's own price | `rates.md:332-372` (§4.3–4.4) | AUTO = `base × (1 + adjust_pct ÷ 100)` from the branch's `RatePolicyVersion`, or MANUAL; then draft → approval → immutable publication |

A new provider plugs in as one more `RateSource` implementation, not a copy of the reference path. That is the ♻️ rule (shared core + one implementation per variant).

## 2. The differences

| # | Topic | Blueprint v3 (the other system) | v3 (`rates.md`) | What v3 must do where they meet | Decided? |
|---|---|---|---|---|---|
| 1 | Base source | SCB banknote rate as announced. BoT and other banks are reference only (BP §02, §03) | BoT: buy = buying transfer, sell = selling (A6 `rates.md:29`, §4.2 `rates.md:313`). BoT covers 6 currencies (`rates.md:315`) | Receive the other system's rows through a new `RateSource` and store them as a reference batch. **Which role they play is Q-1** | **No — Q-1** |
| 2 | Margin | 4 tiers by THB value at the base rate: ≤10,000 → 0.80 %, ≤50,000 → 0.60 %, ≤200,000 → 0.40 %, above → 0.25 %; buy and sell the same; one tier per transaction (BP §02) | one `buy_adjust_pct` / `sell_adjust_pct` per policy version; no number in code (D-6 `rates.md:138`, §4.3 `rates.md:332`) | If v3 takes the other system's **shop** prices, its own `adjust_pct` would add a second margin on top. If it takes the **SCB base**, its own policy applies and the tiers do not. Q-1 decides which | **No — Q-1** |
| 3 | When the price is fixed | depends on the transaction's value, so it is known only with the amount. Quote snapshot per transaction (BP §02, §07 `quote_rate_snapshots`) | one rate per currency and side per publication (D-3 `rates.md:121`); a quote stores its line id and rate snapshot (`rates.md:496`) | A v3 publication holds one rate per side. Tiered prices fit it only as several lines, or as a quote-time call to the other system. **Q-2** | **No — Q-2** |
| 4 | Unit on the wire | `rate_thb_per_1` in API, DB and Telegram; `display_multiplier` only for display (BP §08) | rate per currency `unit_size` (JPY 100, KRW 1000), scale 8 (D-1 `rates.md:91`) | Convert once, on receipt: store the line with `unit_size = 1` as the provider quotes it, and normalise where it is used, as §4.2 already does for BoT (`base = ref_value × currency.unit_size ÷ ref.unit_size`). No change to v3's storage | Follows from R-1 |
| 5 | Rounding | buy rounds down, sell rounds up, to a step not yet approved (BP §02) | rate HALF_UP to 8 places; THB HALF_UP to 2 places once (§3.3 `rates.md:219-220`, §4.4 `rates.md:361`) | If v3 takes the other system's final prices, it stores them as received and does not round them again. v3's THB rounding of the amount stays its own. The step itself is the other system's open question (BP §02) | Partly: v3 side follows R-1; the step is open in BP §02 |
| 6 | Banknote bands | `instrument_key` per SCB band, e.g. USD 50–100 / 5–20 / 1–2 (BP §08) | one `ALL` group per currency (A4 `rates.md:27`, §3.4 `rates.md:222`) | Received rows carry a band; v3 has one group. **Q-3**: which band v3 takes as its line, or whether it starts pricing by group. §3.4 needs no schema change for groups | **No — Q-3** |
| 7 | Approving a change | pending → `/confirm` code in Telegram by the same owner within 10 minutes, bound to a revision; one use (BP §06, §09) | approval core: in-bound → RATES, out-of-bound → OWNER (D-4/D-5 `rates.md:131`, §5) | Each system approves its own changes. A row received from the other system is reference data and is not "approved" by v3; v3's own publish still goes through its approval core | Follows from R-1 |
| 8 | Manual prices | override stays until reset; a new SCB rate that conflicts hides the price (`HIDDEN_CONFLICT`) until fixed (BP §06) | MANUAL per side with a reason (§4.4 `rates.md:351`, CHECK in §4.1) | A side the other system hides arrives as missing. v3 already treats a `NULL` side as unavailable, never guessed (`rates.md:281`) | Follows from R-1 |
| 9 | Stale / failed source | source failure: keep the last announcement with a warning and its real time; screen loses connection: hide prices (BP §05, §11) | `on_stale` = `STOP` / `LIMIT` / `NULL` per policy version (§4.3 `rates.md:338-343`) | v3's stale rule reads the reference's real source time. The other system must send it (BP §08 `source_published_at`, kept apart from `fetched_at`); v3 stores it as `source_date` + `fetched_at` | Follows from R-1, if the API sends both times (Q-4) |
| 10 | Which system | React/Vinext on Sites, D1, Telegram Mini App, branch board (BP §03) | the v3 Rust server and its PostgreSQL | Two systems, linked by API | **Decided — R-1 (B)** |

## 3. What only the owner can decide

| # | Question | Options | Why it matters |
|---|---|---|---|
| Q-1 | What role do the other system's rates play in v3? | **a.** Its SCB **base** becomes v3's AUTO base (instead of BoT); v3's own `adjust_pct` and approval stay. **b.** Its **shop** prices are v3's prices; v3 publishes them as received (adjust 0, no tiers of its own). **c.** Reference only, like `PARTNER` today (`rates.md:284-303`): shown beside BoT, never a price | a keeps `rates.md` almost as is; b makes rows 2, 3 and 5 the other system's rules inside v3; c changes nothing in pricing |
| Q-2 | If prices are tiered (Q-1 b), how does a v3 quote get its tier's price? | publish one line per tier, or ask the other system per quote and snapshot the answer (BP §07 `quote_rate_snapshots`) | v3's publication holds one rate per side (D-3); per-quote calls add a runtime dependency on the other system |
| Q-3 | Which banknote band is v3's price for a currency? | one chosen band (e.g. USD 50–100), or price by group (`rates.md:222`) | the other system sends several rows per currency (BP §08) |
| Q-4 | What exactly does the other system's API send, and how does v3 authenticate? | BP §09 proposes a payload (`source{code,published_at,fetched_at,status}`, `rows[{instrument_key, …, buy_thb_per_1, sell_thb_per_1, availability}]`) and service auth (BP §10), but says the real method must still be confirmed | the `RateSource` implementation is written against it; nothing can be built before it is known |

## 4. Work this affects

Nothing below changes until Q-1 is answered. Per R-1, the RE-* chain continues as designed.

| Work | Affected by | How |
|---|---|---|
| #757 (RE-4c) and the RE-4 readers | rows 4, 5 | **Not affected** under R-1: per-`unit_size` storage stays; a per-1 provider is normalised on receipt (§4.2) |
| A new `RateSource` for the other system | Q-1, Q-4 | one implementation file beside `bot_rate_source.rs`; its ticket needs Q-4's answer first |
| AUTO base selection (RE-7, `rates.md:298-299`) | Q-1 a | the design pins the AUTO base to `reference_type = 'BOT'` in RE-7 (the lookup takes the newest line of any type until then); Q-1 a pins it to the new provider's type instead |
| The TX / quote chain (#551, #670 …) | Q-1 b, Q-2 | tiered prices would need the quote to carry the transaction's THB value and a snapshot of the tier |
| Denominations (§3.4) | Q-3 | groups other than `ALL` |
