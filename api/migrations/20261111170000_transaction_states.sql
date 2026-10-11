-- Migration 2: transaction states, child tables (lines, payments, receipts), backfill, counter rename.

-- Precheck: any existing transaction must have a RECEIPT file
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM transactions t
        WHERE NOT EXISTS (
            SELECT 1 FROM transaction_files f
            WHERE f.transaction_id = t.id AND f.kind = 'RECEIPT'
        )
    ) THEN
        RAISE EXCEPTION 'missing receipt file';
    END IF;
END $$;

-- 1. Alter transactions columns
ALTER TABLE transactions
    ADD COLUMN status text NOT NULL DEFAULT 'COMPLETED'
        CHECK (status IN ('DRAFT','PENDING_APPROVAL','CONFIRMED','COMPLETED','CANCELLED')),
    ADD COLUMN quote_id uuid NULL REFERENCES quotes(id),
    ADD COLUMN booking_id text NULL REFERENCES bookings(id),
    ADD COLUMN completed_at timestamptz NULL,
    ADD COLUMN cancelled_at timestamptz NULL,
    ADD COLUMN cancel_reason text NULL,
    ADD COLUMN approval_request_id uuid NULL REFERENCES approval_requests(id),
    ADD COLUMN kiosk_request_id text NULL,
    ADD COLUMN staff_draft_id text NULL,
    ADD COLUMN count_confirmed boolean NOT NULL DEFAULT false;

-- 2. Constraints on document_ref and bookings foreign key
ALTER TABLE transactions
    ADD CONSTRAINT transactions_organization_id_document_ref_key
        UNIQUE (organization_id, document_ref);

ALTER TABLE bookings
    DROP CONSTRAINT bookings_transaction_ref_fkey;

ALTER TABLE bookings
    ADD CONSTRAINT bookings_organization_id_transaction_ref_fkey
        FOREIGN KEY (organization_id, transaction_ref)
        REFERENCES transactions (organization_id, document_ref);

ALTER TABLE transactions
    ALTER COLUMN document_ref DROP NOT NULL;

ALTER TABLE transactions
    DROP CONSTRAINT transactions_document_ref_key;

-- 3. Backfill existing transactions
UPDATE transactions
SET status = 'COMPLETED',
    completed_at = recorded_at,
    booking_id = (
        SELECT b.id FROM bookings b
        WHERE b.transaction_ref = transactions.document_ref
          AND b.organization_id = transactions.organization_id
        LIMIT 1
    );

-- 4. Status checks on transactions
ALTER TABLE transactions
    ADD CONSTRAINT transactions_completed_document_ref_check
        CHECK ((status = 'COMPLETED') = (document_ref IS NOT NULL)),
    ADD CONSTRAINT transactions_completed_at_check
        CHECK ((status = 'COMPLETED') = (completed_at IS NOT NULL)),
    ADD CONSTRAINT transactions_cancelled_check
        CHECK ((status = 'CANCELLED') = (cancelled_at IS NOT NULL AND cancel_reason IS NOT NULL)),
    ADD CONSTRAINT transactions_non_cancelled_check
        CHECK (status = 'CANCELLED' OR (cancelled_at IS NULL AND cancel_reason IS NULL));

-- 4. Create child tables
CREATE TABLE transaction_lines (
    id                  uuid PRIMARY KEY,
    organization_id     uuid NOT NULL REFERENCES organizations(id),
    transaction_id      uuid NOT NULL REFERENCES transactions(id),
    quote_line_id       uuid NULL REFERENCES quote_lines(id),
    shop_side           text NOT NULL CHECK (shop_side IN ('BUY','SELL')),
    currency            char(3) NOT NULL REFERENCES currencies(code),
    denomination_id     uuid NULL,
    quantity            numeric(20,4) NOT NULL CHECK (quantity > 0),
    unit_size           numeric(20,4) NOT NULL CHECK (unit_size > 0),
    rate_snapshot       numeric(20,8) NOT NULL CHECK (rate_snapshot > 0),
    reference_rate      numeric(20,8) NULL,
    thb_amount          numeric(20,2) NOT NULL,
    fee_thb             numeric(20,2) NOT NULL DEFAULT 0,
    settle_thb          numeric(20,2) NOT NULL,
    cost_basis_thb      numeric(20,2) NULL,
    realized_margin_thb numeric(20,2) NULL,
    line_no             int NOT NULL,
    created_at          timestamptz NOT NULL,
    updated_at          timestamptz NOT NULL,
    UNIQUE (transaction_id, line_no)
);

CREATE TABLE transaction_payments (
    id               uuid PRIMARY KEY,
    organization_id  uuid NOT NULL REFERENCES organizations(id),
    transaction_id   uuid NOT NULL REFERENCES transactions(id),
    payment_no       int NOT NULL DEFAULT 1,
    method           text NOT NULL,
    reference        text NOT NULL DEFAULT '',
    account          text NOT NULL DEFAULT '',
    slip_file_key    text NULL REFERENCES files(key),
    amount_thb       numeric(20,2) NOT NULL,
    bank_fee_thb     numeric(20,2) NOT NULL DEFAULT 0,
    paid_at          timestamptz NOT NULL,
    created_at       timestamptz NOT NULL,
    updated_at       timestamptz NOT NULL,
    UNIQUE (transaction_id, payment_no)
);

