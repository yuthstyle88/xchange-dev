-- CR-b (#5122): quotes.cash_settlement plus the two CHECKs on quote_lines and transaction_lines (§13.6).

ALTER TABLE quotes ADD COLUMN cash_settlement boolean NOT NULL DEFAULT false;
ALTER TABLE quote_lines ADD CONSTRAINT quote_lines_rounding_within_a_coin CHECK (
    abs(settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END) < 0.25);
ALTER TABLE transaction_lines ADD CONSTRAINT transaction_lines_rounding_within_a_coin CHECK (
    abs(settle_thb - CASE shop_side WHEN 'BUY' THEN thb_amount - fee_thb ELSE thb_amount + fee_thb END) < 0.25);
