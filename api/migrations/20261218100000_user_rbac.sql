-- #3098 (design docs/design/blueprint/user-management-rbac.md §4): RBAC migration M.
-- Adds SUSPENDED to users.status, staff profile columns on users, must_change_password on user_sessions,
-- before/after/ip on user_audit, and the login_history table.

ALTER TABLE users
    ADD COLUMN employee_code text NULL,
    ADD COLUMN department text NULL,
    ADD COLUMN position text NULL,
    ADD COLUMN tell_code text NULL;

CREATE UNIQUE INDEX users_org_employee_code_key ON users (organization_id, lower(employee_code));

ALTER TABLE users DROP CONSTRAINT users_status_check;
ALTER TABLE users ADD CONSTRAINT users_status_check
    CHECK (status IN ('PENDING', 'ACTIVE', 'SUSPENDED', 'DISABLED'));

ALTER TABLE user_sessions
    ADD COLUMN must_change_password boolean NOT NULL DEFAULT false;

ALTER TABLE user_audit
    ADD COLUMN before jsonb NULL,
    ADD COLUMN after jsonb NULL,
    ADD COLUMN ip inet NULL;

CREATE TABLE login_history (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NULL REFERENCES organizations(id),
    user_id uuid NULL REFERENCES users(id) ON DELETE SET NULL,
    identifier_hash text NOT NULL,
    ip inet NULL,
    result text NOT NULL CHECK (result IN (
        'OK',
        'BAD_PASSWORD',
        'UNKNOWN_USER',
        'NOT_ACTIVE',
        'TEMP_EXPIRED',
        'RATE_LIMITED',
        'UNAVAILABLE'
    )),
    at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX login_history_org_at_idx ON login_history (organization_id, at DESC);

-- Measure before deploy: SELECT count(*) FROM users; SELECT count(*) FROM user_sessions; SELECT count(*) FROM user_audit;
-- Today's binary keeps working on this schema before any SUSPENDED row exists. Rolling back to a pre-C1 binary while
-- any SUSPENDED user exists is hazardous (pre-C1 reads unknown status as Pending); set SUSPENDED users to DISABLED first.
--
-- Reversal (run by hand; loses profile fields, forced-change flags, audit before/after/ip, and all login_history rows):
--   -- Must return 0 before proceeding; never guess ACTIVE or DISABLED:
--   SELECT count(*) FROM users WHERE status = 'SUSPENDED';
--   DROP INDEX IF EXISTS login_history_org_at_idx;
--   DROP TABLE IF EXISTS login_history;
--   ALTER TABLE user_audit DROP COLUMN IF EXISTS ip, DROP COLUMN IF EXISTS after, DROP COLUMN IF EXISTS before;
--   ALTER TABLE user_sessions DROP COLUMN IF EXISTS must_change_password;
--   ALTER TABLE users DROP CONSTRAINT users_status_check;
--   ALTER TABLE users ADD CONSTRAINT users_status_check CHECK (status IN ('PENDING', 'ACTIVE', 'DISABLED'));
--   DROP INDEX IF EXISTS users_org_employee_code_key;
--   ALTER TABLE users
--       DROP COLUMN IF EXISTS tell_code,
--       DROP COLUMN IF EXISTS position,
--       DROP COLUMN IF EXISTS department,
--       DROP COLUMN IF EXISTS employee_code;