CREATE TABLE receipts (
    id               uuid PRIMARY KEY,
    organization_id  uuid NOT NULL REFERENCES organizations(id),
    transaction_id   uuid NOT NULL REFERENCES transactions(id),
    number           text NOT NULL,
    revision         int NOT NULL CHECK (revision >= 1),
    issued_at        timestamptz NOT NULL,
    document_ref     text NOT NULL REFERENCES files(key),
    created_at       timestamptz NOT NULL,
    updated_at       timestamptz NOT NULL,
    UNIQUE (organization_id, number),
    UNIQUE (transaction_id, revision)
);

-- 5. Rename existing counters to include organization_id
UPDATE counters
SET key = 'document-' || (SELECT id::text FROM organizations LIMIT 1) || '-' || substring(key from 10)
WHERE key ~ '^document-(LIVE|DEMO)-[0-9]{8}$';


-- 7. Backfill transaction_lines
INSERT INTO transaction_lines (
    id,
    organization_id,
    transaction_id,
    quote_line_id,
    shop_side,
    currency,
    denomination_id,
    quantity,
    unit_size,
    rate_snapshot,
    reference_rate,
    thb_amount,
    fee_thb,
    settle_thb,
    cost_basis_thb,
    realized_margin_thb,
    line_no,
    created_at,
    updated_at
)
SELECT
    gen_random_uuid(),
    t.organization_id,
    t.id,
    NULL,
    t.type,
    t.code,
    NULL,
    t.amount,
    t.unit_size,
    t.rate,
    t.reference_rate,
    t.principal,
    t.fee,
    CASE t.type WHEN 'BUY' THEN t.paid ELSE t.received END,
    NULL,
    NULL,
    1,
    t.recorded_at,
    t.recorded_at
FROM transactions t;

-- 8. Backfill transaction_payments
INSERT INTO transaction_payments (
    id,
    organization_id,
    transaction_id,
    payment_no,
    method,
    reference,
    account,
    slip_file_key,
    amount_thb,
    bank_fee_thb,
    paid_at,
    created_at,
    updated_at
)
SELECT
    gen_random_uuid(),
    t.organization_id,
    t.id,
    1,
    t.method,
    t.payment_ref,
    t.account,
    (SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'SLIP' LIMIT 1),
    CASE t.type WHEN 'BUY' THEN t.paid ELSE t.received END,
    t.bank_fee,
    t.paid_at,
    t.recorded_at,
    t.recorded_at
FROM transactions t;

-- 9. Backfill receipts
INSERT INTO receipts (
    id,
    organization_id,
    transaction_id,
    number,
    revision,
    issued_at,
    document_ref,
    created_at,
    updated_at
)
SELECT
    gen_random_uuid(),
    t.organization_id,
    t.id,
    t.document_ref,
    1,
    t.recorded_at,
    (SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'RECEIPT' LIMIT 1),
    t.recorded_at,
    t.recorded_at
FROM transactions t;

-- 10. Index
CREATE INDEX transactions_branch_status_idx ON transactions (branch_id, status, recorded_at);

-- Down Migration:
-- DROP INDEX IF EXISTS transactions_branch_status_idx;
-- DROP TABLE IF EXISTS receipts;
-- DROP TABLE IF EXISTS transaction_payments;
-- DROP TABLE IF EXISTS transaction_lines;
-- ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_non_cancelled_check;
-- ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_cancelled_check;
-- ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_completed_at_check;
-- ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_completed_document_ref_check;
-- ALTER TABLE bookings DROP CONSTRAINT IF EXISTS bookings_organization_id_transaction_ref_fkey;
-- ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_organization_id_document_ref_key;
-- ALTER TABLE transactions ADD CONSTRAINT transactions_document_ref_key UNIQUE (document_ref);
-- ALTER TABLE transactions ALTER COLUMN document_ref SET NOT NULL;
-- ALTER TABLE bookings ADD CONSTRAINT bookings_transaction_ref_fkey FOREIGN KEY (transaction_ref) REFERENCES transactions (document_ref);
-- ALTER TABLE transactions DROP COLUMN count_confirmed;
-- ALTER TABLE transactions DROP COLUMN staff_draft_id;
-- ALTER TABLE transactions DROP COLUMN kiosk_request_id;
-- ALTER TABLE transactions DROP COLUMN approval_request_id;
-- ALTER TABLE transactions DROP COLUMN cancel_reason;
-- ALTER TABLE transactions DROP COLUMN cancelled_at;
-- ALTER TABLE transactions DROP COLUMN completed_at;
-- ALTER TABLE transactions DROP COLUMN booking_id;
-- ALTER TABLE transactions DROP COLUMN quote_id;
-- ALTER TABLE transactions DROP COLUMN status;
-- UPDATE counters SET key = regexp_replace(key, '^document-[0-9a-fA-F-]+-', 'document-') WHERE key ~ '^document-[0-9a-fA-F-]+-(LIVE|DEMO)-[0-9]{8}$';
