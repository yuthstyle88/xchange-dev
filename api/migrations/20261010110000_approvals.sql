CREATE TABLE approval_requests (
    id                  uuid        PRIMARY KEY,
    type                text        NOT NULL,
    branch_id           text        REFERENCES branches (id),
    object_id           text        NOT NULL,
    object_version_hash text        NOT NULL,
    policy_version      text        NOT NULL,
    proposed_payload    jsonb       NOT NULL,
    requested_by        text        NOT NULL,
    requested_at        timestamptz NOT NULL,
    status              text        NOT NULL CHECK (status IN ('PENDING', 'APPROVED', 'REJECTED')),
    idempotency_key     text        NOT NULL,
    UNIQUE (type, idempotency_key)
);
CREATE UNIQUE INDEX one_pending_per_object ON approval_requests (type, object_id) WHERE status = 'PENDING';

CREATE TABLE approval_decisions (
    id            uuid        PRIMARY KEY,
    request_id    uuid        NOT NULL REFERENCES approval_requests (id) UNIQUE,
    approver_id   text        NOT NULL,
    approver_role text        NOT NULL,
    outcome       text        NOT NULL CHECK (outcome IN ('APPROVED', 'REJECTED')),
    reason        text        NOT NULL,
    at            timestamptz NOT NULL
);

CREATE TABLE audit_events (
    id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_id        text        NOT NULL,
    actor_role      text        NOT NULL,
    scope           text        NOT NULL,
    branch_id       text,
    action          text        NOT NULL,
    object_ref      text        NOT NULL,
    before_redacted jsonb,
    after_redacted  jsonb,
    at              timestamptz NOT NULL
);
CREATE INDEX audit_events_actor_id_idx ON audit_events (actor_id);
