-- XC-V9-PRICING (pricing-tiers.md §15.2). Additive only; nothing is backfilled, so every existing currency stays
-- AUTO until someone approves it. v3 migrations are approved until the first install (owner, 2026-10-06).

-- Approved and followed tier sets, append-only (V9:store pricing_tiers).
CREATE TABLE pricing_tier_sets (
    id               uuid        PRIMARY KEY,
    request_id       text        NOT NULL UNIQUE,
    branch_id        text        NOT NULL REFERENCES branches(id),
    currency_code    char(3)     NOT NULL REFERENCES currencies(code),
    source           text        NOT NULL CHECK (source IN ('MANUAL', 'MAIN_PUSH', 'FOLLOW')),
    base_tier_set_id uuid        NULL REFERENCES pricing_tier_sets(id),  -- MAIN's set a branch row was adjusted from
    tiers            jsonb       NOT NULL,          -- [{tier,buy,sell}] × 4, decimal strings, per unit_size
    anchor           jsonb       NULL,              -- {source, buy, sell, fetched_at, batch_id}
    flags            text[]      NOT NULL DEFAULT '{}',
    draft_id         uuid        NULL REFERENCES rate_drafts(id),
    approved_by      text        NOT NULL,
    approved_at      timestamptz NOT NULL
);
CREATE INDEX pricing_tier_sets_latest ON pricing_tier_sets (branch_id, currency_code, approved_at DESC);

-- FOLLOW per currency, MAIN only (V9:automation-ledger; absence = MANUAL/AUTO).
CREATE TABLE pricing_modes (
    currency_code char(3)     PRIMARY KEY REFERENCES currencies(code),
    mode          text        NOT NULL CHECK (mode IN ('FOLLOW', 'MANUAL')),
    revision      bigint      NOT NULL,
    updated_by    text        NOT NULL,
    updated_at    timestamptz NOT NULL
);

-- How a branch follows MAIN (V9:store pricing_branch_policy). currency_code '*' is the branch default.
CREATE TABLE pricing_branch_policy (
    branch_id     text        NOT NULL REFERENCES branches(id),
    currency_code text        NOT NULL CHECK (currency_code = '*' OR currency_code ~ '^[A-Z]{3}$'),
    buy_bp        integer     NULL CHECK (buy_bp BETWEEN -300 AND 300),
    sell_bp       integer     NULL CHECK (sell_bp BETWEEN -300 AND 300),
    own           boolean     NOT NULL DEFAULT false,
    follow        boolean     NOT NULL DEFAULT false,  -- explicit FOLLOW enrolment (H: §7 last bullet)
    updated_by    text        NOT NULL,
    updated_at    timestamptz NOT NULL,
    PRIMARY KEY (branch_id, currency_code)
);

-- What the storefront shows (V9:store rate_display_settings): a branch row, else MAIN's, else the default.
CREATE TABLE rate_display_settings (
    branch_id  text        PRIMARY KEY REFERENCES branches(id),
    tiers      smallint[]  NOT NULL CHECK (cardinality(tiers) BETWEEN 1 AND 4),
    featured   char(3)[]   NOT NULL CHECK (cardinality(featured) <= 10),
    highlight  boolean     NOT NULL,
    all_market boolean     NOT NULL,
    updated_by text        NOT NULL,
    updated_at timestamptz NOT NULL
);

-- A draft line can carry four manual tier prices (§15.3).
CREATE TABLE rate_draft_line_tiers (
    draft_line_id uuid          NOT NULL REFERENCES rate_draft_lines(id) ON DELETE CASCADE,
    tier_no       smallint      NOT NULL CHECK (tier_no BETWEEN 1 AND 4),
    buy           numeric(20,8) NOT NULL CHECK (buy > 0),
    sell          numeric(20,8) NOT NULL CHECK (sell > buy),
    PRIMARY KEY (draft_line_id, tier_no)
);

-- The latest FOLLOW plan per currency at MAIN (§15.5), read by the board and the public projection.
CREATE TABLE pricing_follow_results (
    currency_code   char(3)     PRIMARY KEY REFERENCES currencies(code),
    status          text        NOT NULL CHECK (status IN ('CANDIDATE', 'BLOCKED', 'PUBLISHED')),
    reasons         text[]      NOT NULL DEFAULT '{}',
    anchor_batch_id uuid        NULL,
    tier_set_id     uuid        NULL REFERENCES pricing_tier_sets(id),
    planned_at      timestamptz NOT NULL
);

-- minSpreadPct at the branch '*' level (gap 10).
ALTER TABLE pricing_suggestion_settings ADD COLUMN min_spread_pct numeric(9,4) NULL
    CHECK (min_spread_pct IS NULL OR min_spread_pct BETWEEN 0 AND 5);
