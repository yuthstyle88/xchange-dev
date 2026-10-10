# Rate feed connector: v3 prices from the rate board system

> designed-by: claude/c954feda · 2026-10-10 · owner's order *"ออกแบบ class diagram ตัวเชื่อมระบบเรทเลย"* ·
> measured on `main` at `875f7e8` · `file:line` below is at that sha

## 0. What this is for, and what is decided

v3 takes its counter prices from the other system described in [`xchange-system-blueprint-v3.html`](xchange-system-blueprint-v3.html)
(the "rate board": SCB base → tier margins → Telegram Mini App and branch screen). The mapping of that blueprint
against v3's own rate design is [`blueprint-v3-rates-mapping.md`](blueprint-v3-rates-mapping.md). The owner's
rulings there are this design's inputs:

| # | Ruling (verbatim on #986) | What it fixes here |
|---|---|---|
| R-1 | *"B เดินต่อ #757 ได้"* | The rate board is another system. v3 reads it through an API, and keeps per-`unit_size` storage |
| R-2 | *"bราคาต้องตรงกับจอสาขา เอา B"* | The board's **shop price** is v3's price, used as received. v3 adds no `adjust_pct` to it |
| R-3 | *"…เค้า สร้างได้เองดีกว่า…"* / *"ใช่ เจ้าของร้าน/แอดมินใน v3"* | Banknote groups are created by the owner/admin in v3 and mapped to the board's rows (`instrument_key`). Nothing is fixed in code |
| R-4 | *"ทำตามที่เสนอ"* (Q-2 as proposed) | v3 publishes **every tier's** price in advance. A quote picks the tier from the transaction's value, with no call to the board per quote |
| R-5 | *"ทำตามที่เสนอ"* (Q-5 as proposed) | No second approval in v3. A board price is published automatically and audited |
| R-6 | *"ทำตามที่เสนอ"* (Q-6 as proposed) | While the board is paused, the counter stops selling: board-priced sides are unavailable, and the last published price is **not** kept |
| R-7 | *"Q-7, Q-8 ตามที่เสนอ"* | The board price is switched on **per branch**. A branch not switched on keeps BoT drafts; branches move one at a time and can be switched back |
| R-8 | *"Q-7, Q-8 ตามที่เสนอ"* | v3 pulls on its **own timer, about every minute**, well under the board's 15-minute source check. The interval is a setting, not a constant |

**Not known yet; this design does not guess it:**
- **Q-4:** the board's real API — URL, credential, payload. The developer question went to the owner on 2026-10-10.
- §7 lists the other open questions.

Everything that depends on Q-4 is isolated in **one file pair** (§2, `RateBoardSource` + `RateBoardPayload`). Every
other class can be built, and tested against a fake source, before Q-4 is answered.

## 1. What exists today

| Piece | Where | Fact |
|---|---|---|
| `RateSource` port | `api/crates/xc-rate/src/domain/rate/entities/rate_source.rs:14-16` | `fetch() -> ReferenceRates { source, fetched_at, kind, rows: Vec<serde_json::Value> }` |
| BoT source | `api/crates/xc-rate/src/infrastructure/rate/bot_rate_source.rs:95` | the only production `RateSource` |
| Reference writer | `api/crates/xc-rate/src/application/reference/reference_batch_writer.rs:33-184` | **BoT-specific inside a generic-looking service**: `BOT_CURRENCIES` (`:16`, used `:111`), provider/type `"BOT"` (`:74`, `:162`, `:172`), BoT column names `buyingTransfer`/`selling` (`:148`, `:154`) |
| Reference line | `api/crates/xc-rate/src/domain/reference/reference_batch.rs:6-11` | `ReferenceLine { currency, unit_size, buy, sell }` |
| Publish | `api/crates/xc-rate/src/application/publication/publish_service.rs:28` | publishes an **approved draft**; locks `branches` `FOR UPDATE` and checks the expected version |
| Approval subject | `api/crates/xc-rate/src/application/approval/rate_publication.rs:20-118` | `RatePublicationSubject` (`"RATE_PUBLICATION"`) + `RatePublicationPolicy` (OWNER decides) registered in `ApprovalRegistry` |
| Published line reader | `api/crates/xc-rate/src/domain/publication/published_rates.rs:58` (`PublishedRates`), impl `api/crates/xc-rate/src/infrastructure/publication/published_rates.rs:69-82` | `line_in(tx, branch, code, side)` reads **one** line, hard-wired to `denomination_group = 'ALL'` |
| Readers of `line_in` | `api/src/application/workflows/exchange/quote.rs:103`, `api/src/infrastructure/kiosk/kiosk_ports.rs:415`, `api/src/infrastructure/bookings/booking_ports.rs:110` | three callers |
| Quote request | `api/src/application/workflows/exchange/quote.rs:22-27` | `QuoteRequestLine { currency, shop_side, quantity, fee_thb }`: no group. `:183` stores `denomination_group: "ALL"` |
| THB arithmetic | `api/crates/xc-kernel/src/money/exchange_arithmetic.rs:16` | `ExchangeArithmetic::thb(amount, rate, unit) -> Money`, the one place that divides by `unit_size` |
| Tables | `api/migrations/20261010120000_rate_engine.sql:13-22`, `:78-99`, `api/migrations/20261010100000_rate_units.sql:25-33` | `rate_reference_lines UNIQUE (batch_id, currency_code, reference_type)` (`:22`); `rate_publications.approved_request_id NOT NULL` (`:83`), `draft_id NULL` (`:84`); `published_rate_lines UNIQUE (publication_id, currency_code, denomination_group)`; `denominations(currency_code, face_value, group_code DEFAULT 'ALL')`, seeded one `'ALL'` per currency (`rate_units.sql:32-33`) |
| Quote lines | `api/migrations/20261014100000_quotes.sql:23-30` | `quote_lines.published_rate_line_id NOT NULL` + `denomination_group` |
| System actor | `api/crates/xc-auth/src/domain/policy/value_objects/role.rs:13` | `Role::System` exists |

