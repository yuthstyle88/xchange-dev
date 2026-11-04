-- Backdated keying (docs/design/backdated-keying.md §10): a LIVE sale keyed after the fact by an Owner or Branch
-- Manager has no shift. Owner approval required before this merges to main (fleet rule); deploy is a separate order.
ALTER TABLE transactions ADD COLUMN backdated boolean NOT NULL DEFAULT false;

-- Replace the unnamed CHECK ((mode = 'LIVE') = (shift_id IS NOT NULL)) of 20261007190000_transactions.sql:35,
-- found by its definition so the drop does not depend on the generated name.
DO $$
DECLARE con text;
BEGIN
    SELECT c.conname INTO con
    FROM pg_constraint c
    WHERE c.conrelid = 'transactions'::regclass
      AND c.contype = 'c'
      AND pg_get_constraintdef(c.oid) ILIKE '%mode%LIVE%shift_id IS NOT NULL%';
    IF con IS NULL THEN
        RAISE EXCEPTION 'transactions mode/shift check not found';
    END IF;
    EXECUTE format('ALTER TABLE transactions DROP CONSTRAINT %I', con);
END $$;

ALTER TABLE transactions ADD CONSTRAINT transactions_shift_by_mode_check CHECK (
       (mode = 'DEMO' AND shift_id IS NULL     AND NOT backdated)
    OR (mode = 'LIVE' AND shift_id IS NOT NULL AND NOT backdated)
    OR (mode = 'LIVE' AND shift_id IS NULL     AND backdated)
);

CREATE INDEX transactions_backdated_day_idx ON transactions (branch_id, value_date) WHERE backdated;

-- Down (by hand; sqlx has no down step). Refuse while any backdated row exists, because the old CHECK cannot hold it:
-- DO $$ BEGIN IF EXISTS (SELECT 1 FROM transactions WHERE backdated) THEN
--     RAISE EXCEPTION 'backdated transactions exist; down would lose them'; END IF; END $$;
-- DROP INDEX transactions_backdated_day_idx;
-- ALTER TABLE transactions DROP CONSTRAINT transactions_shift_by_mode_check;
-- ALTER TABLE transactions ADD CHECK ((mode = 'LIVE') = (shift_id IS NOT NULL));
-- ALTER TABLE transactions DROP COLUMN backdated;
