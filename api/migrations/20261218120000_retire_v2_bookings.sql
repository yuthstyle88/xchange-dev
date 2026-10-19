-- BK-11 #1413: migration M2 — retire v2 rate-lock bookings (design docs/design/blueprint/booking-slots.md §6, §8 M2).
-- 1. Clear transactions.booking_id values that reference v2_bookings (not in Blueprint bookings),
--    then re-point transactions_booking_id_fkey at bookings(id).
UPDATE transactions t
   SET booking_id = NULL
 WHERE t.booking_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM bookings b WHERE b.id = t.booking_id);

ALTER TABLE transactions
    DROP CONSTRAINT transactions_booking_id_fkey,
    ADD CONSTRAINT transactions_booking_id_fkey
        FOREIGN KEY (booking_id) REFERENCES bookings (id);

-- 2. Delete v2 bookingRules rows from branch_setting_history first (FK to branch_settings), then branch_settings.
DELETE FROM branch_setting_history WHERE name = 'bookingRules';
DELETE FROM branch_settings WHERE name = 'bookingRules';

-- 3. Drop v2_bookings and members (no CASCADE so any stray FK fails loudly).
DROP TABLE v2_bookings;
DROP TABLE members;

-- down:
-- DO $$
-- BEGIN
--     IF (SELECT count(*) FROM transactions WHERE booking_id IS NOT NULL) <> 0 THEN
--         RAISE EXCEPTION 'retire_v2_bookings down: transactions.booking_id names a Blueprint booking on some rows';
--     END IF;
-- END
-- $$;
-- CREATE TABLE members (
--     id              text        PRIMARY KEY,
--     branch_id       text        NOT NULL REFERENCES branches (id),
--     customer_id     text        NOT NULL REFERENCES customers (id),
--     name            text        NOT NULL,
--     phone           text        NOT NULL,
--     token_hash      text        NOT NULL UNIQUE,
--     verified_at     timestamptz NOT NULL,
--     verified_by     text        NOT NULL,
--     organization_id uuid        NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001',
--     UNIQUE (branch_id, customer_id),
--     FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
-- );
-- CREATE TABLE v2_bookings (
--     id                     text          PRIMARY KEY,
--     branch_id              text          NOT NULL REFERENCES branches (id),
--     request_id             text          NOT NULL,
--     request_hash           text          NOT NULL,
--     name                   text          NOT NULL,
--     phone                  text          NOT NULL,
--     member_id              text          REFERENCES members (id),
--     type                   text          NOT NULL CHECK (type IN ('BUY', 'SELL')),
--     code                   char(3)       NOT NULL REFERENCES currencies (code),
--     amount                 numeric(20,2) NOT NULL CHECK (amount > 0),
--     rate                   numeric(20,8) NOT NULL CHECK (rate > 0),
--     status                 text          NOT NULL CHECK (status IN ('PENDING', 'CONFIRMED', 'USED', 'CANCELLED')),
--     created_at             timestamptz   NOT NULL,
--     created_by             text          NOT NULL,
--     expires_at             timestamptz,
--     confirmed_at           timestamptz,
--     confirmed_by           text,
--     submission_id          text,
--     transaction_ref        text,
--     unit_size              numeric(20,4) NOT NULL DEFAULT 1,
--     published_rate_line_id uuid          NULL REFERENCES published_rate_lines (id),
--     organization_id        uuid          NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001',
--     UNIQUE (branch_id, request_id),
--     FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
--     CONSTRAINT bookings_organization_id_transaction_ref_fkey
--         FOREIGN KEY (organization_id, transaction_ref)
--         REFERENCES transactions (organization_id, document_ref),
--     CHECK (status <> 'PENDING'   OR (member_id IS NOT NULL AND expires_at IS NULL)),
--     CHECK (status <> 'CONFIRMED' OR expires_at IS NOT NULL),
--     CHECK ((status = 'USED') = (submission_id IS NOT NULL AND transaction_ref IS NOT NULL))
-- );
-- CREATE INDEX bookings_branch_created_idx ON v2_bookings (branch_id, created_at DESC, id DESC);
-- ALTER TABLE transactions
--     DROP CONSTRAINT transactions_booking_id_fkey,
--     ADD CONSTRAINT transactions_booking_id_fkey
--         FOREIGN KEY (booking_id) REFERENCES v2_bookings (id);
