CREATE TABLE queue_sequences (
    organization_id uuid NOT NULL, branch_id text NOT NULL, service_date date NOT NULL,
    service_type text NOT NULL CHECK (service_type IN ('WALK_IN','BOOKING')),
    last_sequence integer NOT NULL CHECK (last_sequence > 0),
    created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (branch_id, service_date, service_type),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id));

CREATE TABLE queue_tickets (
    id uuid PRIMARY KEY, organization_id uuid NOT NULL, branch_id text NOT NULL,
    service_date date NOT NULL,
    service_type text NOT NULL CHECK (service_type IN ('WALK_IN','BOOKING')),
    sequence integer NOT NULL CHECK (sequence > 0), display_number text NOT NULL,
    status text NOT NULL CHECK (status IN ('WAITING','CALLED','SERVING','COMPLETED','CANCELLED','NO_SHOW')),
    customer_id text NULL,                              -- no FK: customers is reshaped by CI-4 (#651)
    booking_id text NULL REFERENCES bookings (id),
    kiosk_request_id text NULL,
    counter_teller_id uuid NULL REFERENCES users (id),
    issued_at timestamptz NOT NULL, called_at timestamptz NULL, served_at timestamptz NULL,
    completed_at timestamptz NULL, ended_reason text NULL, idempotency_key text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    UNIQUE (branch_id, service_date, service_type, sequence),
    UNIQUE (branch_id, idempotency_key),
    UNIQUE (branch_id, kiosk_request_id),
    FOREIGN KEY (branch_id, kiosk_request_id) REFERENCES kiosk_requests (branch_id, id),
    CHECK ((service_type = 'BOOKING') = (booking_id IS NOT NULL)));
CREATE UNIQUE INDEX queue_tickets_one_active_per_booking ON queue_tickets (booking_id)
    WHERE status IN ('WAITING','CALLED','SERVING');
CREATE INDEX queue_tickets_waiting ON queue_tickets (branch_id, service_date, service_type, sequence)
    WHERE status = 'WAITING';

CREATE TABLE queue_events (                             -- append-only
    id uuid PRIMARY KEY, organization_id uuid NOT NULL REFERENCES organizations (id),
    ticket_id uuid NOT NULL REFERENCES queue_tickets (id),
    status text NOT NULL CHECK (status IN ('WAITING','CALLED','SERVING','COMPLETED','CANCELLED','NO_SHOW')),
    kind text NOT NULL CHECK (kind IN ('ISSUE','CALL','RECALL','START','FINISH','SKIP','CANCEL')),
    actor_id uuid NULL REFERENCES users (id), at timestamptz NOT NULL, reason text NULL,
    created_at timestamptz NOT NULL DEFAULT now());

CREATE TABLE queue_counters (
    organization_id uuid NOT NULL, branch_id text NOT NULL,
    teller_id uuid NOT NULL REFERENCES users (id),
    status text NOT NULL CHECK (status IN ('ACTIVE','PAUSED')),
    created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (branch_id, teller_id),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id));

CREATE TABLE branch_service_status (
    branch_id text PRIMARY KEY, organization_id uuid NOT NULL,
    queue_open boolean NOT NULL DEFAULT false,
    active_counters integer NOT NULL DEFAULT 0 CHECK (active_counters >= 0),
    queue_version bigint NOT NULL DEFAULT 0,
    estimate_seconds integer NULL, estimate_low_seconds integer NULL, estimate_high_seconds integer NULL,
    estimate_as_of timestamptz NULL,
    created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    CHECK ((estimate_seconds IS NULL) = (estimate_low_seconds IS NULL)
       AND (estimate_seconds IS NULL) = (estimate_high_seconds IS NULL)),
    CHECK (estimate_low_seconds <= estimate_seconds AND estimate_seconds <= estimate_high_seconds));

INSERT INTO branch_service_status (branch_id, organization_id) SELECT id, organization_id FROM branches;
