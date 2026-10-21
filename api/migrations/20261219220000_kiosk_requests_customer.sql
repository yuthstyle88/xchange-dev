-- KWQ (kiosk web queue, slice a): the member who made a web request.
-- Design: docs/design/blueprint/kiosk-shared-phone.md §4.6. Owner-approved: PR 4249 issuecomment-6311349162.
-- NULL for a kiosk request and for every row before this migration; the service, not the schema,
-- requires it for a web request, so old rows stay valid. No FK, as queue_tickets.customer_id. No backfill.
-- Renumber with scripts/migration-slot.sh next right before pr_open.

ALTER TABLE kiosk_requests
    ADD COLUMN customer_id text NULL;
