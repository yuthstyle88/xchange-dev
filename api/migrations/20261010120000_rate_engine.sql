-- Migration 4: Blueprint rate engine tables, publications, and backfill from rates (RE-1)
-- 1. Reference batches and reference lines
CREATE TABLE rate_reference_batches (
    id uuid PRIMARY KEY,
    provider text NOT NULL,
    source_date date NOT NULL,
    fetched_at timestamptz NOT NULL,
    raw_checksum text NOT NULL,
    status text NOT NULL CHECK (status IN ('COMPLETE','PARTIAL')),
    UNIQUE (provider, source_date, raw_checksum)
);

CREATE TABLE rate_reference_lines (
    id uuid PRIMARY KEY,
    batch_id uuid NOT NULL REFERENCES rate_reference_batches(id) ON DELETE CASCADE,
    currency_code char(3) NOT NULL REFERENCES currencies(code),
    reference_type text NOT NULL,
    unit_size numeric(20,4) NOT NULL,
    buy numeric(20,8) NULL,
    sell numeric(20,8) NULL,
    raw jsonb NOT NULL,
    UNIQUE (batch_id, currency_code, reference_type)
);

-- 2. Rate policy versions
CREATE TABLE rate_policy_versions (
    id uuid PRIMARY KEY,
    branch_id text NOT NULL REFERENCES branches(id),
    version integer NOT NULL,
    buy_adjust_pct numeric(9,4) NOT NULL,
    sell_adjust_pct numeric(9,4) NOT NULL,
    guards jsonb NOT NULL,
    max_reference_age_hours integer NULL,
    on_stale text NULL CHECK (on_stale IS NULL OR on_stale IN ('STOP', 'LIMIT')),
    stale_limit_thb numeric(20,2) NULL,
    valid_from timestamptz NOT NULL,
    valid_to timestamptz NULL,
    approved_by text NOT NULL,
    reason text NOT NULL,
    source_version integer NULL,
    UNIQUE (branch_id, version)
);

-- 3. Rate drafts and draft lines
CREATE TABLE rate_drafts (
    id uuid PRIMARY KEY,
    branch_id text NOT NULL REFERENCES branches(id),
    status text NOT NULL CHECK (status IN ('DRAFT','PENDING_APPROVAL','APPROVED','REJECTED','PUBLISHED')),
    base_publication_version integer NULL,
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    version integer NOT NULL DEFAULT 1,
    evaluation jsonb NULL,
    evaluation_hash text NULL
);

CREATE TABLE rate_draft_lines (
    id uuid PRIMARY KEY,
    draft_id uuid NOT NULL REFERENCES rate_drafts(id) ON DELETE CASCADE,
    branch_id text NOT NULL REFERENCES branches(id),
    currency_code char(3) NOT NULL REFERENCES currencies(code),
    denomination_group text NOT NULL DEFAULT 'ALL',
    reference_line_id uuid NULL REFERENCES rate_reference_lines(id),
    policy_version_id uuid NULL REFERENCES rate_policy_versions(id),
    buy_mode text NOT NULL CHECK (buy_mode IN ('AUTO','MANUAL')),
    sell_mode text NOT NULL CHECK (sell_mode IN ('AUTO','MANUAL')),
    manual_buy numeric(20,8) NULL,
    manual_sell numeric(20,8) NULL,
    reason text NULL,
    UNIQUE (draft_id, currency_code, denomination_group),
    CHECK ((buy_mode = 'MANUAL') = (manual_buy IS NOT NULL)),
    CHECK ((sell_mode = 'MANUAL') = (manual_sell IS NOT NULL)),
    CHECK ((buy_mode = 'AUTO' AND sell_mode = 'AUTO') OR (reason IS NOT NULL AND reason <> ''))
);

-- 4. Rate publications and published rate lines
CREATE TABLE rate_publications (
    id uuid PRIMARY KEY,
    branch_id text NOT NULL REFERENCES branches(id),
    version integer NOT NULL,
    published_at timestamptz NOT NULL DEFAULT now(),
    approved_request_id uuid NOT NULL REFERENCES approval_requests(id),
    draft_id uuid NULL UNIQUE REFERENCES rate_drafts(id),
    published_by text NOT NULL,
    UNIQUE (branch_id, version)
);

