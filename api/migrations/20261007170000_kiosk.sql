-- The kiosk: requests, the currencies each branch offers, and published receipts.
-- Keys are per branch. A request's status is never 'EXPIRED': that is computed when read.

CREATE TABLE kiosk_requests (
    branch_id      text          NOT NULL REFERENCES branches (id),
    id             text          NOT NULL,
    status         text          NOT NULL
                   CHECK (status IN ('QUOTED', 'WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED')),
    created_at     timestamptz   NOT NULL,
    staff          text,
    side           text          NOT NULL CHECK (side IN ('BUY', 'SELL')),
    currency       char(3)       NOT NULL REFERENCES currencies (code),
    rate           numeric(14,4) NOT NULL CHECK (rate > 0),
    foreign_amount numeric(20,2) NOT NULL CHECK (foreign_amount > 0),
    thb            numeric(20,2) NOT NULL CHECK (thb >= 0),
    input_amount   numeric(20,2) NOT NULL CHECK (input_amount > 0),
    payment_method text          NOT NULL
                   CHECK (payment_method IN ('Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer')),
    client_ip      text          NOT NULL DEFAULT '',
    submission_id  text,
    document_ref   text,
    PRIMARY KEY (branch_id, id),
    CHECK ((submission_id IS NULL) = (document_ref IS NULL)),
    CHECK (staff IS NOT NULL OR status IN ('QUOTED', 'WAITING'))
);
CREATE INDEX kiosk_requests_branch_status_created_idx
    ON kiosk_requests (branch_id, status, created_at DESC);
CREATE INDEX kiosk_requests_branch_ip_created_idx
    ON kiosk_requests (branch_id, client_ip, created_at);

CREATE TABLE kiosk_currencies (
    branch_id text    NOT NULL REFERENCES branches (id),
    code      char(3) NOT NULL REFERENCES currencies (code),
    enabled   boolean NOT NULL,
    PRIMARY KEY (branch_id, code)
);

CREATE TABLE kiosk_receipts (
    branch_id  text        NOT NULL,
    request_id text        NOT NULL,
    image      text        NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (branch_id, request_id),
    FOREIGN KEY (branch_id, request_id) REFERENCES kiosk_requests (branch_id, id) ON DELETE CASCADE
);
