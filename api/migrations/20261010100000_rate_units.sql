-- RU-1: rates per unit_size at scale 8, foreign amounts by minor unit, rate units migration.
-- Design: docs/design/blueprint/rates.md §3, §8 migration 1 (corrected: converts no stored rate).

-- 1. Register columns on currencies
ALTER TABLE currencies
    ADD COLUMN unit_size numeric(20,4) NOT NULL DEFAULT 1 CHECK (unit_size > 0),
    ADD COLUMN minor_unit smallint NOT NULL DEFAULT 2 CHECK (minor_unit BETWEEN 0 AND 3);

-- Seed unit sizes (A1: JPY 100, KRW 1000, all others 1)
UPDATE currencies SET unit_size = 100 WHERE code = 'JPY';
UPDATE currencies SET unit_size = 1000 WHERE code = 'KRW';

-- Seed minor units from ISO 4217
-- Minor unit 0: JPY, KRW and other zero-decimal currencies in stock register
UPDATE currencies SET minor_unit = 0 WHERE code IN (
    'BIF', 'CLP', 'DJF', 'GNF', 'ISK', 'JPY', 'KMF', 'KRW', 'PYG', 'RWF', 'UGX', 'VND', 'VUV', 'XAF', 'XOF', 'XPF'
);

-- Minor unit 3: 3-decimal currencies
UPDATE currencies SET minor_unit = 3 WHERE code IN (
    'BHD', 'IQD', 'JOD', 'KWD', 'LYD', 'OMR', 'TND'
);

-- 2. Denominations table (A4: group 'ALL' only)
CREATE TABLE denominations (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    currency_code char(3)        NOT NULL REFERENCES currencies (code),
    face_value    numeric(20,4),
    group_code    text           NOT NULL DEFAULT 'ALL'
);

INSERT INTO denominations (currency_code, face_value, group_code)
SELECT code, NULL, 'ALL' FROM currencies;

-- 3. Widen rate columns to numeric(20,8)
ALTER TABLE rates
    ALTER COLUMN buy TYPE numeric(20,8),
    ALTER COLUMN sell TYPE numeric(20,8),
    ALTER COLUMN partner_cost TYPE numeric(20,8);

ALTER TABLE transactions
    ALTER COLUMN rate TYPE numeric(20,8),
    ALTER COLUMN reference_rate TYPE numeric(20,8);

ALTER TABLE kiosk_requests
    ALTER COLUMN rate TYPE numeric(20,8);

ALTER TABLE bookings
    ALTER COLUMN rate TYPE numeric(20,8);

-- 4. Add unit_size DEFAULT 1 to exchange tables (convert NO stored rate row)
ALTER TABLE transactions
    ADD COLUMN unit_size numeric(20,4) NOT NULL DEFAULT 1;

ALTER TABLE kiosk_requests
    ADD COLUMN unit_size numeric(20,4) NOT NULL DEFAULT 1;

ALTER TABLE bookings
    ADD COLUMN unit_size numeric(20,4) NOT NULL DEFAULT 1;

-- down:
-- Warning: this down section is lossy. Narrowing rate columns back to numeric(14,4)
-- truncates rates with more than 4 places. Drops unit_size, minor_unit, and denominations table.
--
-- ALTER TABLE transactions DROP COLUMN unit_size;
-- ALTER TABLE kiosk_requests DROP COLUMN unit_size;
-- ALTER TABLE bookings DROP COLUMN unit_size;
-- DROP TABLE denominations;
-- ALTER TABLE currencies DROP COLUMN unit_size, DROP COLUMN minor_unit;
-- ALTER TABLE rates
--     ALTER COLUMN buy TYPE numeric(14,4),
--     ALTER COLUMN sell TYPE numeric(14,4),
--     ALTER COLUMN partner_cost TYPE numeric(14,4);
-- ALTER TABLE transactions
--     ALTER COLUMN rate TYPE numeric(14,4),
--     ALTER COLUMN reference_rate TYPE numeric(14,4);
-- ALTER TABLE kiosk_requests
--     ALTER COLUMN rate TYPE numeric(14,4);
-- ALTER TABLE bookings
--     ALTER COLUMN rate TYPE numeric(14,4);

