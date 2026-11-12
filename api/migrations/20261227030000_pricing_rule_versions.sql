-- The Owner's pricing rules (docs/design/blueprint/owner-set-values.md §2, §5; xchange#6555). Plan approved by the
-- owner on 2026-11-09 08:09 ICT ("อนุมัติ", issuecomment-6729853935).
-- Additive: a new table, nothing else changes. The constraint changes of §5 belong to the GA-3 and GA-4 tickets.
--
-- One row per save, never updated. The organisation's current rules are its newest row; no row means "not set".
-- Every rule column is NULL-able, and NULL means "not set". No row is seeded: the Owner enters the values.
CREATE TABLE pricing_rule_versions (
    version                     text        PRIMARY KEY,
    organization_id             uuid        NOT NULL REFERENCES organizations(id),
    far_from_anchor_limit_pct   numeric(9,4) NULL CHECK (far_from_anchor_limit_pct > 0),
    large_move_pct              numeric(9,4) NULL CHECK (large_move_pct > 0),
    rate_decimals               jsonb        NULL,
    thb_cash_step               numeric(6,2) NULL CHECK (thb_cash_step > 0),
    branch_bp_cap               integer      NULL CHECK (branch_bp_cap > 0),
    better_pct_min              numeric(9,4) NULL,
    better_pct_max              numeric(9,4) NULL,
    min_spread_pct_min          numeric(9,4) NULL CHECK (min_spread_pct_min >= 0),
    min_spread_pct_max          numeric(9,4) NULL,
    market_stale_after_minutes  integer      NULL CHECK (market_stale_after_minutes > 0),
    advice_far_pct              numeric(9,4) NULL CHECK (advice_far_pct > 0),
    advice_spread_ratio         numeric(9,4) NULL CHECK (advice_spread_ratio > 0),
    advice_spread_gap_pt        numeric(9,4) NULL CHECK (advice_spread_gap_pt > 0),
    advice_market_moved_pct     numeric(9,4) NULL CHECK (advice_market_moved_pct > 0),
    set_by                      text        NOT NULL,
    created_at                  timestamptz NOT NULL,
    CHECK (better_pct_min IS NULL OR better_pct_max IS NULL OR better_pct_min <= better_pct_max),
    CHECK (min_spread_pct_min IS NULL OR min_spread_pct_max IS NULL OR min_spread_pct_min <= min_spread_pct_max)
);
CREATE INDEX pricing_rule_versions_current ON pricing_rule_versions (organization_id, created_at DESC);
