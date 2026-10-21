-- XC-PRICING #4324 (pricing-tiers.md §4, §5.2, §12.1). Additive only; approved by the owner 2026-10-22
-- (#564 issuecomment-6323680134). Neither table feeds the draft path, so no price changes.

-- V.9's per-currency `better` % (src/pricing/store.mjs:10-11): '*' is the branch default, else a currency code.
CREATE TABLE pricing_suggestion_settings (
    branch_id  text           NOT NULL REFERENCES branches(id),
    scope      text           NOT NULL CHECK (scope = '*' OR scope ~ '^[A-Z]{3}$'),
    better_pct numeric(9,4)[] NOT NULL CHECK (array_length(better_pct, 1) = 4),
    updated_by text           NOT NULL,
    updated_at timestamptz    NOT NULL,
    PRIMARY KEY (branch_id, scope)
);

-- Each market source's last fetch attempt and error (V.9 src/pricing/market.mjs:99-113).
CREATE TABLE market_feed_status (
    source          text        PRIMARY KEY CHECK (source IN ('TT', 'SR')),
    last_attempt_at timestamptz NOT NULL,
    last_error      text        NULL
);