## 2. Class diagram

```mermaid
classDiagram
    direction LR

    class RateSource {
        <<trait>>
        +fetch() BoxFuture~AppResult~ReferenceRates~~
    }
    class ReferenceProvider {
        <<trait>>
        +provider() &'static str
        +reference_type() &'static str
        +row_key(row: &Value) String
        +parse(rates: &ReferenceRates) Result~ParsedBatch, RateEngineError~
    }
    class ParsedBatch {
        +source_date: NaiveDate
        +source_published_at: Option~DateTime~Utc~~
        +complete: bool
        +lines: Vec~ParsedLine~
    }
    class ParsedLine {
        +currency: CurrencyCode
        +instrument_key: Option~String~
        +value_tier: Option~TierCode~
        +tier_band: Option~TierBand~
        +unit_size: UnitSize
        +base_buy: Option~Rate~
        +base_sell: Option~Rate~
        +buy: Option~Rate~
        +sell: Option~Rate~
        +raw: serde_json::Value
    }
    class ReferenceBatchWriter {
        +new(source: Arc~dyn RateSource~, store) Self
        +with_provider(provider: Arc~dyn ReferenceProvider~) Self
        +fetch_and_write(tx) Result~Uuid, RateEngineError~
    }
    class BotProvider
    class RateBoardProvider
    class BotRateSource
    class RateBoardSource {
        +new(config: RateBoardConfig) Self
    }
    class RateBoardPayload {
        <<Q-4: stub until the API is known>>
    }

    class TierCode
    class TierBand {
        +code: TierCode
        +lower_exclusive_thb: Option~Money~
        +upper_inclusive_thb: Option~Money~
        +contains(value: Money) bool
    }
    class TierTable {
        +bands: Vec~TierBand~
        +pick(base_value_thb: Money) Option~TierCode~
    }

    class DenominationGroups {
        <<trait>>
        +groups_of(tx, code) Vec~GroupCode~
        +source_of(tx, code, group, provider) Option~String~
    }
    class PgDenominationGroups
    class DenominationGroupService {
        +create_group(tx, who, code, group) Result
        +map_group(tx, who, code, group, provider, instrument_key) Result
        +remove_group(tx, who, code, group) Result
    }

    class FeedPublisher {
        +publish_latest(tx, branch, provider) Result~Option~PublicationRef~, RateEngineError~
    }
    class RateFeedPublicationSubject {
        +type_name() "RATE_FEED_PUBLICATION"
    }
    class RateFeedPublicationPolicy {
        +may_decide(who) only Role::System
    }

    class PublishedRates {
        <<trait>>
        +line_in(tx, branch, code, side) Option~PublishedSide~
        +line_for(tx, branch, sel: LineSelector) Option~PublishedSide~
    }
    class LineSelector {
        +code: CurrencyCode
        +side: Side
        +group: GroupCode
        +quantity: Option~ForeignAmount~
    }
    class PublishedSide {
        +value_tier: TierCode
    }
    class BranchPriceSource {
        <<trait>>
        +source_of(tx, branch) PriceSource
        +set(tx, who, branch, source) Result
    }
    class PgBranchPriceSource
    class RateFeedLoop {
        +run(interval: Duration)
        +tick() Result
    }
    class QuoteService {
        +quote_in(tx, req, who, now) AppResult~Quote~
    }

    RateSource <|.. BotRateSource
    RateSource <|.. RateBoardSource
    ReferenceProvider <|.. BotProvider
    ReferenceProvider <|.. RateBoardProvider
    RateBoardSource ..> RateBoardPayload : decodes
    RateBoardProvider ..> RateBoardPayload : reads rows of
    ReferenceBatchWriter --> RateSource
    ReferenceBatchWriter --> ReferenceProvider
    ReferenceProvider ..> ParsedBatch : returns
    ParsedBatch *-- ParsedLine
    ParsedLine --> TierBand
    TierTable *-- TierBand
    DenominationGroups <|.. PgDenominationGroups
    DenominationGroupService --> DenominationGroups
    FeedPublisher --> DenominationGroups : group → instrument_key
    FeedPublisher --> RateFeedPublicationSubject : opens + closes request as SYSTEM
    RateFeedPublicationSubject .. RateFeedPublicationPolicy
    PublishedRates ..> LineSelector
    PublishedRates ..> PublishedSide
    PublishedRates ..> TierTable : picks the tier
    QuoteService --> PublishedRates : line_for
    BranchPriceSource <|.. PgBranchPriceSource
    RateFeedLoop --> ReferenceBatchWriter : one fetch per tick
    RateFeedLoop --> BranchPriceSource : branches on the board
    RateFeedLoop --> FeedPublisher : publish_latest per branch
```

