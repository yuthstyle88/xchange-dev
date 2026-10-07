-- Row AG-1 (design §5.4c, *Data migration — 20261008120000_device_agents.sql*).
-- The counter drawer agents, the lease on a drawer job, and the status CHECK widened to the
-- statuses the agent writes. It also fixes a latent bug: until now a job could not be stored
-- as DRAWER_OPENED (20261007180000_devices.sql:8). No backfill: every existing job keeps its
-- status and gets no lease; the table of agents starts empty.

CREATE TABLE device_agents (
    id                   uuid        PRIMARY KEY,
    branch_id            text        NOT NULL REFERENCES branches (id),
    name                 text        NOT NULL,
    created_at           timestamptz NOT NULL,
    created_by           text        NOT NULL,
    pair_code_hash       text        UNIQUE,
    pair_code_expires_at timestamptz,
    token_hash           text        UNIQUE,
    paired_at            timestamptz,
    revoked_at           timestamptz,
    revoked_by           text,
    last_seen_at         timestamptz,
    agent_version        text,
    agent_build          text,
    agent_os             text,
    agent_host           text,
    drawer_transport     text        CHECK (drawer_transport IN ('CUPS', 'TCP')),
    drawer_target        text,
    claim_window_start   timestamptz,
    claim_count          integer     NOT NULL DEFAULT 0
);
-- One active agent per branch, paired or not; a revoked one does not count.
CREATE UNIQUE INDEX device_agents_one_active_per_branch_idx
    ON device_agents (branch_id) WHERE revoked_at IS NULL;

ALTER TABLE device_jobs
    ADD COLUMN lease_id       uuid,
    ADD COLUMN leased_by      uuid REFERENCES device_agents (id),
    ADD COLUMN lease_until    timestamptz,
    ADD COLUMN outcome_reason text;

ALTER TABLE device_jobs DROP CONSTRAINT device_jobs_status_check;
ALTER TABLE device_jobs ADD CONSTRAINT device_jobs_status_check CHECK (status IN (
    'QUEUED', 'FAILED', 'SIMULATED_PRINTED', 'COMMAND_SIMULATED',
    'LEASED', 'DRAWER_OPENED', 'EXPIRED'
));

CREATE INDEX device_jobs_drawer_queued_idx ON device_jobs (branch_id, created_at)
    WHERE status = 'QUEUED' AND kind = 'DRAWER_REQUEST';
CREATE INDEX device_jobs_leased_idx ON device_jobs (branch_id, lease_until)
    WHERE status = 'LEASED';
