-- CM-3 (#704 Part 2): key idempotency_records by real organization_id instead of org_id placeholder

DO $$
DECLARE
    dup_count integer;
BEGIN
    SELECT count(*) INTO dup_count FROM (
        SELECT organization_id, operation, key
        FROM idempotency_records
        GROUP BY 1, 2, 3
        HAVING count(*) > 1
    ) d;
    IF dup_count > 0 THEN
        RAISE EXCEPTION 'duplicate (organization_id, operation, key) in idempotency_records';
    END IF;
END $$;

ALTER TABLE idempotency_records DROP CONSTRAINT idempotency_records_pkey;
ALTER TABLE idempotency_records ADD PRIMARY KEY (organization_id, operation, key);
ALTER TABLE idempotency_records DROP COLUMN org_id;
