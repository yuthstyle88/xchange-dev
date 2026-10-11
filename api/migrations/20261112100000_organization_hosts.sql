CREATE TABLE organization_hosts (
    host text PRIMARY KEY CHECK (host = lower(host) AND host <> ''),
    organization_id uuid NOT NULL REFERENCES organizations (id),
    created_at timestamptz NOT NULL DEFAULT now()
);

-- down:
-- DROP TABLE organization_hosts;
