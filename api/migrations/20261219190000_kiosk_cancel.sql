-- KC-M (#4173): kiosk request cancel — stored CANCELLED status, five cancel columns, and five named CHECKs.
-- Design: docs/design/blueprint/kiosk-cancel.md §3.1 (KC-4 migration approved; KC-5 DAY_CLOSED code).

ALTER TABLE kiosk_requests
    DROP CONSTRAINT kiosk_requests_status_check,
    DROP CONSTRAINT kiosk_requests_check1,
    ADD CONSTRAINT kiosk_requests_status_check
        CHECK (status IN ('QUOTED', 'WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED', 'CANCELLED')),
    ADD CONSTRAINT kiosk_requests_staff_check
        CHECK (staff IS NOT NULL OR status IN ('QUOTED', 'WAITING', 'CANCELLED')),
    ADD COLUMN cancelled_at      timestamptz,
    ADD COLUMN cancelled_by      text,
    ADD COLUMN cancel_reason     text,
    ADD COLUMN cancel_note       text,
    ADD COLUMN cancel_token_hash text,
    ADD CONSTRAINT kiosk_requests_cancel_reason_check
        CHECK (cancel_reason IN ('CUSTOMER_LEFT', 'CUSTOMER_CANCELLED', 'CUSTOMER_CHANGED_MIND', 'DUPLICATE', 'OTHER', 'DAY_CLOSED')),
    ADD CONSTRAINT kiosk_requests_cancelled_check
        CHECK ((status = 'CANCELLED') = (cancelled_at IS NOT NULL AND cancelled_by IS NOT NULL AND cancel_reason IS NOT NULL)),
    ADD CONSTRAINT kiosk_requests_cancel_note_check
        CHECK (cancel_reason IS DISTINCT FROM 'OTHER' OR (cancel_note IS NOT NULL AND length(btrim(cancel_note)) BETWEEN 1 AND 200));

-- down:
-- Warning: this down section is lossy. It refuses while any row is CANCELLED, and drops every cancel record
-- (cancelled_at, cancelled_by, cancel_reason, cancel_note) and every cancel token (cancel_token_hash).
--
-- DO $$
-- BEGIN
--     IF EXISTS (SELECT 1 FROM kiosk_requests WHERE status = 'CANCELLED') THEN
--         RAISE EXCEPTION 'cannot roll back kiosk_cancel migration while kiosk_requests contains CANCELLED rows';
--     END IF;
-- END
-- $$;
--
-- ALTER TABLE kiosk_requests
--     DROP CONSTRAINT kiosk_requests_cancel_note_check,
--     DROP CONSTRAINT kiosk_requests_cancelled_check,
--     DROP CONSTRAINT kiosk_requests_cancel_reason_check,
--     DROP COLUMN cancel_token_hash,
--     DROP COLUMN cancel_note,
--     DROP COLUMN cancel_reason,
--     DROP COLUMN cancelled_by,
--     DROP COLUMN cancelled_at,
--     DROP CONSTRAINT kiosk_requests_staff_check,
--     DROP CONSTRAINT kiosk_requests_status_check,
--     ADD CONSTRAINT kiosk_requests_status_check
--         CHECK (status IN ('QUOTED', 'WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED')),
--     ADD CONSTRAINT kiosk_requests_check1
--         CHECK (staff IS NOT NULL OR status IN ('QUOTED', 'WAITING'));
