CREATE TABLE device_jobs (
    id           text        PRIMARY KEY,
    request_id   text        NOT NULL,
    request_hash text        NOT NULL,
    branch_id    text        NOT NULL REFERENCES branches (id),
    ref          text        NOT NULL,
    kind         text        NOT NULL CHECK (kind IN ('PRINT_RECEIPT', 'DRAWER_REQUEST')),
    status       text        NOT NULL CHECK (status IN ('QUEUED', 'FAILED', 'SIMULATED_PRINTED', 'COMMAND_SIMULATED')),
    actor        text        NOT NULL,
    created_at   timestamptz NOT NULL,
    updated_at   timestamptz NOT NULL,
    UNIQUE (branch_id, request_id)
);
CREATE INDEX device_jobs_branch_created_idx ON device_jobs (branch_id, created_at DESC, id DESC);

CREATE TABLE device_events (
    seq       bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    dedupe    text,
    branch_id text        NOT NULL REFERENCES branches (id),
    actor     text        NOT NULL,
    kind      text        NOT NULL,
    ref       text        NOT NULL,
    detail    jsonb       NOT NULL,
    at        timestamptz NOT NULL,
    UNIQUE (branch_id, dedupe)
);
CREATE INDEX device_events_branch_seq_idx ON device_events (branch_id, seq DESC);

CREATE TABLE device_settings (
    branch_id text NOT NULL REFERENCES branches (id),
    key       text NOT NULL,
    value     text NOT NULL,
    PRIMARY KEY (branch_id, key)
);
