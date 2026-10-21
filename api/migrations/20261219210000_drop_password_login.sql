-- OR-7.m M3 (#1027 / #3514, design docs/design/blueprint/or7m-password-login-removal.md §5):
-- Drop v3's password-login tables (employee_sessions, owner_sessions, employees, owners) and
-- users.legacy_employee_id.
--
-- Data migration (design §5):
-- 1. Existing rows: owners, owner_sessions, employees, employee_sessions and their rows go;
--    users loses legacy_employee_id. employee_audit and auth_attempts stay unchanged.
-- 2. Backfill: none. The precheck below refuses while any employee has no matching users row
--    (migration 2 mirrored them).
-- 3. Deploy order: deploy this migration with the binary that no longer reads the dropped
--    tables/column; the old binary must not run after it.
-- 4. What down loses: v3 migrations here have no down file; rolling back would lose every
--    password hash, every old session, and the employee-to-user link (legacy_employee_id) for good.

DO $$
DECLARE
    unmirrored bigint;
BEGIN
    SELECT count(*)
      INTO unmirrored
      FROM employees e
     WHERE NOT EXISTS (
         SELECT 1 FROM users u WHERE u.legacy_employee_id = e.id
     );

    IF unmirrored > 0 THEN
        RAISE EXCEPTION 'OR-7.m M3 precheck failed: % employees have no matching users row; employees must be mirrored into users first',
            unmirrored;
    END IF;
END $$;

DROP TABLE employee_sessions;
DROP TABLE owner_sessions;
DROP TABLE employees;
DROP TABLE owners;

ALTER TABLE users DROP COLUMN legacy_employee_id;
