CREATE TABLE ops_state (
    branch_id         text        PRIMARY KEY REFERENCES branches (id),
    enabled           boolean     NOT NULL DEFAULT false,
    phase             text        NOT NULL CHECK (phase IN ('CLOSED', 'READY', 'OPEN', 'CUT_OFF', 'COUNTING')),
    custodian_id      text,
    entity            jsonb,
    target            jsonb,
    safe              jsonb,
    cutoff            jsonb,
    service_opened_at timestamptz
);

CREATE TABLE attendance (
    id          text    PRIMARY KEY,
    branch_id   text    NOT NULL REFERENCES branches (id),
    position    integer NOT NULL,
    employee_id text    NOT NULL,
    data        jsonb   NOT NULL,
    UNIQUE (branch_id, position)
);

CREATE TABLE custody_requests (
    id          text    PRIMARY KEY,
    branch_id   text    NOT NULL REFERENCES branches (id),
    position    integer NOT NULL,
    request_id  text    NOT NULL,
    kind        text    NOT NULL,
    status      text    NOT NULL CHECK (status IN ('PENDING', 'REJECTED', 'COMPLETED')),
    data        jsonb   NOT NULL,
    UNIQUE (branch_id, position),
    UNIQUE (branch_id, request_id)
);

CREATE UNIQUE INDEX custody_requests_one_pending_per_branch ON custody_requests (branch_id) WHERE status = 'PENDING';

CREATE TABLE custody_signatures (
    request_id  text        NOT NULL REFERENCES custody_requests (id) ON DELETE CASCADE,
    position    integer     NOT NULL,
    employee_id text        NOT NULL,
    role        text        NOT NULL,
    at          timestamptz NOT NULL,
    PRIMARY KEY (request_id, position)
);

CREATE TABLE customer_holds (
    id        text    PRIMARY KEY,
    branch_id text    NOT NULL REFERENCES branches (id),
    position  integer NOT NULL,
    data      jsonb   NOT NULL,
    UNIQUE (branch_id, position)
);

CREATE TABLE tx_drafts (
    id        text    PRIMARY KEY,
    branch_id text    NOT NULL REFERENCES branches (id),
    position  integer NOT NULL,
    data      jsonb   NOT NULL,
    UNIQUE (branch_id, position)
);

CREATE TABLE cash_exceptions (
    id        text    PRIMARY KEY,
    branch_id text    NOT NULL REFERENCES branches (id),
    position  integer NOT NULL,
    data      jsonb   NOT NULL,
    UNIQUE (branch_id, position)
);

CREATE TABLE ops_events (
    id        text        PRIMARY KEY,
    seq       bigint      GENERATED ALWAYS AS IDENTITY UNIQUE,
    branch_id text        NOT NULL REFERENCES branches (id),
    kind      text        NOT NULL,
    at        timestamptz NOT NULL,
    actor     text        NOT NULL,
    role      text        NOT NULL,
    details   jsonb       NOT NULL
);

CREATE INDEX ops_events_branch ON ops_events (branch_id, seq);
