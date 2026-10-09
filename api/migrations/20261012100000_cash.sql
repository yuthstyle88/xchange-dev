-- CA-1 (#617): cash schema migration M1 and teller_id on shifts.
-- Design: docs/design/blueprint/cash-shift-transfer.md §4.1, §8 M1

-- 1. Shifts: add teller_id referencing users(id)
ALTER TABLE shifts
    ADD COLUMN teller_id uuid REFERENCES users(id);

-- Opener resolution and backfill for existing shifts
DO $$
DECLARE
    sh_row RECORD;
    resolved_teller uuid;
    owner_count integer;
    has_owner_shifts boolean;
BEGIN
    -- Check if any shift is opened by owner
    SELECT EXISTS (
        SELECT 1 FROM shifts sh
        WHERE sh.opened_by = 'owner'
           OR (EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'owners')
               AND EXISTS (SELECT 1 FROM owners o WHERE o.username = sh.opened_by))
    ) INTO has_owner_shifts;

    IF has_owner_shifts THEN
        -- Verify that there is exactly 1 organisation-wide EXECUTIVE + ADMIN owner user
        SELECT count(*) INTO owner_count
        FROM users u
        WHERE u.legacy_employee_id IS NULL
          AND EXISTS (
              SELECT 1 FROM user_roles r
              WHERE r.user_id = u.id
                AND r.role_code = 'EXECUTIVE'
                AND r.branch_id IS NULL
          )
          AND EXISTS (
              SELECT 1 FROM user_roles r
              WHERE r.user_id = u.id
                AND r.role_code = 'ADMIN'
                AND r.branch_id IS NULL
          );

        IF owner_count <> 1 THEN
            RAISE EXCEPTION 'expected exactly 1 owner user, found %', owner_count;
        END IF;
    END IF;

    -- Loop through all shifts to backfill teller_id
    FOR sh_row IN SELECT * FROM shifts ORDER BY seq LOOP
        resolved_teller := NULL;

        -- 1. users.id
        SELECT id INTO resolved_teller
        FROM users
        WHERE organization_id = sh_row.organization_id
          AND id::text = sh_row.opened_by
        LIMIT 1;

        -- 2. employees.id -> users.legacy_employee_id
        IF resolved_teller IS NULL THEN
            SELECT id INTO resolved_teller
            FROM users
            WHERE organization_id = sh_row.organization_id
              AND legacy_employee_id = sh_row.opened_by
            LIMIT 1;
        END IF;

        -- 3. employees.username -> users
        IF resolved_teller IS NULL THEN
            IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'employees') THEN
                SELECT u.id INTO resolved_teller
                FROM employees e
                JOIN users u ON u.organization_id = sh_row.organization_id AND u.legacy_employee_id = e.id
                WHERE e.username = sh_row.opened_by
                LIMIT 1;
            END IF;
        END IF;

        -- 4. 'owner' or owners.username -> owner user
        IF resolved_teller IS NULL THEN
            IF sh_row.opened_by = 'owner' OR (
                EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'owners')
                AND EXISTS (SELECT 1 FROM owners o WHERE o.username = sh_row.opened_by)
            ) THEN
                SELECT u.id INTO resolved_teller
                FROM users u
                WHERE u.organization_id = sh_row.organization_id
                  AND u.legacy_employee_id IS NULL
                  AND EXISTS (
                      SELECT 1 FROM user_roles r
                      WHERE r.user_id = u.id
                        AND r.organization_id = u.organization_id
                        AND r.role_code = 'EXECUTIVE'
                        AND r.branch_id IS NULL
                  )
                  AND EXISTS (
                      SELECT 1 FROM user_roles r
                      WHERE r.user_id = u.id
                        AND r.organization_id = u.organization_id
                        AND r.role_code = 'ADMIN'
                        AND r.branch_id IS NULL
                  )
                LIMIT 1;
            END IF;
        END IF;

        IF resolved_teller IS NULL THEN
            RAISE EXCEPTION 'shift % opener % maps to nobody', sh_row.id, sh_row.opened_by;
        END IF;

        UPDATE shifts SET teller_id = resolved_teller WHERE id = sh_row.id;
    END LOOP;
END $$;

ALTER TABLE shifts
    ALTER COLUMN teller_id SET NOT NULL;

-- 2. New tables (§4.1)

