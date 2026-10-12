CREATE TABLE phone_verifications (
    id                 uuid PRIMARY KEY,
    organization_id    uuid NOT NULL REFERENCES organizations (id),
    customer_id        text NOT NULL REFERENCES customers (id),
    branch_id          text NOT NULL REFERENCES branches (id),
    phone_encrypted    bytea NOT NULL,
    key_id             text NOT NULL,
    phone_lookup_hash  bytea NOT NULL,
    code_hash          bytea NOT NULL,
    expires_at         timestamptz NOT NULL,
    attempts           integer NOT NULL DEFAULT 0,
    status             text NOT NULL CHECK (status IN ('PENDING', 'VERIFIED', 'LOCKED', 'SUPERSEDED')),
    requested_by       uuid NOT NULL REFERENCES users (id),
    identity_id        uuid REFERENCES customer_identities (id),
    created_at         timestamptz NOT NULL DEFAULT now(),
    decided_at         timestamptz
);

CREATE UNIQUE INDEX one_pending_per_customer
    ON phone_verifications (organization_id, customer_id)
    WHERE status = 'PENDING';

CREATE INDEX phone_verifications_rate
    ON phone_verifications (organization_id, phone_lookup_hash, created_at);

-- down:
-- DROP TABLE phone_verifications;
