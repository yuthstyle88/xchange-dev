-- Broker exchange feed (design docs/design/blueprint/broker-seam.md §6.1).
-- Forward-only migration; dropping the table moves Broker's cursor past rows that are gone.

CREATE TABLE broker_exchange_feed (
    transaction_id  uuid PRIMARY KEY REFERENCES transactions (id),
    organization_id uuid NOT NULL REFERENCES organizations (id),
    branch_id       text NOT NULL REFERENCES branches (id),
    customer_sub    text NULL,
    phone           text NULL,
    status          text NOT NULL CHECK (status IN ('COMPLETED','REVERSED')),
    value_satang    bigint NOT NULL CHECK (value_satang > 0),
    reversed_satang bigint NOT NULL DEFAULT 0 CHECK (reversed_satang BETWEEN 0 AND value_satang),
    completed_at    timestamptz NOT NULL,
    reversed_at     timestamptz NULL,
    updated_at      timestamptz NOT NULL,
    CHECK ((status = 'REVERSED') = (reversed_at IS NOT NULL)),
    CHECK (status = 'REVERSED' OR reversed_satang = 0)
);

CREATE INDEX broker_exchange_feed_cursor ON broker_exchange_feed (updated_at, transaction_id);
