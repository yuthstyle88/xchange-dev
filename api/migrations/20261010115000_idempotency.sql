CREATE TABLE idempotency_records (
    org_id       text        NOT NULL,
    operation    text        NOT NULL,
    key          text        NOT NULL,
    request_hash text        NOT NULL,
    response_ref jsonb,
    status       text        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    claim_token  text,
    PRIMARY KEY (org_id, operation, key)
);
