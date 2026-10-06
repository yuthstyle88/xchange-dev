CREATE TABLE counters (
    key   text   PRIMARY KEY,
    value bigint NOT NULL CHECK (value >= 0)
);

CREATE TABLE customers (
    id                text        PRIMARY KEY,
    branch_id         text        NOT NULL REFERENCES branches (id),
    name              text        NOT NULL,
    passport_no       text        NOT NULL,
    nationality       text        NOT NULL,
    birth_date        date,
    passport_expiry   date,
    mode              text        NOT NULL CHECK (mode IN ('LIVE', 'DEMO')),
    created_at        timestamptz NOT NULL,
    last_document_ref text,
    UNIQUE (branch_id, passport_no, nationality, mode)
);
CREATE INDEX customers_branch_created_idx ON customers (branch_id, created_at, id);
