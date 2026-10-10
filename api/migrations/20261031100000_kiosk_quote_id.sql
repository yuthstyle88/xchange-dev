-- 20261031100000_kiosk_quote_id.sql
-- TX-7 (#675): store kiosk_requests.quote_id pointing to quotes(id)

ALTER TABLE kiosk_requests ADD COLUMN quote_id uuid NULL REFERENCES quotes(id);
