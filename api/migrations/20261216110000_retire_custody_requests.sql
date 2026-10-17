-- CA-10 (#626 / #2955, cash-shift-transfer.md §8 migration 3):
-- Retire v2 custody requests, custodian/enabled columns on ops_state, and the
-- M2 dual-mode shift marker (shifts.opened_by_count).

DO $$
DECLARE
    bad_teller uuid;
    open_count bigint;
BEGIN
    SELECT teller_id, count(*)
      INTO bad_teller, open_count
      FROM shifts
     WHERE status = 'OPEN'
     GROUP BY 1
    HAVING count(*) > 1
     LIMIT 1;

    IF bad_teller IS NOT NULL THEN
        RAISE EXCEPTION 'M3 precheck failed: teller % holds % OPEN shifts; close extra OPEN shifts before retiring custody requests',
            bad_teller, open_count;
    END IF;
END $$;

DROP TABLE custody_signatures;
DROP TABLE custody_requests;

ALTER TABLE ops_state
    DROP COLUMN custodian_id,
    DROP COLUMN enabled;

DROP INDEX shifts_one_open_per_branch_v2;
DROP INDEX shifts_one_open_per_teller;

ALTER TABLE shifts
    DROP COLUMN opened_by_count;

CREATE UNIQUE INDEX shifts_one_open_per_teller
    ON shifts (teller_id)
    WHERE status = 'OPEN';
