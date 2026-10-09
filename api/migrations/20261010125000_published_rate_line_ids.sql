-- RE-4a: migration 5 — published_rate_line_id on transactions, kiosk_requests and bookings,
-- and conversion of stored JPY/KRW rates to per-unit_size.
-- Design: docs/design/blueprint/rates.md §8 migration 5.

-- 1. Add published_rate_line_id nullable foreign keys
ALTER TABLE transactions
    ADD COLUMN published_rate_line_id uuid NULL REFERENCES published_rate_lines (id);

ALTER TABLE kiosk_requests
    ADD COLUMN published_rate_line_id uuid NULL REFERENCES published_rate_lines (id);

ALTER TABLE bookings
    ADD COLUMN published_rate_line_id uuid NULL REFERENCES published_rate_lines (id);

-- 2. Convert stored rates and reference rates to per-unit_size
UPDATE transactions t
   SET rate = t.rate * c.unit_size,
       reference_rate = t.reference_rate * c.unit_size,
       unit_size = c.unit_size
  FROM currencies c
 WHERE c.code = t.code
   AND c.unit_size <> 1
   AND t.unit_size = 1;

UPDATE kiosk_requests t
   SET rate = t.rate * c.unit_size,
       unit_size = c.unit_size
  FROM currencies c
 WHERE c.code = t.currency
   AND c.unit_size <> 1
   AND t.unit_size = 1;

UPDATE bookings t
   SET rate = t.rate * c.unit_size,
       unit_size = c.unit_size
  FROM currencies c
 WHERE c.code = t.code
   AND c.unit_size <> 1
   AND t.unit_size = 1;

-- down:
-- ALTER TABLE transactions DROP COLUMN published_rate_line_id;
-- ALTER TABLE kiosk_requests DROP COLUMN published_rate_line_id;
-- ALTER TABLE bookings DROP COLUMN published_rate_line_id;
-- UPDATE transactions t SET rate = t.rate / c.unit_size, reference_rate = t.reference_rate / c.unit_size, unit_size = 1 FROM currencies c WHERE c.code = t.code AND c.unit_size <> 1 AND t.unit_size = c.unit_size;
-- UPDATE kiosk_requests t SET rate = t.rate / c.unit_size, unit_size = 1 FROM currencies c WHERE c.code = t.currency AND c.unit_size <> 1 AND t.unit_size = c.unit_size;
-- UPDATE bookings t SET rate = t.rate / c.unit_size, unit_size = 1 FROM currencies c WHERE c.code = t.code AND c.unit_size <> 1 AND t.unit_size = c.unit_size;
