-- F5 (#6094): widen kiosk_requests_cancel_reason_check to accept STOCK_SHORT.
-- Design: docs/design/blueprint/kiosk-cancel.md §2, §3.1 (owner approval #6094 issuecomment-6627165045).

ALTER TABLE kiosk_requests
    DROP CONSTRAINT kiosk_requests_cancel_reason_check,
    ADD CONSTRAINT kiosk_requests_cancel_reason_check
        CHECK (cancel_reason IN ('CUSTOMER_LEFT', 'STOCK_SHORT', 'CUSTOMER_CANCELLED', 'CUSTOMER_CHANGED_MIND', 'DUPLICATE', 'OTHER', 'DAY_CLOSED'));

-- down:
-- Warning: this down section is lossy. Restoring the old CHECK fails while any row holds STOCK_SHORT,
-- so it first maps those rows to OTHER with note STOCK_SHORT (keeping kiosk_requests_cancel_note_check satisfied).
--
-- UPDATE kiosk_requests
--    SET cancel_reason = 'OTHER',
--        cancel_note = COALESCE(NULLIF(btrim(cancel_note), ''), 'STOCK_SHORT')
--  WHERE cancel_reason = 'STOCK_SHORT';
--
-- ALTER TABLE kiosk_requests
--     DROP CONSTRAINT kiosk_requests_cancel_reason_check,
--     ADD CONSTRAINT kiosk_requests_cancel_reason_check
--         CHECK (cancel_reason IN ('CUSTOMER_LEFT', 'CUSTOMER_CANCELLED', 'CUSTOMER_CHANGED_MIND', 'DUPLICATE', 'OTHER', 'DAY_CLOSED'));
