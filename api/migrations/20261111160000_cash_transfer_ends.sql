-- Migration: 20261111110000_cash_transfer_ends.sql
-- Add per-kind CHECK constraint on cash_transfers tying ends to kind (CA-5 + CA-6 #621)

ALTER TABLE cash_transfers ADD CONSTRAINT cash_transfers_ends_check CHECK (
    (kind = 'BRANCH' AND to_branch_id IS NOT NULL AND from_shift_id IS NULL AND to_shift_id IS NULL) OR
    (kind = 'DRAWER_TO_SAFE' AND to_branch_id IS NULL AND from_shift_id IS NOT NULL AND to_shift_id IS NULL) OR
    (kind = 'SAFE_TO_DRAWER' AND to_branch_id IS NULL AND from_shift_id IS NULL AND to_shift_id IS NOT NULL) OR
    (kind = 'EXTERNAL_IN' AND to_branch_id IS NULL AND from_shift_id IS NULL AND to_shift_id IS NULL) OR
    (kind = 'EXTERNAL_OUT' AND to_branch_id IS NULL AND from_shift_id IS NULL AND to_shift_id IS NULL)
);