CREATE TABLE published_rate_lines (
    id uuid PRIMARY KEY,
    publication_id uuid NOT NULL REFERENCES rate_publications(id) ON DELETE CASCADE,
    currency_code char(3) NOT NULL REFERENCES currencies(code),
    denomination_group text NOT NULL DEFAULT 'ALL',
    unit_size numeric(20,4) NOT NULL,
    buy_rate numeric(20,8) NULL,
    sell_rate numeric(20,8) NULL,
    source_snapshot jsonb NOT NULL,
    UNIQUE (publication_id, currency_code, denomination_group),
    CHECK (buy_rate IS NULL OR sell_rate IS NULL OR buy_rate <= sell_rate)
);

-- 5. Branch current publication pointer
ALTER TABLE branches
    ADD COLUMN current_publication_id uuid NULL REFERENCES rate_publications(id);

-- 6. Backfill publication version 1 from rates for every branch that has rates rows
DO $$
DECLARE
    b_rec RECORD;
    req_id uuid;
    pub_id uuid;
    latest_by text;
BEGIN
    FOR b_rec IN SELECT DISTINCT branch_id FROM rates LOOP
        IF EXISTS (SELECT 1 FROM branches WHERE id = b_rec.branch_id) THEN
            SELECT updated_by INTO latest_by FROM rates WHERE branch_id = b_rec.branch_id ORDER BY id DESC LIMIT 1;
            
            req_id := gen_random_uuid();
            pub_id := gen_random_uuid();
            
            INSERT INTO approval_requests (
                id, type, branch_id, object_id, object_version_hash, policy_version,
                proposed_payload, requested_by, requested_at, status, idempotency_key
            ) VALUES (
                req_id, 'RATE_PUBLICATION', b_rec.branch_id, pub_id::text, 'migrated_from_rates', '1',
                '{"migrated_from": "rates"}'::jsonb, coalesce(latest_by, 'migration'), now(), 'APPROVED',
                'migration_rate_engine_' || b_rec.branch_id
            );
            
            INSERT INTO approval_decisions (
                id, request_id, approver_id, approver_role, outcome, reason, at
            ) VALUES (
                gen_random_uuid(), req_id, 'migration', 'SYSTEM', 'APPROVED', 'migrated_from_rates', now()
            );
            
            INSERT INTO rate_publications (
                id, branch_id, version, published_at, approved_request_id, draft_id, published_by
            ) VALUES (
                pub_id, b_rec.branch_id, 1, now(), req_id, NULL, coalesce(latest_by, 'migration')
            );
            
            INSERT INTO published_rate_lines (
                id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot
            )
            SELECT
                gen_random_uuid(),
                pub_id,
                r.code,
                'ALL',
                c.unit_size,
                r.buy * c.unit_size,
                r.sell * c.unit_size,
                jsonb_build_object(
                    'migrated_from', 'rates',
                    'rates_id', r.id::text,
                    'per_one', jsonb_build_object('buy', r.buy, 'sell', r.sell),
                    'reference', r.reference
                )
            FROM (
                SELECT DISTINCT ON (code) id, code, buy, sell, reference
                FROM rates
                WHERE branch_id = b_rec.branch_id
                ORDER BY code, id DESC
            ) r
            JOIN currencies c ON c.code = r.code;
            
            UPDATE branches SET current_publication_id = pub_id WHERE id = b_rec.branch_id;
        END IF;
    END LOOP;
END $$;

-- down:
-- UPDATE branches SET current_publication_id = NULL WHERE current_publication_id IS NOT NULL;
-- ALTER TABLE branches DROP COLUMN current_publication_id;
-- DROP TABLE published_rate_lines;
-- DROP TABLE rate_publications;
-- DROP TABLE rate_draft_lines;
-- DROP TABLE rate_drafts;
-- DROP TABLE rate_policy_versions;
-- DROP TABLE rate_reference_lines;
-- DROP TABLE rate_reference_batches;
-- DELETE FROM approval_decisions WHERE request_id IN (SELECT id FROM approval_requests WHERE idempotency_key LIKE 'migration_rate_engine_%');
-- DELETE FROM approval_requests WHERE idempotency_key LIKE 'migration_rate_engine_%';
