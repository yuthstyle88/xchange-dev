-- QU-8 (#1267): retire queue work times from kiosk_requests (design §8 M2)
ALTER TABLE kiosk_requests
    DROP COLUMN started_at,
    DROP COLUMN completed_at;
