-- Dropping this table loses undelivered events.

CREATE TABLE outbox_events (
    id uuid PRIMARY KEY,
    seq bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    event_type text NOT NULL,
    destination text NOT NULL,
    aggregate_type text NOT NULL,
    aggregate_id text NOT NULL,
    idempotency_key text NOT NULL,
    payload_hash text NOT NULL,
    payload_redacted jsonb NOT NULL,
    status text NOT NULL CHECK (status IN ('PENDING','DELIVERED','FAILED','EXPIRED')),
    attempts integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL,
    last_http_status integer NULL,
    last_error text NULL,
    created_at timestamptz NOT NULL,
    delivered_at timestamptz NULL,
    UNIQUE (organization_id, event_type, idempotency_key)
);
CREATE INDEX outbox_due ON outbox_events (next_attempt_at) WHERE status = 'PENDING';
CREATE INDEX outbox_fifo ON outbox_events (aggregate_type, aggregate_id, destination, seq) WHERE status = 'PENDING';