### Where each class lives

🔴 Two classes that different tickets touch are never in the same file.

| Class | File | Ticket (§6) | Kind |
|---|---|---|---|
| `ReferenceProvider`, `ParsedBatch`, `ParsedLine` | `api/crates/xc-rate/src/domain/reference/reference_provider.rs` (new) | F1 | `[required]` |
| `BotProvider` | `api/crates/xc-rate/src/infrastructure/reference/bot_provider.rs` (new) | F1 | `[required]` |
| `ReferenceBatchWriter` (changed) | `api/crates/xc-rate/src/application/reference/reference_batch_writer.rs` | F1 | `[required]` |
| `TierCode`, `TierBand`, `TierTable` | `api/crates/xc-rate/src/domain/publication/value_tier.rs` (new) | F2 | `[required]` |
| migration | `api/migrations/<next>_rate_feed.sql` (new) | F2 | `[required]` |
| `DenominationGroups`, `GroupCode` | `api/crates/xc-rate/src/domain/currency/denomination_groups.rs` (new) | F3 | `[required]` |
| `PgDenominationGroups` | `api/crates/xc-rate/src/infrastructure/currency/denomination_groups.rs` (new) | F3 | `[required]` |
| `DenominationGroupService` | `api/crates/xc-rate/src/application/currency/denomination_group_service.rs` (new) | F3 | `[required]` |
| admin commands `denominationGroupCreate` / `…Map` / `…Remove` | `api/src/presentation/http/v3/commands/denomination_groups.rs` (new) | F3 | `[proposed]` — the command-seam path is the one `rates.md` §6 uses; take the real directory from where the existing v3 commands live |
| admin screen | `web/src/screens/admin/denomination-groups/` (new directory, whole subtree) | F3w | `[proposed]` |
| `RateBoardSource`, `RateBoardConfig` | `api/crates/xc-rate/src/infrastructure/rate/rate_board_source.rs` (new) | F4 | `[required]` |
| `RateBoardPayload` | `api/crates/xc-rate/src/infrastructure/rate/rate_board_payload.rs` (new) | F4 | **Q-4** |
| `RateBoardProvider` | `api/crates/xc-rate/src/infrastructure/rate/rate_board_provider.rs` (new) | F4 | `[required]` |
| `ReferenceLineRecord` gains the §4 columns; insert writes them | `api/crates/xc-rate/src/infrastructure/reference/pg_reference_batch_store.rs` | F4 | `[required]`: only F4 produces those values. F1 does not touch this file |
| `RateFeedPublicationSubject`, `RateFeedPublicationPolicy` | `api/crates/xc-rate/src/application/approval/rate_feed_publication.rs` (new) | F5 | `[required]` |
| `FeedPublisher` | `api/crates/xc-rate/src/application/publication/feed_publisher.rs` (new) | F5 | `[required]` |
| registering `RATE_FEED_PUBLICATION` | `api/src/presentation/http/v3/commands/mod.rs` (`make_approval_service`, `:17-23`) | F5 | `[required]` |
| registering it in the test engine | `api/crates/xc-rate/tests/rate_engine_common/mod.rs` (`:224-225`) | F5 | `[required]`: a shared test helper. F5 only adds one `register` line; no provided test is edited |
| `LineSelector`, `PublishedSide.value_tier`, `PublishedRates::line_for` | `api/crates/xc-rate/src/domain/publication/published_rates.rs` | F6 | `[required]` |
| `PgPublishedRates::line_for` | `api/crates/xc-rate/src/infrastructure/publication/published_rates.rs` | F6 | `[required]` |
| `QuoteRequestLine.denomination_group`, step 3 of `quote_in` | `api/src/application/workflows/exchange/quote.rs` | F6 | `[required]` |
| `BranchPriceSource`, `PriceSource` | `api/crates/xc-rate/src/domain/branch/price_source.rs` (new) | F9 | `[required]` |
| `PgBranchPriceSource` | `api/crates/xc-rate/src/infrastructure/branch/price_source.rs` (new) | F9 | `[required]` |
| BoT publish refused on a `'RATE_BOARD'` branch | `api/crates/xc-rate/src/application/publication/publish_service.rs` | F9 | `[required]` |
| admin command `branchPriceSourceSet` | `api/src/presentation/http/v3/commands/branch_price_source.rs` (new) | F9 | `[proposed]` — same directory note as F3's commands |
| `RateFeedLoop`, `RateFeedConfig` | `api/src/infrastructure/rate_feed/feed_loop.rs` (new directory, whole subtree) | F10 | `[required]` |
| wiring the loop into server start | the server's start-up file (`api/src/main.rs` or the module that builds `AppState`) | F10 | `[proposed]` — the implementer names the real file in the ticket |

