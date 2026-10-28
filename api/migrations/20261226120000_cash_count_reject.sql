-- SC-0 (staff-corrections design §6, M-SC1; owner approved 2026-10-28, #564): a verifier rejects a waiting count.
-- Additive: every existing row keeps all three columns NULL and reads WAITING or VERIFIED as before. No backfill.
-- Count before deploy: SELECT count(*) FILTER (WHERE verified_by IS NULL) AS waiting, count(*) AS total FROM cash_counts;

ALTER TABLE cash_counts
    ADD COLUMN rejected_by uuid REFERENCES users (id),
    ADD COLUMN rejected_at timestamptz,
    ADD COLUMN reject_reason text,
    ADD CONSTRAINT cash_counts_rejected_complete
        CHECK ((rejected_by IS NULL) = (rejected_at IS NULL) AND (rejected_by IS NULL) = (reject_reason IS NULL)),
    ADD CONSTRAINT cash_counts_verified_or_rejected
        CHECK (verified_by IS NULL OR rejected_by IS NULL),
    ADD CONSTRAINT cash_counts_rejecter_not_counter
        CHECK (rejected_by IS NULL OR rejected_by <> counted_by);

-- down:
-- ALTER TABLE cash_counts
--     DROP CONSTRAINT cash_counts_rejecter_not_counter,
--     DROP CONSTRAINT cash_counts_verified_or_rejected,
--     DROP CONSTRAINT cash_counts_rejected_complete,
--     DROP COLUMN reject_reason,
--     DROP COLUMN rejected_at,
--     DROP COLUMN rejected_by;
