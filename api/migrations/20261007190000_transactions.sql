-- Saved exchanges. One row per exchange; its three documents in transaction_files.
CREATE TABLE transactions (
    id             uuid          PRIMARY KEY DEFAULT gen_random_uuid(),
    seq            bigint        GENERATED ALWAYS AS IDENTITY UNIQUE,
    document_ref   text          NOT NULL UNIQUE,
    branch_id      text          NOT NULL REFERENCES branches (id),
    mode           text          NOT NULL CHECK (mode IN ('LIVE', 'DEMO')),
    shift_id       text          REFERENCES shifts (id),
    submission_id  text          NOT NULL,
    payload_hash   text          NOT NULL,
    customer_id    text          NOT NULL REFERENCES customers (id),
    customer_name  text          NOT NULL,
    nationality    text          NOT NULL,
    type           text          NOT NULL CHECK (type IN ('BUY', 'SELL')),
    code           char(3)       NOT NULL,
    amount         numeric(20,2) NOT NULL CHECK (amount > 0),
    rate           numeric(14,4) NOT NULL CHECK (rate > 0),
    reference_rate numeric(14,4) CHECK (reference_rate > 0),
    principal      numeric(20,2) NOT NULL,
    fee            numeric(20,2) NOT NULL,
    bank_fee       numeric(20,2) NOT NULL,
    received       numeric(20,2) NOT NULL,
    paid           numeric(20,2) NOT NULL,
    method         text          NOT NULL,
    payment_ref    text          NOT NULL,
    account        text          NOT NULL,
    staff_id       text          NOT NULL,
    booking_no     text          NOT NULL,
    notes          text          NOT NULL,
    value_date     date          NOT NULL,
    paid_at        timestamptz   NOT NULL,
    recorded_at    timestamptz   NOT NULL,
    recorded_by    text          NOT NULL,
    UNIQUE (branch_id, mode, submission_id),
    CHECK ((mode = 'LIVE') = (shift_id IS NOT NULL))
);
CREATE INDEX transactions_branch_mode_seq_idx ON transactions (branch_id, mode, seq);

CREATE TABLE transaction_files (
    transaction_id uuid NOT NULL REFERENCES transactions (id),
    kind           text NOT NULL CHECK (kind IN ('PASSPORT', 'SLIP', 'RECEIPT')),
    file_key       text NOT NULL REFERENCES files (key),
    PRIMARY KEY (transaction_id, kind)
);
