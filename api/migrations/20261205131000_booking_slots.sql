-- BK-1 #1377: migration M1 of the Blueprint bookings (design docs/design/blueprint/booking-slots.md §4.1, §8 M1).
-- v2's rate-lock `bookings` becomes `v2_bookings` with its rows, constraints and FKs; the Blueprint tables are
-- created empty; one `booking_policies` row per organisation is seeded with the owner's 60 / 15 / 60 (§0 Q-B2..Q-B4);
-- the queue's and the referral capture's booking FKs are re-pointed at the new `bookings` (§8 M1 item 1).

-- 1. Guards: a row naming a booking would name a v2 one, which the re-pointed FKs refuse (§8 M1 item 1).
DO $$
BEGIN
    IF (SELECT count(*) FROM queue_tickets WHERE booking_id IS NOT NULL) <> 0 THEN
        RAISE EXCEPTION 'booking_slots M1: queue_tickets.booking_id is set on some rows; re-pointing its FK would break them';
    END IF;
    IF (SELECT count(*) FROM referral_attributions WHERE booking_id IS NOT NULL) <> 0 THEN
        RAISE EXCEPTION 'booking_slots M1: referral_attributions.booking_id is set on some rows; re-pointing its FK would break them';
    END IF;
END
$$;

-- 2. v2's table keeps its rows under a new name. Index names are schema-wide, so its primary key moves too.
ALTER TABLE bookings RENAME TO v2_bookings;
ALTER INDEX bookings_pkey RENAME TO v2_bookings_pkey;