The `mod.rs` files each ticket must edit are its own: F1 `domain/reference/mod.rs` + `infrastructure/reference/mod.rs`;
F2 `domain/publication/mod.rs`; F3 `domain/currency/mod.rs`, `infrastructure/currency/mod.rs`, `application/mod.rs` (+ new
`application/currency/mod.rs`); F4 `infrastructure/rate/mod.rs`; F5 `application/approval/mod.rs` + `api/src/presentation/http/v3/commands/mod.rs` +
`application/publication/mod.rs`; F9 `domain/branch/mod.rs` + `infrastructure/branch/mod.rs`; F10 `api/src/infrastructure/mod.rs`. No two tickets share one.

**New v3 commands need no `mod.rs` edit.** `api/build.rs:238-257` reads every file in `api/src/presentation/http/v3/commands/` and generates the command table, which `api/src/presentation/http/v3/mod.rs:24` `include!`s. So F3's and F9's command files only add their own file. F5 is the only ticket that edits `commands/mod.rs`, and only for the registry.

### Who provides, who uses

| Provider | Used by | Fallback if the provider has not landed |
|---|---|---|
| `ReferenceProvider` + `ParsedBatch` (F1) | F4 `RateBoardProvider`, F5 `FeedPublisher` | F4/F5 start after F1 merges; F1 is a pure move, small |
| `TierTable` / `TierBand` (F2) | F1's `ParsedLine.tier_band`, F6 tier pick | F1 declares `ParsedLine` with `value_tier: Option<String>` and `tier_band: None` until F2 merges, then switches; comment in the file |
| migration (F2) | F3, F4, F5, F6 (columns) | every DB-touching ticket waits for F2 |
| `DenominationGroups` (F3) | F5 | F5 maps every currency's `'ALL'` group to the provider's first row of that currency **only in tests**; production needs F3 |
| `RateBoardSource` (F4) | F5 in production | F5 is tested with a fake `RateSource` + `RateBoardProvider`'s parse of a fixture |
| `FeedPublisher` (F5) | F10 `RateFeedLoop` | F10 waits for F5 |
| `BranchPriceSource` (F9) | F10, and `publish_service.rs` refusing BoT drafts on board branches | F10 waits for F9 |
| `PublishedRates::line_for` (F6) | `QuoteService` (F6); kiosk and bookings later | `line_in` keeps working for untiered publications (§3.4) |

## 3. Behaviour

### 3.1 Reading the board (F1, F4)

- **The ♻️ seam.** `ReferenceBatchWriter` keeps the shared core: checksum, dedupe, batch status, and the store write.
  - What differs per provider moves behind `ReferenceProvider`:
    - `parse`: column names, the currency list that makes a batch `COMPLETE`, and where the source date comes from.
    - `row_key`: the key one row is identified by. The checksum sort (`reference_batch_writer.rs:60-66`) and the one-line-per-key dedupe (`:129`) both use it. Today both use the currency `code` alone, which is right for BoT (one row per currency) and wrong for the board (several rows per currency: bands and tiers).
  - **Constructor kept.** `ReferenceBatchWriter::new(source, store)` keeps its signature and means "BoT provider". The board path adds `.with_provider(provider)`, in the same style as the existing `.with_source` (`:43`).
    - So the three existing callers compile and behave unchanged: `rework_hardening_test.rs:294`, `:388`, and `api/src/presentation/http/v3/commands/rate_reference_fetch.rs:75`.
  - `BotProvider` is today's code, moved: `row_key` = `code` (or `currency_code`) upper-cased, exactly `:60-66` and `:121-131` today. **F1's criterion is that every existing test passes with no test file touched.**
  - `RateBoardProvider::row_key` = `code` + `instrument_key` + `value_tier`. That decision sits in F4, never in the writer.
  - Adding the board = `RateBoardSource` + `RateBoardProvider`, with no copy of the writer.
- **Lines kept as received.** One reference line per board row: `instrument_key` (e.g. `USD`, `USD1`, `USD2`), `value_tier`, the tier's THB band, base and shop rates. The `unit_size` is **1**, because the board sends THB per 1 (BP §08).
  - Nothing is converted on write.
  - The conversion to the currency's `unit_size` happens once, where the line is published (§3.3), with the same formula as `rates.md` §4.2: `ref × currency.unit_size ÷ ref.unit_size`.
