-- #1157 (design docs/design/blueprint/org-roles-branches.md §9.4, B6): no invite exists anywhere.
-- The shop owner (or an ADMIN) creates a staff member's Identity account server-side, so:
--   * the `user_invites` table goes;
--   * the user status INVITED is renamed PENDING ("created, no Identity account bound yet").
--
-- Existing rows: every INVITED user becomes PENDING; the owner completes it by running the create again
-- for that user. user_invites rows are dropped with the table (no code writes them any more).
-- Measure before deploy: SELECT count(*) FROM user_invites; SELECT count(*) FROM users WHERE status = 'INVITED';
-- An older binary on this schema is NOT safe (it writes 'INVITED' and reads user_invites).
--
-- Reversal (run by hand before starting an older binary; the invite rows are lost):
--   ALTER TABLE users DROP CONSTRAINT users_status_check;
--   UPDATE users SET status = 'INVITED' WHERE status = 'PENDING';
--   ALTER TABLE users ADD CONSTRAINT users_status_check CHECK (status IN ('INVITED', 'ACTIVE', 'DISABLED'));
--   CREATE TABLE user_invites (user_id uuid PRIMARY KEY REFERENCES users(id), code_hash text NOT NULL UNIQUE,
--                              expires_at timestamptz NOT NULL, used_at timestamptz NULL);

DROP TABLE user_invites;

-- The CHECK was declared inline in 20261011110000_users_roles.sql, so its name is Postgres's; find it.
DO $$
DECLARE
    con text;
BEGIN
    FOR con IN
        SELECT c.conname
        FROM pg_constraint c
        WHERE c.conrelid = 'users'::regclass
          AND c.contype = 'c'
          AND pg_get_constraintdef(c.oid) LIKE '%INVITED%'
    LOOP
        EXECUTE format('ALTER TABLE users DROP CONSTRAINT %I', con);
    END LOOP;
END $$;

UPDATE users SET status = 'PENDING' WHERE status = 'INVITED';

ALTER TABLE users ADD CONSTRAINT users_status_check CHECK (status IN ('PENDING', 'ACTIVE', 'DISABLED'));
