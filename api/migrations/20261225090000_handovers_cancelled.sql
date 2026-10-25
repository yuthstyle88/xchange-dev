-- BE-3 (#5107): widen handovers.status to include CANCELLED so the giver may cancel a PENDING hand-over,
-- and enforce at most one PENDING hand-over per shift.

ALTER TABLE handovers
    DROP CONSTRAINT handovers_status_check,
    ADD CONSTRAINT handovers_status_check
        CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED', 'CANCELLED'));

CREATE UNIQUE INDEX handovers_one_pending_per_shift ON handovers (from_shift_id) WHERE status = 'PENDING';

