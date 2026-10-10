-- CA-3 (#619 Part 2): M2 - shifts per teller (option A)
-- Design: docs/design/blueprint/cash-shift-transfer.md §4.1, §8 M2

ALTER TABLE shifts
    ADD COLUMN opened_by_count uuid NULL REFERENCES cash_counts(id);

DROP INDEX shifts_one_open_per_branch;

CREATE UNIQUE INDEX shifts_one_open_per_teller ON shifts (teller_id)
    WHERE status = 'OPEN' AND opened_by_count IS NOT NULL;

CREATE UNIQUE INDEX shifts_one_open_per_branch_v2 ON shifts (branch_id)
    WHERE status = 'OPEN' AND opened_by_count IS NULL;
