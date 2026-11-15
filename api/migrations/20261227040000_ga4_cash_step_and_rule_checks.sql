-- GA-4 (xchange#6928): the THB cash step is the Owner's rule and every line records it, and the DB stops
-- repeating the bounds the Owner's rules now set (docs/design/blueprint/owner-set-values.md §5, row 5, §7).
-- Data Risk: approved with GA-1's plan (§5); the backfill is §5 item 2. No pricing rule is seeded: a rule is
-- a business value the Owner enters on the card.

-- 1. The step each line was settled with. NULL = no cash rounding.
ALTER TABLE quote_lines ADD COLUMN cash_step_thb numeric(6,2) NULL CHECK (cash_step_thb > 0);
ALTER TABLE transaction_lines ADD COLUMN cash_step_thb numeric(6,2) NULL CHECK (cash_step_thb > 0);

-- 2. Backfill (§5 item 2): every line that was rounded was rounded under the code step 0.25. This records
--    history, not a business value. A line that was not rounded keeps NULL.
UPDATE quote_lines SET cash_step_thb = 0.25
WHERE settle_thb <> CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END;
UPDATE transaction_lines SET cash_step_thb = 0.25
WHERE settle_thb <> CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END;

-- 3. The line CHECKs, under the same names (tests drop them by name): no step means no rounding, and a
--    rounding stays inside the line's own step.
ALTER TABLE quote_lines DROP CONSTRAINT quote_lines_rounding_within_a_coin;
ALTER TABLE quote_lines ADD CONSTRAINT quote_lines_rounding_within_a_coin CHECK (
    (cash_step_thb IS NULL
        AND settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END = 0)
    OR (cash_step_thb IS NOT NULL
        AND abs(settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END)
            < cash_step_thb));
ALTER TABLE transaction_lines DROP CONSTRAINT transaction_lines_rounding_within_a_coin;
ALTER TABLE transaction_lines ADD CONSTRAINT transaction_lines_rounding_within_a_coin CHECK (
    (cash_step_thb IS NULL
        AND settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END = 0)
    OR (cash_step_thb IS NOT NULL
        AND abs(settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END)
            < cash_step_thb));

-- 4. The branch bp cap is the Owner's branch_bp_cap, checked by the save (row 8). The names were confirmed
--    from pg_constraint: the CHECKs were declared inline in 20261226100000_v9_pricing.sql.
ALTER TABLE pricing_branch_policy DROP CONSTRAINT pricing_branch_policy_buy_bp_check;
ALTER TABLE pricing_branch_policy DROP CONSTRAINT pricing_branch_policy_sell_bp_check;

-- 5. The minimum spread range is the Owner's, checked by the save (row 9); the DB keeps only "not negative".
ALTER TABLE rate_policy_currency_settings DROP CONSTRAINT rate_policy_currency_settings_min_spread_pct_check;
ALTER TABLE rate_policy_currency_settings ADD CONSTRAINT rate_policy_currency_settings_min_spread_pct_check
    CHECK (min_spread_pct IS NULL OR min_spread_pct >= 0);
ALTER TABLE pricing_suggestion_settings DROP CONSTRAINT pricing_suggestion_settings_min_spread_pct_check;
ALTER TABLE pricing_suggestion_settings ADD CONSTRAINT pricing_suggestion_settings_min_spread_pct_check
    CHECK (min_spread_pct IS NULL OR min_spread_pct >= 0);

-- 6. better_pct may be "not set" (row 6). pricing_suggestion_settings_better_pct_check (4 elements) stays; it
--    already passes for NULL.
ALTER TABLE pricing_suggestion_settings ALTER COLUMN better_pct DROP NOT NULL;