-- Table: cash_counts
CREATE TABLE cash_counts (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid        NOT NULL REFERENCES organizations(id),
    branch_id       text        NOT NULL,
    shift_id        text        REFERENCES shifts(id),
    kind            text        NOT NULL CHECK (kind IN ('OPEN', 'HANDOVER_OUT', 'CLOSE', 'SAFE_OPEN', 'SAFE')),
    counted_by      uuid        NOT NULL REFERENCES users(id),
    counted_at      timestamptz NOT NULL,
    verified_by     uuid        REFERENCES users(id),
    verified_at     timestamptz,
    note            text,
    idempotency_key text        NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id),
    UNIQUE (branch_id, idempotency_key),
    CHECK (verified_by IS NULL OR verified_by <> counted_by)
);

-- Table: cash_count_lines
CREATE TABLE cash_count_lines (
    count_id        uuid        NOT NULL REFERENCES cash_counts(id) ON DELETE CASCADE,
    organization_id uuid        NOT NULL REFERENCES organizations(id),
    currency        char(3)     NOT NULL REFERENCES currencies(code),
    denomination_id bigint      NOT NULL REFERENCES denominations(id),
    qty             integer     NOT NULL CHECK (qty >= 0),
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (count_id, currency, denomination_id)
);

-- Table: cash_count_totals
CREATE TABLE cash_count_totals (
    count_id        uuid          NOT NULL REFERENCES cash_counts(id) ON DELETE CASCADE,
    organization_id uuid          NOT NULL REFERENCES organizations(id),
    currency        char(3)       NOT NULL REFERENCES currencies(code),
    counted         numeric(20,4) NOT NULL,
    expected        numeric(20,4),
    difference      numeric(20,4),
    created_at      timestamptz   NOT NULL DEFAULT now(),
    PRIMARY KEY (count_id, currency)
);

-- Table: handovers
CREATE TABLE handovers (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid        NOT NULL REFERENCES organizations(id),
    branch_id       text        NOT NULL,
    from_shift_id   text        NOT NULL REFERENCES shifts(id),
    to_shift_id     text        REFERENCES shifts(id),
    count_id        uuid        NOT NULL UNIQUE REFERENCES cash_counts(id),
    receiver_id     uuid        NOT NULL REFERENCES users(id),
    status          text        NOT NULL CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id)
);

-- Table: cash_transfers
CREATE TABLE cash_transfers (
    id                  uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id     uuid        NOT NULL REFERENCES organizations(id),
    kind                text        NOT NULL CHECK (kind IN ('BRANCH', 'DRAWER_TO_SAFE', 'SAFE_TO_DRAWER', 'EXTERNAL_IN', 'EXTERNAL_OUT')),
    from_branch_id      text        NOT NULL,
    to_branch_id        text,
    from_shift_id       text        REFERENCES shifts(id),
    to_shift_id         text        REFERENCES shifts(id),
    status              text        NOT NULL CHECK (status IN ('DRAFT', 'APPROVED', 'DISPATCHED', 'RECEIVED', 'RECONCILED')),
    created_by          uuid        NOT NULL REFERENCES users(id),
    approval_request_id uuid        REFERENCES approval_requests(id),
    dispatched_at       timestamptz,
    dispatched_by       uuid        REFERENCES users(id),
    received_at         timestamptz,
    received_by         uuid        REFERENCES users(id),
    reconciled_at       timestamptz,
    reconciled_by       uuid        REFERENCES users(id),
    reconcile_note      text,
    note                text,
    idempotency_key     text        NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, from_branch_id) REFERENCES branches(organization_id, id),
    FOREIGN KEY (organization_id, to_branch_id) REFERENCES branches(organization_id, id),
    UNIQUE (from_branch_id, idempotency_key)
);

-- Table: cash_transfer_lines
CREATE TABLE cash_transfer_lines (
    transfer_id     uuid        NOT NULL REFERENCES cash_transfers(id) ON DELETE CASCADE,
    organization_id uuid        NOT NULL REFERENCES organizations(id),
    currency        char(3)     NOT NULL REFERENCES currencies(code),
    denomination_id bigint      NOT NULL REFERENCES denominations(id),
    qty_sent        integer     NOT NULL CHECK (qty_sent >= 0),
    qty_received    integer     CHECK (qty_received >= 0),
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (transfer_id, currency, denomination_id)
);

