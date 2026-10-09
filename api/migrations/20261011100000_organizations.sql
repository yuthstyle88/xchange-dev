CREATE TABLE organizations (
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE,
    legal_name text,
    timezone text NOT NULL,
    created_at timestamptz NOT NULL
);

INSERT INTO organizations (id, code, legal_name, timezone, created_at)
VALUES ('00000000-0000-0000-0000-000000000001', 'XC', NULL, 'Asia/Bangkok', now());

ALTER TABLE branches
    ADD COLUMN organization_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001',
    ADD COLUMN code text,
    ADD COLUMN address text,
    ADD COLUMN timezone text NOT NULL DEFAULT 'Asia/Bangkok',
    ADD COLUMN opening_hours jsonb,
    ADD COLUMN booking_enabled boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT branches_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organizations (id),
    ADD CONSTRAINT branches_organization_id_code_key UNIQUE (organization_id, code),
    ADD CONSTRAINT branches_organization_id_id_key UNIQUE (organization_id, id);

UPDATE branches SET code = id WHERE code IS NULL;

DO $$
DECLARE
    tbl text;
    has_branch boolean;
BEGIN
    FOR tbl IN
        SELECT table_name::text
        FROM information_schema.tables
        WHERE table_schema = current_schema()
          AND table_type = 'BASE TABLE'
          AND table_name NOT IN ('_sqlx_migrations', 'organizations', 'branches')
        ORDER BY table_name
    LOOP
        -- Add organization_id column if not present
        IF NOT EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = tbl
              AND column_name = 'organization_id'
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I ADD COLUMN organization_id uuid NOT NULL DEFAULT %L',
                tbl,
                '00000000-0000-0000-0000-000000000001'
            );
        END IF;

        -- Check if table has a branch_id column
        has_branch := EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = tbl
              AND column_name = 'branch_id'
        );

        -- Add composite FK for tables with branch_id except branch_versions and employees;
        -- for the rest, add FK to organizations(id).
        IF has_branch AND tbl NOT IN ('branch_versions', 'employees') THEN
            EXECUTE format(
                'ALTER TABLE %I ADD FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)',
                tbl
            );
        ELSE
            EXECUTE format(
                'ALTER TABLE %I ADD FOREIGN KEY (organization_id) REFERENCES organizations (id)',
                tbl
            );
        END IF;
    END LOOP;
END $$;