- **Two times.** `source_published_at` (SCB's announcement time) is stored apart from `fetched_at`. Freshness and stale policy read the announcement time (BP §08). If the payload has no announcement time, that is a Q-4 answer to record, not a default to invent.
- **Missing is missing.** A side the board does not price, hides (`HIDDEN_CONFLICT`) or has paused arrives as `NULL` with its status in `raw`. It is never 0 (BP §08, `rates.md:281`).

### 3.2 Banknote groups (F3, R-3)

- **Groups.** OWNER/admin creates a currency's groups (`denominations.group_code`), and maps each group to one `instrument_key` of a provider: `denomination_group_sources`, §4.
- **Defaults.** A currency starts with its one `'ALL'` group, as today (`rate_units.sql:32-33`). Mapping `'ALL'` to a row is what makes a currency priceable from the board.
- **Unmapped.** A group with no mapping gets **no line** at publish, so it is unavailable at the counter. *Follows from R-3 + `rates.md:281`, not ruled.*
- **Removing.** Removing a group that a current publication still prices is refused with `validationFailed`. The group stays until a publication without it is current.

### 3.3 Publishing a board batch (F5, R-2, R-4, R-5)

`FeedPublisher::publish_latest(tx, branch, provider)`:

1. **Read and stop early.** Lock `branches` `FOR UPDATE`, as `publish_service.rs` does. Read the provider's newest batch. If its checksum equals the one the current publication was built from (`source_snapshot.batch_checksum`), return `None`: nothing changed, nothing written.
2. **Build the lines.** For every currency, group and mapped `instrument_key`: one `published_rate_lines` row **per tier** the batch carries for that row.
   - **Paused board (R-6).** If the batch says the board is paused, every line is written with both sides `NULL`. Quotes are then `rateUnavailable` until a batch from a resumed board is published. How "paused" appears in the payload is part of Q-4.
   - `buy_rate` / `sell_rate` are the board's shop prices, converted per §3.1, **with no `adjust_pct`** (R-2).
   - `base_buy_rate` / `base_sell_rate` carry the board's SCB base, converted the same way, for the tier pick (§3.4).
   - `tier_lower_exclusive_thb` / `tier_upper_inclusive_thb` carry the band.
3. **Approve as SYSTEM.** The approval core is used, so `rate_publications.approved_request_id` (`NOT NULL`, `rate_engine.sql:83`) is filled through the one core (`rates.md` D-4), not a bypass. R-5: no second approval, still audited.
   - **Calls.** `ApprovalService::request(tx, "RATE_FEED_PUBLICATION", object_id, payload, system_actor, key)` (`api/crates/xc-approval/src/application/approval_service.rs:82`), then `ApprovalService::decide_self(tx, &request.id, system_actor, "rate feed")` (`:339-375`) in the same transaction.
     - **Not `decide`.** `decide` refuses a decider who is the proposer (`:156-164`). `decide_self` requires exactly that, and asks the policy.
   - **The actor.** `system_actor` is an `Actor` with `Role::System` (`role.rs:13`) and id `"SYSTEM:rate-feed:<provider>"`. The same actor proposes and decides.
   - **`RateFeedPublicationPolicy`:**
     - `allows_self_decision(who)` is true only for `Role::System` (the hook is `xc-approval/src/domain/policy.rs:41`);
     - `may_decide` refuses everyone, so no person can decide this type through `approvalDecide`;
     - `policy_version` is `"1"`.
   - **`RateFeedPublicationSubject`:**
     - `object_id` = `"<branch_id>:<batch_id>"`;
     - `key` = `"<branch_id>:<batch raw_checksum>"`, so a retried tick replays the same request (`request`'s idempotency, `:76-100`);
     - `version_hash(object_id)` = the batch's `raw_checksum` read from `rate_reference_batches`, so the request binds to exactly the rows published;
     - `on_decided` does nothing. The publication is written by `FeedPublisher` after `decide_self` returns, in the same transaction, and `Rejected` cannot happen because only `decide_self` can close the request.
   - **Registered** beside `rate_publication::register` in `make_approval_service` (`api/src/presentation/http/v3/commands/mod.rs:17-23`), and in the test engine's registry (`api/crates/xc-rate/tests/rate_engine_common/mod.rs:224-225`). Both are F5's.
   - `draft_id` is `NULL`, which the column allows (`rate_engine.sql:84`).
   - `published_by = "SYSTEM:rate-feed:<provider>"`.
4. **Insert and move the pointer.** Insert the publication at the next version, and move `branches.current_publication_id`.
5. **Record the source.** `source_snapshot` holds `provider`, `batch_id`, `batch_checksum`, the board `revision`, `source_published_at`, `fetched_at`, `instrument_key`, `value_tier` and the received row's `raw` status.
   - That is the audit: who (SYSTEM), when, and from which board revision.

**Applies to:** only the branches whose price source is the board, which is Q-7. A branch on BoT drafts keeps `publish_service.rs` unchanged.

### 3.4 Reading a price at the counter (F6)

`PublishedRates::line_for(tx, branch, LineSelector { code, side, group, quantity })`:

- **Untiered publication** (every line `value_tier = 'ALL'`, i.e. all BoT-draft publications): the line of `(code, group)`. `quantity` is not needed.
- **Tiered publication:**
  - `quantity` is required. Without it → `None` (unavailable), never a guessed tier.
  - The tier is picked on the **base** rate, before the margin (BP §02): `base_value = ExchangeArithmetic::thb(quantity, base_rate_of_side, unit_size)`, then `TierTable::pick(base_value)`. The lines of `(code, group)` with their bands form the table. The result is the line of that tier.
  - One tier for the whole transaction (BP §02). `quote_in` already takes exactly one line (`quote.rs:63-68`).
- **`line_in(tx, branch, code, side)` stays** for its three callers, and means `line_for` with `group = 'ALL'` and `quantity = None`. On a tiered publication it therefore answers `None`, the unavailable path those callers already handle. It does not silently use a wrong tier.
  - Moving the kiosk (`kiosk_ports.rs:415`) and bookings (`booking_ports.rs:110`) onto `line_for` with their quantity is follow-up work. It is listed in §6, not done in F6.
- **`QuoteRequestLine` gains `denomination_group: GroupCode`** (default `'ALL'`). `quote.rs:183` stores the group that was asked for instead of the literal.

### 3.5 Per-branch switch (F9, R-7)

- **The setting.** `branches.price_source` is `'BOT_DRAFT'` (default, today's path) or `'RATE_BOARD'`. OWNER sets it with the `branchPriceSourceSet` command, and the change is audited (who, when, from, to).
- **Switching on:** `RateFeedLoop` includes the branch from its next tick. The first tick publishes the board's prices (§3.3).
- **Switching back to `'BOT_DRAFT'`:** the loop stops publishing for the branch. The current publication stays a board publication until OWNER publishes a BoT draft.
  - `[proposed]`: the switch is refused unless an approved BoT draft is published in the same command. Otherwise board prices would keep being sold after the board stopped feeding them. **This is my proposal, not a ruling**; the reviewer or owner may change it.
- **Reading:** `publish_service.rs` (BoT drafts) refuses to publish on a `'RATE_BOARD'` branch (`validationFailed`). Two price sources never alternate on one branch.

### 3.6 The timer (F10, R-8)

- **One loop.** A single background task in the server runs `tick()` every `RATE_FEED_INTERVAL_SECS` (default 60; a setting, R-8).
- **Each tick:**
  1. **Skip if another tick is running.** It takes a Postgres advisory lock (`pg_try_advisory_lock`) and returns at once if another tick, or another server instance, holds it. No overlapping runs.
     - **Lock scope** (PR 1038 review follow-up): a session lock (`pg_try_advisory_lock`) taken on a pooled connection stays held if the tick errors before unlocking, and every later tick then skips silently. Use `pg_try_advisory_xact_lock` inside an outer transaction, so the lock ends with the transaction on every path. Or use a dedicated connection that unlocks on every path. F10's tests include: a tick that fails does not block the next one.
  2. **Fetch once.** `ReferenceBatchWriter::fetch_and_write` with the board's source and provider. A repeat checksum writes nothing (§3.1).
  3. **Publish per branch.** `FeedPublisher::publish_latest` for every branch whose `price_source = 'RATE_BOARD'`, one transaction per branch, so one branch's failure does not stop the others.
- **Board unreachable.** The fetch fails, nothing is written, and the current publication stays. Its freshness is measured from `source_published_at`, so the branch's stale policy (`rates.md` §4.3, `on_stale`) decides when quotes stop. Each failed tick is logged with the error. Whether an unreachable board should stop the counter at once, like a paused one (R-6), is **Q-9** (§7).
- **Where it runs.** v3 had no scheduler at `875f7e8` (§7 Q-8 sweep), so this is the first background task. It starts with the server and stops with it.

### 3.7 Input / output and errors

| Call | Accepts | Returns | Errors |
|---|---|---|---|
| `ReferenceProvider::parse` | `&ReferenceRates` | `ParsedBatch` | `RateEngineError::RateUnavailable` (payload unreadable); `ValidationFailed` (a rate ≤ 0, buy > sell on a row) |
| `ReferenceBatchWriter::fetch_and_write` | tx | batch id (existing id on a repeat checksum) | as today |
| `DenominationGroupService::map_group` | code, group, provider, instrument_key | `()` | `validationFailed` (unknown currency/group, empty key); `outOfScope` (not OWNER/admin) |
| `FeedPublisher::publish_latest` | tx, branch, provider | `Some(PublicationRef)` or `None` (nothing new) | `RateUnavailable` (no batch); `StaleVersion` (lost the branch lock race) |
| `PublishedRates::line_for` | `LineSelector` | `Option<PublishedSide>` | DB errors only; "no such line / no tier fits / quantity missing on a tiered publication" is `None` |
| `TierTable::pick` | `Money` | `Option<TierCode>` | — (`None` when no band contains the value, or bands overlap) |

### 3.8 Edge cases

| Case | Behaviour | Test |
|---|---|---|
| value exactly on a band edge (10,000.00) | `lower_exclusive < v ≤ upper_inclusive` → 10,000.00 is the lower tier (BP §02 "≤10,000") | F2 unit test |
| bands overlap or leave a gap | `TierTable::pick` → `None`; `FeedPublisher` refuses the batch (`ValidationFailed`) | F2 + F5 |
| board row hidden / paused / missing side | that side `NULL`; quote → `rateUnavailable` | F5 + F6 |
| whole board paused (R-6) | a publication with every side `NULL`; every quote → `rateUnavailable`; the next resumed batch publishes prices again | F5 |
| a group mapped to a key the batch lacks | no line for that group in the publication | F5 |
| same batch fetched twice | writer returns the existing batch id; publisher returns `None` | F1 (existing behaviour kept) + F5 |
| JPY: board 0.2301 per 1, `unit_size` 100 | published 23.01 per 100; quote of 10,000 JPY at base 0.22 → base value 2,200 THB → tier ≤10,000 | F5 + F6 |
| BoT-draft branch | unchanged: untiered `'ALL'` lines, `line_in` as today | F6 regression |
| two server instances tick at once | one holds the advisory lock, the other returns at once | F10 |
| board unreachable for several ticks | nothing written; quotes follow the stale policy from `source_published_at` | F10 |
| BoT draft published on a `'RATE_BOARD'` branch | refused `validationFailed` | F9 |
| kiosk/bookings on a tiered publication before their follow-up | `line_in` → `None` → their existing unavailable path | F6 |

## 4. Data migration (F2)

One migration, `api/migrations/<next>_rate_feed.sql`:

```sql
ALTER TABLE rate_reference_batches ADD COLUMN source_published_at timestamptz NULL;
ALTER TABLE rate_reference_lines
    ADD COLUMN instrument_key text NULL,
    ADD COLUMN value_tier text NULL,
    ADD COLUMN tier_lower_exclusive_thb numeric(20,2) NULL,
    ADD COLUMN tier_upper_inclusive_thb numeric(20,2) NULL,
    ADD COLUMN base_buy numeric(20,8) NULL,
    ADD COLUMN base_sell numeric(20,8) NULL;
-- the line's identity now includes the board row and the tier
ALTER TABLE rate_reference_lines DROP CONSTRAINT <existing unique of (batch_id, currency_code, reference_type)>;
CREATE UNIQUE INDEX rate_reference_lines_identity
    ON rate_reference_lines (batch_id, currency_code, reference_type,
                             COALESCE(instrument_key, ''), COALESCE(value_tier, ''));

ALTER TABLE published_rate_lines
    ADD COLUMN value_tier text NOT NULL DEFAULT 'ALL',
    ADD COLUMN tier_lower_exclusive_thb numeric(20,2) NULL,
    ADD COLUMN tier_upper_inclusive_thb numeric(20,2) NULL,
    ADD COLUMN base_buy_rate numeric(20,8) NULL,
    ADD COLUMN base_sell_rate numeric(20,8) NULL;
ALTER TABLE published_rate_lines DROP CONSTRAINT <existing unique of (publication_id, currency_code, denomination_group)>;
ALTER TABLE published_rate_lines
    ADD CONSTRAINT published_rate_lines_identity UNIQUE (publication_id, currency_code, denomination_group, value_tier);

ALTER TABLE branches
    ADD COLUMN price_source text NOT NULL DEFAULT 'BOT_DRAFT'
        CHECK (price_source IN ('BOT_DRAFT', 'RATE_BOARD'));

CREATE TABLE denomination_group_sources (
    currency_code  char(3) NOT NULL REFERENCES currencies(code),
    group_code     text    NOT NULL,
    provider       text    NOT NULL,
    instrument_key text    NOT NULL,
    updated_by     text    NOT NULL,
    updated_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (currency_code, group_code, provider)
);
```

The two existing constraint names are read from the database in F2, not guessed:
`SELECT conname FROM pg_constraint WHERE conrelid = 'published_rate_lines'::regclass AND contype = 'u';` and the same
for `rate_reference_lines`.

1. **Existing rows after it:**
   - `rate_reference_lines` (BoT): the new columns are `NULL`. They keep their identity: `COALESCE` makes `(…, '', '')` unique exactly as before.
   - `rate_reference_batches`: `source_published_at` is `NULL`. Readers keep using `source_date`.
   - `published_rate_lines`: `value_tier = 'ALL'`, bands and base rates `NULL`. Every existing publication reads as untiered (§3.4), exactly as today.
   - `denomination_group_sources` starts empty.
   - `branches.price_source` is `'BOT_DRAFT'` for every branch, which is today's behaviour. No branch is switched on by the migration.
2. **Backfill: none.** The defaults are the old meaning; nothing is computed lazily either. Rows that will carry the new defaults, to look at before deploy:
   `SELECT (SELECT count(*) FROM published_rate_lines) AS lines, (SELECT count(*) FROM rate_reference_lines) AS refs;`
   v2/v3 data is test data (`xchange-v3.md` D6), but the count still goes in the deploy note.
3. **Deploy order:** migration first, then the binary.
   - **Old binary, new schema, no tiered rows:** works. `line_in`'s query (`infrastructure/publication/published_rates.rs:77-82`) filters `denomination_group = 'ALL'`, and while every line is `value_tier = 'ALL'` there is one row. The writer's inserts name their columns, so the new nullable columns take their defaults.
   - 🔴 **Old binary, tiered rows: unsafe.** Once a tiered publication exists, the old query matches four rows, and `fetch_optional` (`:83-86`) silently returns whichever comes first: a **wrong tier's price, with no error**.
   - **The guard:** F6 adds `AND l.value_tier = 'ALL'` to `line_in`'s query. From F6's binary on, `line_in` can never read a tiered row; it answers `None` on a tiered publication (§3.4).
   - **The rule that follows:**
     - No tiered publication may be written before F6's binary is deployed. F10, the only writer, waits for F6 (§6).
     - Once tiered publications exist, **rolling the server back to a binary older than F6 is forbidden**: it would sell at a random tier.
     - The deploy note for F10 must say so, with the check `SELECT count(*) FROM published_rate_lines WHERE value_tier <> 'ALL'` (0 means a rollback is safe).
4. **What `down` loses:**
   - It drops `denomination_group_sources` (the owner's group mappings), `branches.price_source` (which branches were switched on) and the new columns.
   - It must first refuse when `SELECT count(*) FROM published_rate_lines WHERE value_tier <> 'ALL'` is not 0, because restoring the old `UNIQUE (publication_id, currency_code, denomination_group)` would fail on tiered publications. Rolling back after board publications exist means deleting those publications, which is a destructive step the owner orders, not `down`.

## 5. Not in this design

- **The board side.** Its API, credential and payload (Q-4) belong to the board's developers. `RateBoardPayload` is written once those are known.
- **Kiosk and bookings on tiers.** Moving them onto `line_for` with their quantity is a follow-up per reader (§6 F7, F8).
- **Showing tiers on screens.** Desk, staff and kiosk display of four prices per currency is a screen design that has not been started.

## 6. Tickets (one class set each; no shared files)

| # | Outcome | Waits for |
|---|---|---|
| F1 | `ReferenceProvider` extracted; `BotProvider` holds today's BoT parsing; the writer takes a provider. **Pure move: every existing reference test passes unchanged** | this design merged |
| F2 | migration §4 + `TierTable`/`TierBand`/`TierCode` with the band-edge tests | this design |
| F3 | denomination groups: trait, Pg impl, service, admin commands (API) | F2 |
| F3w | admin screen for groups and mappings | F3 |
| F4 | `RateBoardSource` + `RateBoardProvider` + `RateBoardPayload` | F1, F2, **Q-4** |
| F5 | `FeedPublisher` + `RATE_FEED_PUBLICATION` subject/policy | F1, F2, F3 |
| F6 | `PublishedRates::line_for` + `LineSelector` + tier pick; `QuoteRequestLine.denomination_group`; `quote_in` step 3 | F2 |
| F7 | kiosk onto `line_for` with the requested quantity (`kiosk_ports.rs:415`) | F6 |
| F8 | bookings onto `line_for` (`booking_ports.rs:110`) | F6 |
| F9 | per-branch switch: `BranchPriceSource`, Pg impl, `branchPriceSourceSet`, BoT publish refused on board branches | F2 |
| F10 | `RateFeedLoop`: timer, advisory lock, one fetch per tick, publish per switched-on branch | F4, F5, **F6** (the `line_in` guard, §4 item 3), F9 |

Each ticket comes with its behaviour table and the failing tests its author writes first (🧪). None is released before this
design is reviewed and merged (📐).

**🔴 Release gate: switching a branch onto the board.** No branch is set to `'RATE_BOARD'` (F9's command) until **F6, F7 and F8 are merged and deployed**.
- Before F7/F8, the kiosk (`kiosk_ports.rs:415`) and bookings (`booking_ports.rs:110`) read through `line_in`, which answers `None` on a tiered publication (§3.4). On that branch they would show "unavailable" for every currency.
- **Enforcement:** the gate is checked by whoever switches a branch, at the time of switching. The code does not know what is deployed. F9's ticket repeats this gate in its done criteria and in the command's description.

## 7. Must ask — not decided here

| # | Question | Why it blocks |
|---|---|---|
| Q-4 | The board API: URL, server credential, payload; whether all four tiers come in one response; how hidden/paused/missing sides look | F4 is written against it. R-4 needs all tiers in one response; if the board sends one tier, the board needs a change first |
| Q-6 | When the board is paused, does the counter stop selling? | **Answered — R-6: it stops** (§3.3 step 2) |
| Q-7 | Which branches take their price from the board? | **Answered — R-7: per branch** (§3.5) |
| Q-8 | What triggers `FeedPublisher`? | **Answered — R-8: v3's own timer, ~1 minute, configurable** (§3.6) |
| Q-9 | When the board is **unreachable** (fetches fail), does the counter stop at once, like a paused board (R-6)? Or keep the last publication until the branch's stale policy stops it (§3.6, today's design)? | Proposed to the owner 2026-10-10: keep the stale policy, so a brief network drop does not stop the counter. Not yet answered |
