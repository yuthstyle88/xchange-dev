-- OR-2: users, scoped roles, and migration 2 backfill

DO $$
DECLARE
    unknown_role_count integer;
    orphan_branch_count integer;
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'employees') THEN
        SELECT count(*) INTO unknown_role_count
        FROM employees
        WHERE role NOT IN ('FRONT', 'BACKEND', 'MANAGER', 'RATES', 'FINANCE', 'SYSTEM', 'MONITOR', 'DOCUMENTS', 'DOCUMENT_REVIEW');

        IF unknown_role_count > 0 THEN
            RAISE EXCEPTION 'unknown employee role';
        END IF;

        IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'branches') THEN
            SELECT count(*) INTO orphan_branch_count
            FROM employees e
            WHERE NOT EXISTS (SELECT 1 FROM branches b WHERE b.id = e.branch_id);

            IF orphan_branch_count > 0 THEN
                RAISE EXCEPTION 'employee branch does not exist';
            END IF;
        END IF;
    END IF;
END $$;

CREATE TABLE roles (
    code text PRIMARY KEY
);

INSERT INTO roles (code) VALUES
    ('ADMIN'),
    ('BRANCH_MANAGER'),
    ('CUSTOMER'),
    ('EXECUTIVE'),
    ('FINANCE'),
    ('MARKETING'),
    ('RATE_MANAGER'),
    ('TELLER'),
    ('VERIFICATION_REVIEWER');

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id),
    auth_subject text NULL UNIQUE,
    name text NOT NULL,
    status text NOT NULL CHECK (status IN ('INVITED', 'ACTIVE', 'DISABLED')),
    version integer NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by text NOT NULL,
    legacy_employee_id text NULL UNIQUE
);

CREATE TABLE user_invites (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    code_hash text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz NULL
);

CREATE TABLE user_roles (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_code text NOT NULL REFERENCES roles(code),
    organization_id uuid NOT NULL REFERENCES organizations(id),
    branch_id text NULL,
    valid_from timestamptz NOT NULL DEFAULT now(),
    valid_to timestamptz NULL,
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id)
);

CREATE TABLE user_sessions (
    hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    version integer NOT NULL,
    expires timestamptz NOT NULL,
    refresh_token_enc bytea,
    organization_id uuid NOT NULL REFERENCES organizations(id)
);

CREATE TABLE user_audit (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id text NOT NULL,
    subject_id text NOT NULL,
    action text NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    organization_id uuid NOT NULL REFERENCES organizations(id)
);

DO $$
DECLARE
    org_id uuid;
    emp RECORD;
    own RECORD;
    uid uuid;
    mapped_role text;
    user_status text;
BEGIN
    SELECT id INTO org_id FROM organizations WHERE code = 'XC' LIMIT 1;
    IF org_id IS NULL THEN
        org_id := '00000000-0000-0000-0000-000000000001'::uuid;
    END IF;

    -- Backfill owners
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'owners') THEN
        FOR own IN SELECT * FROM owners LOOP
            uid := gen_random_uuid();
            INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, legacy_employee_id)
            VALUES (uid, org_id, NULL, own.name, 'INVITED', 1, own.created_at, 'migration', NULL);

            INSERT INTO user_roles (id, user_id, role_code, organization_id, branch_id, valid_from, valid_to)
            VALUES
                (gen_random_uuid(), uid, 'ADMIN', org_id, NULL, own.created_at, NULL),
                (gen_random_uuid(), uid, 'EXECUTIVE', org_id, NULL, own.created_at, NULL);
        END LOOP;
    END IF;

    -- Backfill employees
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'employees') THEN
        FOR emp IN SELECT * FROM employees LOOP
            mapped_role := CASE emp.role
                WHEN 'FRONT' THEN 'TELLER'
                WHEN 'BACKEND' THEN 'BRANCH_MANAGER'
                WHEN 'MANAGER' THEN 'BRANCH_MANAGER'
                WHEN 'RATES' THEN 'RATE_MANAGER'
                WHEN 'FINANCE' THEN 'FINANCE'
                WHEN 'SYSTEM' THEN 'ADMIN'
                WHEN 'MONITOR' THEN 'VERIFICATION_REVIEWER'
                WHEN 'DOCUMENTS' THEN 'VERIFICATION_REVIEWER'
                WHEN 'DOCUMENT_REVIEW' THEN 'VERIFICATION_REVIEWER'
                ELSE NULL
            END;

            IF emp.active THEN
                user_status := 'INVITED';
            ELSE
                user_status := 'DISABLED';
            END IF;

            uid := gen_random_uuid();
            INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, legacy_employee_id)
            VALUES (uid, org_id, NULL, emp.name, user_status, 1, emp.created_at, 'migration', emp.id);

            INSERT INTO user_roles (id, user_id, role_code, organization_id, branch_id, valid_from, valid_to)
            VALUES (gen_random_uuid(), uid, mapped_role, org_id, emp.branch_id, emp.created_at, NULL);

            INSERT INTO user_invites (user_id, code_hash, expires_at, used_at)
            VALUES (uid, 'expired-migration-invite', emp.created_at, NULL);
        END LOOP;
    END IF;
END $$;
