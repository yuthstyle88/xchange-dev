-- Per-currency tier margins and the configurable AUTO anchor (pricing-tiers.md §3, §3.1, §12; owner P-1, P-4, P-5).
--
-- Existing rows: every rate_policy_versions row gets anchor_source = 'BOT' (the default), which is what the draft path
-- reads today; every version has zero override rows, so every currency resolves to the branch default. No backfill.
-- Deploy: this migration before the new binary (migrations run at boot); the old binary never reads the new tables.
-- down:
--   ALTER TABLE rate_policy_versions DROP COLUMN anchor_source;
--   DROP TABLE rate_policy_currency_settings; DROP TABLE rate_policy_currency_tiers;
--   It loses every per-currency override, min spread and anchor choice; policy versions survive and price from BoT.

CREATE TABLE rate_policy_currency_tiers (
    policy_version_id uuid         NOT NULL REFERENCES rate_policy_versions(id),
    currency_code     char(3)      NOT NULL REFERENCES currencies(code),
    tier_no           smallint     NOT NULL CHECK (tier_no BETWEEN 1 AND 4),
    buy_margin_pct    numeric(9,4) NOT NULL,
    sell_margin_pct   numeric(9,4) NOT NULL,
    PRIMARY KEY (policy_version_id, currency_code, tier_no)
);

CREATE TABLE rate_policy_currency_settings (
    policy_version_id uuid         NOT NULL REFERENCES rate_policy_versions(id),
    currency_code     char(3)      NOT NULL REFERENCES currencies(code),
    min_spread_pct    numeric(9,4) NULL CHECK (min_spread_pct IS NULL OR min_spread_pct BETWEEN 0 AND 5),
    anchor_source     text         NULL
        CHECK (anchor_source IS NULL OR anchor_source IN ('BOT', 'MARKET_TT', 'MARKET_SR')),
    PRIMARY KEY (policy_version_id, currency_code)
);

ALTER TABLE rate_policy_versions
    ADD COLUMN anchor_source text NOT NULL DEFAULT 'BOT'
        CHECK (anchor_source IN ('BOT', 'MARKET_TT', 'MARKET_SR'));