-- 3. The Blueprint tables (§4.1).
CREATE TABLE booking_policies (
    organization_id        uuid        PRIMARY KEY REFERENCES organizations (id),
    cancel_cutoff_minutes  integer     NOT NULL CHECK (cancel_cutoff_minutes >= 0),
    no_show_grace_minutes  integer     NOT NULL CHECK (no_show_grace_minutes >= 0),
    confirm_expiry_minutes integer     NOT NULL CHECK (confirm_expiry_minutes > 0),
    updated_by             uuid        NULL REFERENCES users (id),
    updated_at             timestamptz NOT NULL DEFAULT now(),
    created_at             timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE booking_slot_rules (
    organization_id uuid        NOT NULL REFERENCES organizations (id),
    branch_id       text        PRIMARY KEY,
    slot_minutes    integer     NOT NULL CHECK (slot_minutes BETWEEN 5 AND 240),
    capacity        integer     NOT NULL CHECK (capacity > 0),
    horizon_days    integer     NOT NULL CHECK (horizon_days BETWEEN 1 AND 90),
    updated_by      uuid        NOT NULL REFERENCES users (id),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE TABLE branch_closures (
    organization_id uuid        NOT NULL REFERENCES organizations (id),
    branch_id       text        NOT NULL,
    closed_on       date        NOT NULL,
    reason          text        NOT NULL,
    created_by      uuid        NOT NULL REFERENCES users (id),
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (branch_id, closed_on),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE TABLE booking_slots (
    id              uuid        PRIMARY KEY,
    organization_id uuid        NOT NULL REFERENCES organizations (id),
    branch_id       text        NOT NULL,
    service_date    date        NOT NULL,                 -- the branch's local date of start_at
    start_at        timestamptz NOT NULL,
    end_at          timestamptz NOT NULL,
    capacity        integer     NOT NULL CHECK (capacity >= 0),
    reserved        integer     NOT NULL DEFAULT 0 CHECK (reserved >= 0 AND reserved <= capacity),
    status          text        NOT NULL CHECK (status IN ('OPEN', 'CLOSED')),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK (end_at > start_at),
    UNIQUE (branch_id, start_at),
    UNIQUE (organization_id, branch_id, id),              -- target of the bookings FK
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE TABLE booking_sequences (
    organization_id uuid        NOT NULL REFERENCES organizations (id),
    issued_on       date        NOT NULL,
    last_sequence   integer     NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, issued_on)
);

CREATE TABLE bookings (
    id                     text          PRIMARY KEY,     -- BK-<uuid>, the form v2 ids use
    organization_id        uuid          NOT NULL REFERENCES organizations (id),
    branch_id              text          NOT NULL,
    booking_no             text          NOT NULL,        -- B<yyMMdd>-<0001>
    customer_id            text          NOT NULL REFERENCES customers (id),
    slot_id                uuid          NOT NULL,
    quote_id               uuid          NULL REFERENCES quotes (id),
    shop_side              text          NOT NULL CHECK (shop_side IN ('BUY', 'SELL')),
    requested_currency     char(3)       NOT NULL REFERENCES currencies (code),
    requested_quantity     numeric(20,4) NOT NULL CHECK (requested_quantity > 0),
    source_channel         text          NOT NULL CHECK (source_channel IN ('APP', 'DESK', 'PARTNER')),
    status                 text          NOT NULL CHECK (status IN ('PENDING_CUSTOMER_CONFIRMATION', 'CONFIRMED',
                                             'CHECKED_IN', 'IN_SERVICE', 'COMPLETED', 'CANCELLED', 'NO_SHOW',
                                             'EXPIRED')),
    qr_token_hash          bytea         NOT NULL UNIQUE,
    qr_version             integer       NOT NULL DEFAULT 1,
    confirm_expires_at     timestamptz   NULL,
    checked_in_at          timestamptz   NULL,
    transaction_id         uuid          NULL REFERENCES transactions (id),
    partner_attribution_id uuid          NULL,            -- FK added by BK-12
    ended_reason           text          NULL,
    created_by             text          NOT NULL,        -- user id, or "system:<route>"
    created_at             timestamptz   NOT NULL DEFAULT now(),
    updated_at             timestamptz   NOT NULL DEFAULT now(),
    UNIQUE (organization_id, booking_no),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    FOREIGN KEY (organization_id, branch_id, slot_id) REFERENCES booking_slots (organization_id, branch_id, id),
    CHECK ((status = 'COMPLETED') = (transaction_id IS NOT NULL)),
    CHECK ((status = 'PENDING_CUSTOMER_CONFIRMATION') <= (confirm_expires_at IS NOT NULL)),
    CHECK ((source_channel = 'PARTNER') <= (partner_attribution_id IS NOT NULL))
);
CREATE UNIQUE INDEX bookings_one_per_transaction ON bookings (transaction_id) WHERE transaction_id IS NOT NULL;
CREATE INDEX bookings_slot_holding ON bookings (slot_id)
    WHERE status IN ('PENDING_CUSTOMER_CONFIRMATION', 'CONFIRMED', 'CHECKED_IN', 'IN_SERVICE', 'COMPLETED');
CREATE INDEX bookings_branch_day ON bookings (branch_id, slot_id, status);

CREATE TABLE booking_events (
    id              uuid        PRIMARY KEY,
    organization_id uuid        NOT NULL REFERENCES organizations (id),
    booking_id      text        NOT NULL REFERENCES bookings (id),
    kind            text        NOT NULL CHECK (kind IN ('CREATE', 'CONFIRM', 'RESCHEDULE', 'CHECK_IN', 'START',
                                    'COMPLETE', 'CANCEL', 'NO_SHOW', 'EXPIRE', 'QR_REISSUE')),
    from_status     text        NULL,
    to_status       text        NOT NULL,
    actor_id        text        NOT NULL,
    at              timestamptz NOT NULL,
    reason          text        NULL,
    detail          jsonb       NOT NULL DEFAULT '{}',    -- RESCHEDULE: {fromSlot, toSlot}
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION prevent_booking_events_modification()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'booking_events is append-only: UPDATE and DELETE are not allowed';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER booking_events_no_update_or_delete
BEFORE UPDATE OR DELETE ON booking_events
FOR EACH STATEMENT EXECUTE FUNCTION prevent_booking_events_modification();

-- 4. The booking FKs that the rename carried to v2_bookings name the new bookings (§8 M1 item 1).
ALTER TABLE queue_tickets
    DROP CONSTRAINT queue_tickets_booking_id_fkey,
    ADD CONSTRAINT queue_tickets_booking_id_fkey FOREIGN KEY (booking_id) REFERENCES bookings (id);
ALTER TABLE referral_attributions
    DROP CONSTRAINT referral_attributions_booking_id_fkey,
    ADD CONSTRAINT referral_attributions_booking_id_fkey FOREIGN KEY (booking_id) REFERENCES bookings (id);

-- 5. Backfill: one policy row per organisation, the owner's 2026-10-10 values (§8 M1 item 2).
INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes,
                              updated_by)
SELECT id, 60, 15, 60, NULL FROM organizations;

-- down:
-- DO $$
-- BEGIN
--     IF (SELECT count(*) FROM queue_tickets WHERE booking_id IS NOT NULL) <> 0 THEN
--         RAISE EXCEPTION 'booking_slots down: queue_tickets.booking_id names a Blueprint booking on some rows';
--     END IF;
--     IF (SELECT count(*) FROM referral_attributions WHERE booking_id IS NOT NULL) <> 0 THEN
--         RAISE EXCEPTION 'booking_slots down: referral_attributions.booking_id names a Blueprint booking on some rows';
--     END IF;
-- END
-- $$;
-- ALTER TABLE queue_tickets
--     DROP CONSTRAINT queue_tickets_booking_id_fkey,
--     ADD CONSTRAINT queue_tickets_booking_id_fkey FOREIGN KEY (booking_id) REFERENCES v2_bookings (id);
-- ALTER TABLE referral_attributions
--     DROP CONSTRAINT referral_attributions_booking_id_fkey,
--     ADD CONSTRAINT referral_attributions_booking_id_fkey FOREIGN KEY (booking_id) REFERENCES v2_bookings (id);
-- DROP TABLE booking_events;
-- DROP FUNCTION prevent_booking_events_modification();
-- DROP TABLE bookings;
-- DROP TABLE booking_sequences;
-- DROP TABLE booking_slots;
-- DROP TABLE branch_closures;
-- DROP TABLE booking_slot_rules;
-- DROP TABLE booking_policies;
-- ALTER INDEX v2_bookings_pkey RENAME TO bookings_pkey;
-- ALTER TABLE v2_bookings RENAME TO bookings;