-- Table: cash_transfer_policy_versions
CREATE TABLE cash_transfer_policy_versions (
    version         text        PRIMARY KEY,
    organization_id uuid        NOT NULL REFERENCES organizations(id),
    limits          jsonb       NOT NULL DEFAULT '{}'::jsonb,
    set_by          uuid        NOT NULL REFERENCES users(id),
    created_at      timestamptz NOT NULL DEFAULT now()
);

-- 3. Seed face values into denominations (A7)
INSERT INTO denominations (currency_code, face_value, group_code) VALUES
    ('THB', 0.25, 'ALL'),
    ('THB', 0.5, 'ALL'),
    ('THB', 1, 'ALL'),
    ('THB', 2, 'ALL'),
    ('THB', 5, 'ALL'),
    ('THB', 10, 'ALL'),
    ('THB', 20, 'ALL'),
    ('THB', 50, 'ALL'),
    ('THB', 100, 'ALL'),
    ('THB', 500, 'ALL'),
    ('THB', 1000, 'ALL'),
    ('USD', 0.01, 'ALL'),
    ('USD', 0.05, 'ALL'),
    ('USD', 0.1, 'ALL'),
    ('USD', 0.25, 'ALL'),
    ('USD', 1, 'ALL'),
    ('USD', 2, 'ALL'),
    ('USD', 5, 'ALL'),
    ('USD', 10, 'ALL'),
    ('USD', 20, 'ALL'),
    ('USD', 50, 'ALL'),
    ('USD', 100, 'ALL'),
    ('EUR', 0.01, 'ALL'),
    ('EUR', 0.02, 'ALL'),
    ('EUR', 0.05, 'ALL'),
    ('EUR', 0.1, 'ALL'),
    ('EUR', 0.2, 'ALL'),
    ('EUR', 0.5, 'ALL'),
    ('EUR', 1, 'ALL'),
    ('EUR', 2, 'ALL'),
    ('EUR', 5, 'ALL'),
    ('EUR', 10, 'ALL'),
    ('EUR', 20, 'ALL'),
    ('EUR', 50, 'ALL'),
    ('EUR', 100, 'ALL'),
    ('EUR', 200, 'ALL'),
    ('EUR', 500, 'ALL'),
    ('GBP', 0.01, 'ALL'),
    ('GBP', 0.02, 'ALL'),
    ('GBP', 0.05, 'ALL'),
    ('GBP', 0.1, 'ALL'),
    ('GBP', 0.2, 'ALL'),
    ('GBP', 0.5, 'ALL'),
    ('GBP', 1, 'ALL'),
    ('GBP', 2, 'ALL'),
    ('GBP', 5, 'ALL'),
    ('GBP', 10, 'ALL'),
    ('GBP', 20, 'ALL'),
    ('GBP', 50, 'ALL'),
    ('JPY', 1, 'ALL'),
    ('JPY', 5, 'ALL'),
    ('JPY', 10, 'ALL'),
    ('JPY', 50, 'ALL'),
    ('JPY', 100, 'ALL'),
    ('JPY', 500, 'ALL'),
    ('JPY', 1000, 'ALL'),
    ('JPY', 2000, 'ALL'),
    ('JPY', 5000, 'ALL'),
    ('JPY', 10000, 'ALL'),
    ('CNY', 0.1, 'ALL'),
    ('CNY', 0.5, 'ALL'),
    ('CNY', 1, 'ALL'),
    ('CNY', 5, 'ALL'),
    ('CNY', 10, 'ALL'),
    ('CNY', 20, 'ALL'),
    ('CNY', 50, 'ALL'),
    ('CNY', 100, 'ALL'),
    ('KRW', 10, 'ALL'),
    ('KRW', 50, 'ALL'),
    ('KRW', 100, 'ALL'),
    ('KRW', 500, 'ALL'),
    ('KRW', 1000, 'ALL'),
    ('KRW', 5000, 'ALL'),
    ('KRW', 10000, 'ALL'),
    ('KRW', 50000, 'ALL');

-- down:
-- ALTER TABLE shifts DROP COLUMN teller_id;
-- DELETE FROM denominations WHERE face_value IS NOT NULL;
-- DROP TABLE cash_transfer_policy_versions;
-- DROP TABLE cash_transfer_lines;
-- DROP TABLE cash_transfers;
-- DROP TABLE handovers;
-- DROP TABLE cash_count_totals;
-- DROP TABLE cash_count_lines;
-- DROP TABLE cash_counts;
