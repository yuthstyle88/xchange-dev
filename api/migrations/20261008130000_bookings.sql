CREATE TABLE members (
    id          text        PRIMARY KEY,                 -- MB-<uuid>
    branch_id   text        NOT NULL REFERENCES branches (id),
    customer_id text        NOT NULL REFERENCES customers (id),
    name        text        NOT NULL,
    phone       text        NOT NULL,
    token_hash  text        NOT NULL UNIQUE,
    verified_at timestamptz NOT NULL,
    verified_by text        NOT NULL,
    UNIQUE (branch_id, customer_id)
);

CREATE TABLE bookings (
    id              text          PRIMARY KEY,           -- BK-<uuid>
    branch_id       text          NOT NULL REFERENCES branches (id),
    request_id      text          NOT NULL,
    request_hash    text          NOT NULL,
    name            text          NOT NULL,
    phone           text          NOT NULL,
    member_id       text          REFERENCES members (id),
    type            text          NOT NULL CHECK (type IN ('BUY', 'SELL')),
    code            char(3)       NOT NULL REFERENCES currencies (code),
    amount          numeric(20,2) NOT NULL CHECK (amount > 0),
    rate            numeric(14,4) NOT NULL CHECK (rate > 0),
    status          text          NOT NULL CHECK (status IN ('PENDING', 'CONFIRMED', 'USED', 'CANCELLED')),
    created_at      timestamptz   NOT NULL,
    created_by      text          NOT NULL,
    expires_at      timestamptz,
    confirmed_at    timestamptz,
    confirmed_by    text,
    submission_id   text,
    transaction_ref text          REFERENCES transactions (document_ref),
    UNIQUE (branch_id, request_id),
    CHECK (status <> 'PENDING'   OR (member_id IS NOT NULL AND expires_at IS NULL)),
    CHECK (status <> 'CONFIRMED' OR expires_at IS NOT NULL),
    CHECK ((status = 'USED') = (submission_id IS NOT NULL AND transaction_ref IS NOT NULL))
);
CREATE INDEX bookings_branch_created_idx ON bookings (branch_id, created_at DESC, id DESC);
