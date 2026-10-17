-- TX-9 (#677): migration 3 — drop wide figure and payment columns from transactions.
-- Design: docs/design/blueprint/quote-transaction.md §8 item 3.

-- 1. Precheck: every transaction must have at least one transaction_lines row;
--    log any SLIP transaction_files rows whose file_key does not match transaction_payments.slip_file_key.
DO $$
DECLARE
    kept_slips bigint;
BEGIN
    IF EXISTS (
        SELECT 1 FROM transactions t
        WHERE NOT EXISTS (
            SELECT 1 FROM transaction_lines l
            WHERE l.transaction_id = t.id
        )
    ) THEN
        RAISE EXCEPTION 'transactions row without transaction_lines row';
    END IF;

    SELECT count(*) INTO kept_slips
    FROM transaction_files f
    WHERE f.kind = 'SLIP'
      AND NOT EXISTS (
          SELECT 1 FROM transaction_payments p
          WHERE p.transaction_id = f.transaction_id
            AND p.slip_file_key = f.file_key
      );
    RAISE NOTICE 'kept % unmatched SLIP transaction_files rows', kept_slips;
END $$;

-- 2. Delete SLIP rows in transaction_files whose file_key matches that exchange's transaction_payments.slip_file_key.
DELETE FROM transaction_files f
WHERE f.kind = 'SLIP'
  AND EXISTS (
      SELECT 1 FROM transaction_payments p
      WHERE p.transaction_id = f.transaction_id
        AND p.slip_file_key = f.file_key
  );

-- 3. Drop the 15 wide figure and payment columns on transactions (booking_no stays until BK-11 #1413).
ALTER TABLE transactions
    DROP COLUMN type,
    DROP COLUMN code,
    DROP COLUMN amount,
    DROP COLUMN rate,
    DROP COLUMN reference_rate,
    DROP COLUMN unit_size,
    DROP COLUMN principal,
    DROP COLUMN fee,
    DROP COLUMN bank_fee,
    DROP COLUMN received,
    DROP COLUMN paid,
    DROP COLUMN method,
    DROP COLUMN payment_ref,
    DROP COLUMN account,
    DROP COLUMN published_rate_line_id;

-- down:
-- ALTER TABLE transactions
--     ADD COLUMN type text NULL CHECK (type IN ('BUY', 'SELL')),
--     ADD COLUMN code char(3) NULL,
--     ADD COLUMN amount numeric(20,2) NULL CHECK (amount > 0),
--     ADD COLUMN rate numeric(20,8) NULL CHECK (rate > 0),
--     ADD COLUMN reference_rate numeric(20,8) NULL CHECK (reference_rate > 0),
--     ADD COLUMN unit_size numeric(20,4) NULL,
--     ADD COLUMN principal numeric(20,2) NULL,
--     ADD COLUMN fee numeric(20,2) NULL,
--     ADD COLUMN bank_fee numeric(20,2) NULL,
--     ADD COLUMN received numeric(20,2) NULL,
--     ADD COLUMN paid numeric(20,2) NULL,
--     ADD COLUMN method text NULL,
--     ADD COLUMN payment_ref text NULL,
--     ADD COLUMN account text NULL,
--     ADD COLUMN published_rate_line_id uuid NULL REFERENCES published_rate_lines (id);
-- UPDATE transactions t
-- SET type = l.shop_side,
--     code = l.currency,
--     amount = l.quantity::numeric(20,2),
--     rate = l.rate_snapshot,
--     reference_rate = l.reference_rate,
--     unit_size = l.unit_size,
--     principal = l.thb_amount,
--     fee = l.fee_thb,
--     bank_fee = p.bank_fee_thb,
--     received = CASE l.shop_side WHEN 'SELL' THEN l.settle_thb ELSE 0.00 END,
--     paid = CASE l.shop_side WHEN 'BUY' THEN l.settle_thb ELSE 0.00 END,
--     method = p.method,
--     payment_ref = p.reference,
--     account = p.account,
--     published_rate_line_id = ql.published_rate_line_id
-- FROM transaction_lines l
-- LEFT JOIN transaction_payments p
--   ON p.transaction_id = l.transaction_id AND p.payment_no = 1
-- LEFT JOIN quote_lines ql
--   ON ql.id = l.quote_line_id
-- WHERE l.transaction_id = t.id AND l.line_no = 1;
