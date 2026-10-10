-- CI-4 (#651) M1: one customer per company with encrypted identities
-- Blueprint: docs/design/blueprint/customer-identity-consent.md §4.1, §8

-- 1. Refuse on existing customers (U4: recreate the test database)
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM customers) THEN
        RAISE EXCEPTION 'recreate the test database first (U4)';
    END IF;
END $$;

-- 2. Reshape customers table
DROP INDEX IF EXISTS customers_branch_created_idx;
ALTER TABLE customers DROP CONSTRAINT IF EXISTS customers_branch_id_passport_no_nationality_mode_key;

ALTER TABLE customers
    DROP COLUMN IF EXISTS branch_id,
    DROP COLUMN IF EXISTS name,
    DROP COLUMN IF EXISTS passport_no,
    DROP COLUMN IF EXISTS nationality,
    DROP COLUMN IF EXISTS birth_date,
    DROP COLUMN IF EXISTS passport_expiry;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema() AND table_name = 'customers' AND column_name = 'organization_id'
    ) THEN
        ALTER TABLE customers ADD COLUMN organization_id uuid NOT NULL REFERENCES organizations(id);
    ELSE
        ALTER TABLE customers ALTER COLUMN organization_id DROP DEFAULT;
        IF NOT EXISTS (
            SELECT 1 FROM information_schema.table_constraints tc
            JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name
            WHERE tc.table_schema = current_schema() AND tc.table_name = 'customers'
              AND tc.constraint_type = 'FOREIGN KEY' AND ccu.table_name = 'organizations'
        ) THEN
            ALTER TABLE customers ADD CONSTRAINT customers_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organizations(id);
        END IF;
    END IF;
END $$;

ALTER TABLE customers
    ADD COLUMN customer_no text NOT NULL,
    ADD COLUMN display_name text NOT NULL,
    ADD COLUMN preferred_language text NOT NULL DEFAULT 'th' CHECK (preferred_language IN ('th', 'en', 'zh')),
    ADD COLUMN status text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'CLOSED')),
    ADD COLUMN user_id uuid NULL UNIQUE REFERENCES users(id),
    ADD COLUMN updated_at timestamptz NULL,
    ADD CONSTRAINT customers_org_customer_no_key UNIQUE (organization_id, customer_no);

-- 3. Customer identities
CREATE TABLE customer_identities (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    customer_id     text NOT NULL REFERENCES customers (id),
    type            text NOT NULL CHECK (type IN ('PASSPORT', 'NATIONAL_ID', 'PHONE', 'EMAIL')),
    encrypted_value bytea NOT NULL,
    key_id          text NOT NULL,
    lookup_hash     bytea NOT NULL,
    nationality     char(3) NULL,
    verified_at     timestamptz NULL,
    status          text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'REVOKED')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NULL
);

CREATE UNIQUE INDEX customer_identities_one_active
    ON customer_identities (organization_id, type, lookup_hash)
    WHERE status = 'ACTIVE';

-- 4. Consent texts
CREATE TABLE consent_texts (
    organization_id uuid NOT NULL REFERENCES organizations (id),
    purpose         text NOT NULL,
    version         integer NOT NULL,
    lang            text NOT NULL CHECK (lang IN ('th', 'en', 'zh')),
    body            text NOT NULL,
    published_by    uuid REFERENCES users (id),
    published_at    timestamptz,
    PRIMARY KEY (organization_id, purpose, version, lang)
);

-- 5. Consents (append-only)
CREATE TABLE consents (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    customer_id     text NOT NULL REFERENCES customers (id),
    purpose         text NOT NULL CHECK (purpose IN ('TERMS', 'MARKETING')),
    channel         text NULL CHECK (channel IN ('SMS', 'EMAIL', 'PUSH')),
    policy_version  integer NOT NULL,
    lang            text NOT NULL CHECK (lang IN ('th', 'en', 'zh')),
    granted         boolean NOT NULL,
    effective_at    timestamptz NOT NULL,
    evidence_ref    text NOT NULL,
    recorded_by     uuid NULL REFERENCES users (id),
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION prevent_consents_modification()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'consents is append-only: UPDATE and DELETE are not allowed';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER consents_no_update_or_delete
BEFORE UPDATE OR DELETE ON consents
FOR EACH STATEMENT EXECUTE FUNCTION prevent_consents_modification();

-- 6. Retention policies
CREATE TABLE retention_policies (
    organization_id uuid NOT NULL REFERENCES organizations (id),
    data_type       text NOT NULL,
    period_days     integer NULL CHECK (period_days > 0),
    set_by          uuid REFERENCES users (id),
    set_at          timestamptz,
    PRIMARY KEY (organization_id, data_type)
);
