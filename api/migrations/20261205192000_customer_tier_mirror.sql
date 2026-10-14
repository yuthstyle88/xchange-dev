CREATE TABLE customer_tier_mirror (
    organization_id   uuid NOT NULL REFERENCES organizations(id),
    phone_lookup_hash bytea NOT NULL,
    tier_code         text NOT NULL,
    tier_rule_id      uuid NOT NULL,
    tier_rule_version int  NOT NULL CHECK (tier_rule_version > 0),
    effective_at      timestamptz NOT NULL,
    effective_until   timestamptz NULL,
    loyalty_seq       bigint NOT NULL,
    synced_at         timestamptz NOT NULL,
    PRIMARY KEY (organization_id, phone_lookup_hash)
);

CREATE TABLE loyalty_sync_cursor (
    organization_id   uuid PRIMARY KEY REFERENCES organizations(id),
    cursor            bigint NOT NULL DEFAULT 0,
    last_ok_at        timestamptz NULL,
    last_error        text NULL
);

-- down:
-- DROP TABLE customer_tier_mirror;
-- DROP TABLE loyalty_sync_cursor;
