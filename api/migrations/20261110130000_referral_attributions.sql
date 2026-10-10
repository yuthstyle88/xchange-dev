-- Migration: referral_attributions
--
-- Data migration:
-- 1. Existing rows: none in the new table, and no existing table changes.
-- 2. Backfill: none. v2's referrals are not carried into v3 (owner, 2026-10-08), and v3 has never recorded a code.
-- 3. Deploy order: with this binary. Nothing calls capture_in until the routes (RA-3) or a caller's ticket ships.
-- 4. What down loses: dropping the table loses every capture and its broker_outcome. Broker keeps its own attributions. v3 is forward-only.

CREATE TABLE referral_attributions (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    branch_id       text NOT NULL REFERENCES branches(id),
    code            text NOT NULL CHECK (code ~ '^[A-Z0-9]{6,12}$'),
    customer_id     text NOT NULL REFERENCES customers(id),
    customer_sub    text NULL,
    booking_id      text NULL REFERENCES bookings(id),
    transaction_id  uuid NULL REFERENCES transactions(id),
    source          text NOT NULL CHECK (source IN ('APP', 'BOOKING', 'DESK')),
    captured_at     timestamptz NOT NULL,
    captured_by     text NOT NULL,
    status          text NOT NULL CHECK (status IN ('ACTIVE', 'SUPERSEDED')),
    supersedes_id   uuid NULL REFERENCES referral_attributions(id),
    reason          text NULL,
    pushed_at       timestamptz NULL,
    broker_outcome  text NULL CHECK (broker_outcome IN
                      ('CREATED', 'EXISTING', 'UNKNOWN_CODE', 'SELF_REFERRAL', 'HELD_BY_OTHER', 'REFUSED', 'EXPIRED')),
    CHECK ((supersedes_id IS NULL) = (reason IS NULL))
);

CREATE UNIQUE INDEX one_active_per_transaction ON referral_attributions (transaction_id)
  WHERE status = 'ACTIVE' AND transaction_id IS NOT NULL;

CREATE UNIQUE INDEX one_active_per_booking ON referral_attributions (booking_id)
  WHERE status = 'ACTIVE' AND booking_id IS NOT NULL;

CREATE INDEX referral_attributions_waiting ON referral_attributions (customer_id)
  WHERE status = 'ACTIVE' AND pushed_at IS NULL;
