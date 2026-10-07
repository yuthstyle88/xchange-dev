-- Row Q1: queue work times (started_at, completed_at) on kiosk_requests.
-- Nullable, additive, no backfill (design §5.5.3).

ALTER TABLE kiosk_requests
    ADD COLUMN started_at timestamptz,
    ADD COLUMN completed_at timestamptz;
